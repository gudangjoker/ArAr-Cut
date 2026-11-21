// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors
//
// Classical MFCC + agglomerative speaker diarization adapted from FilmCraft's
// `crates/speech/src/mel.rs` and `crates/speech/src/diarize.rs` (MIT OR Apache-2.0,
// Copyright FilmCraft contributors / storytold).

//! Speaker diarization ("who spoke when") without downloading any neural model:
//! 16 kHz mono audio -> 40-band Slaney log-mel -> DCT-II MFCC 1..19 -> voiced-frame
//! statistics normalized over the file -> average-linkage agglomerative clustering ->
//! captions labelled `"Speaker 1: ..."`, `"Speaker 2: ..."`, etc. in one undo step.

use std::sync::Arc;

use concat_media::{AudioDecoder, AudioOptions, SampleFormat};
use concat_project::model::ClipKind;
use concat_project::{ClipPatch, Command, srt};

use crate::session::{MobileSession, TimelineView};
use crate::MobileError;

const SAMPLE_RATE: u32 = 16_000;
const N_FFT: usize = 400;
const HOP: usize = 160;
const N_BINS: usize = N_FFT / 2 + 1;
const N_MELS: usize = 40;
const N_MFCC: usize = 20;
const MAX_CLUSTER_CHUNKS: usize = 256;
/// Cap decoded diarization audio at 30 minutes (16 kHz mono f32 ≈ 115 MB) to protect phone RAM.
const MAX_DIARIZE_SECONDS: f64 = 1800.0;

/// Result of auto-labelling caption speakers on the active timeline.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DiarizeReport {
    /// Distinct speakers detected (1..=max_speakers).
    pub speakers_found: u32,
    /// Number of caption clips updated with speaker prefixes.
    pub captions_labelled: u32,
    /// Updated timeline snapshot.
    pub view: TimelineView,
}

fn hz_to_mel(f: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if f >= min_log_hz {
        min_log_mel + (f / min_log_hz).ln() / logstep
    } else {
        f / f_sp
    }
}

fn mel_to_hz(m: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if m >= min_log_mel {
        min_log_hz * (logstep * (m - min_log_mel)).exp()
    } else {
        f_sp * m
    }
}

fn mel_filters(n_mels: usize) -> Vec<f32> {
    let sr = f64::from(SAMPLE_RATE);
    let fftfreqs: Vec<f64> = (0..N_BINS).map(|i| i as f64 * sr / N_FFT as f64).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64))
        .collect();
    let mut w = vec![0f32; n_mels * N_BINS];
    for m in 0..n_mels {
        let enorm = 2.0 / (mel_f[m + 2] - mel_f[m]);
        for (k, &f) in fftfreqs.iter().enumerate() {
            let lower = (f - mel_f[m]) / (mel_f[m + 1] - mel_f[m]);
            let upper = (mel_f[m + 2] - f) / (mel_f[m + 2] - mel_f[m + 1]);
            w[m * N_BINS + k] = (lower.min(upper).max(0.0) * enorm) as f32;
        }
    }
    w
}

fn twiddles() -> (Vec<f32>, Vec<f32>) {
    let mut c = vec![0f32; N_BINS * N_FFT];
    let mut s = vec![0f32; N_BINS * N_FFT];
    for k in 0..N_BINS {
        for i in 0..N_FFT {
            let a = 2.0 * std::f64::consts::PI * ((k * i) % N_FFT) as f64 / N_FFT as f64;
            c[k * N_FFT + i] = a.cos() as f32;
            s[k * N_FFT + i] = -a.sin() as f32;
        }
    }
    (c, s)
}

fn power_spectrum(audio: &[f32]) -> (usize, Vec<f32>) {
    let pad = N_FFT / 2;
    let n = audio.len();
    let at = |i: isize| -> f32 {
        if n == 0 {
            return 0.0;
        }
        if n == 1 {
            return audio[0];
        }
        let p = 2 * (n as isize - 1);
        let mut j = i.rem_euclid(p);
        if j >= n as isize {
            j = p - j;
        }
        audio[j as usize]
    };
    let frames = n / HOP;
    let window: Vec<f32> = (0..N_FFT)
        .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N_FFT as f64).cos()) as f32)
        .collect();
    let (cos, sin) = twiddles();
    let mut out = vec![0f32; frames * N_BINS];
    for f in 0..frames {
        let mut x = [0f32; N_FFT];
        let base = (f * HOP) as isize - pad as isize;
        for (i, v) in x.iter_mut().enumerate() {
            *v = at(base + i as isize) * window[i];
        }
        let row = &mut out[f * N_BINS..(f + 1) * N_BINS];
        for (k, r) in row.iter_mut().enumerate() {
            let (mut re, mut im) = (0f32, 0f32);
            let (c, s) = (&cos[k * N_FFT..(k + 1) * N_FFT], &sin[k * N_FFT..(k + 1) * N_FFT]);
            for i in 0..N_FFT {
                re += x[i] * c[i];
                im += x[i] * s[i];
            }
            *r = re * re + im * im;
        }
    }
    (frames, out)
}

fn mfcc_frames(audio: &[f32]) -> (usize, Vec<f32>, Vec<f32>) {
    let (frames, pow) = power_spectrum(audio);
    let filters = mel_filters(N_MELS);
    let mut energy = vec![0f32; frames];
    let mut mfcc = vec![0f32; frames * N_MFCC];
    let dct: Vec<f32> = (0..N_MFCC * N_MELS)
        .map(|i| {
            let (k, m) = (i / N_MELS, i % N_MELS);
            (std::f32::consts::PI * k as f32 * (m as f32 + 0.5) / N_MELS as f32).cos()
        })
        .collect();
    let mut lm = [0f32; N_MELS];
    for f in 0..frames {
        let p = &pow[f * N_BINS..(f + 1) * N_BINS];
        let mut e = 0f32;
        for m in 0..N_MELS {
            let w = &filters[m * N_BINS..(m + 1) * N_BINS];
            let v: f32 = w.iter().zip(p).map(|(a, b)| a * b).sum();
            e += v;
            lm[m] = (v + 1e-10).ln();
        }
        energy[f] = 10.0 * (e + 1e-10).log10();
        for k in 0..N_MFCC {
            mfcc[f * N_MFCC + k] = (0..N_MELS).map(|m| dct[k * N_MELS + m] * lm[m]).sum();
        }
    }
    (frames, energy, mfcc)
}

fn rms_distance(a: &[f32], b: &[f32]) -> f32 {
    (a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / a.len().max(1) as f32).sqrt()
}

/// Clusters `(start_sec, end_sec)` speech segments over 16 kHz mono `audio` into speaker indices
/// numbered `0..speakers` by order of first appearance. Returns `(speaker_count, labels_per_segment)`.
pub(crate) fn cluster_segments(
    audio: &[f32],
    segments: &[(f64, f64)],
    max_speakers: usize,
    threshold: f32,
) -> (usize, Vec<usize>) {
    if segments.is_empty() {
        return (0, Vec::new());
    }
    if segments.len() < 2 || segments.len() > MAX_CLUSTER_CHUNKS {
        return (1, vec![0; segments.len()]);
    }
    let (frames, energy, mfcc) = mfcc_frames(audio);
    if frames == 0 {
        return (1, vec![0; segments.len()]);
    }
    let frames_per_sec = f64::from(SAMPLE_RATE) / HOP as f64;
    let dim = 2 * (N_MFCC - 1);
    let mut vecs = Vec::with_capacity(segments.len());
    let mut weight = Vec::with_capacity(segments.len());

    for &(start, end) in segments {
        let a = ((start.max(0.0) * frames_per_sec).floor() as usize).min(frames);
        let b = ((end.max(start) * frames_per_sec).ceil() as usize).min(frames).max(a);
        let peak = energy[a..b].iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let voiced: Vec<usize> = (a..b).filter(|&f| energy[f] > peak - 30.0).collect();
        let mut v = vec![0f32; dim];
        if !voiced.is_empty() {
            let n = voiced.len() as f32;
            for k in 1..N_MFCC {
                let mean = voiced.iter().map(|&f| mfcc[f * N_MFCC + k]).sum::<f32>() / n;
                let var = voiced
                    .iter()
                    .map(|&f| (mfcc[f * N_MFCC + k] - mean).powi(2))
                    .sum::<f32>()
                    / n;
                v[k - 1] = mean;
                v[N_MFCC - 1 + k - 1] = var.sqrt();
            }
        }
        vecs.push(v);
        weight.push(voiced.len() as f32);
    }

    let peak_all = energy.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let all: Vec<usize> = (0..frames).filter(|&f| energy[f] > peak_all - 40.0).collect();
    let nf = all.len().max(1) as f32;
    for k in 1..N_MFCC {
        let mean = all.iter().map(|&f| mfcc[f * N_MFCC + k]).sum::<f32>() / nf;
        let sd = (all
            .iter()
            .map(|&f| (mfcc[f * N_MFCC + k] - mean).powi(2))
            .sum::<f32>()
            / nf)
            .sqrt()
            .max(1e-6);
        for v in &mut vecs {
            v[k - 1] = (v[k - 1] - mean) / sd;
            v[N_MFCC - 1 + k - 1] /= sd;
        }
    }

    let n = vecs.len();
    let mut dist = vec![0f32; n * n];
    for i in 0..n {
        for j in i + 1..n {
            let d = rms_distance(&vecs[i], &vecs[j]);
            dist[i * n + j] = d;
            dist[j * n + i] = d;
        }
    }
    let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let link = |a: &[usize], b: &[usize]| -> f32 {
        let mut s = 0f32;
        for &i in a {
            for &j in b {
                s += dist[i * n + j];
            }
        }
        s / (a.len() * b.len()) as f32
    };
    let max_sp = max_speakers.clamp(1, 12);
    loop {
        if clusters.len() <= 1 {
            break;
        }
        let mut best = (f32::INFINITY, 0usize, 0usize);
        for i in 0..clusters.len() {
            for j in i + 1..clusters.len() {
                let d = link(&clusters[i], &clusters[j]);
                if d < best.0 {
                    best = (d, i, j);
                }
            }
        }
        if best.0 > threshold && clusters.len() <= max_sp {
            break;
        }
        let b = clusters.remove(best.2);
        clusters[best.1].extend(b);
    }

    let total_weight: f32 = weight.iter().sum::<f32>().max(1.0);
    loop {
        if clusters.len() <= 1 {
            break;
        }
        let small = clusters
            .iter()
            .position(|c| c.iter().map(|&i| weight[i]).sum::<f32>() < 0.05 * total_weight);
        let Some(si) = small else { break };
        let s = clusters.remove(si);
        let near = (0..clusters.len())
            .min_by(|&a, &b| link(&s, &clusters[a]).total_cmp(&link(&s, &clusters[b])))
            .unwrap_or(0);
        clusters[near].extend(s);
    }

    let mut raw_label = vec![0usize; n];
    for (ci, c) in clusters.iter().enumerate() {
        for &i in c {
            raw_label[i] = ci;
        }
    }
    let mut order = Vec::new();
    for &l in &raw_label {
        if !order.contains(&l) {
            order.push(l);
        }
    }
    let labels = raw_label
        .into_iter()
        .map(|l| order.iter().position(|&o| o == l).unwrap_or(0))
        .collect();
    (order.len(), labels)
}

/// Strips an existing `"Speaker N: "` prefix so diarization can be re-run cleanly.
pub(crate) fn strip_speaker_prefix(text: &str) -> &str {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix("Speaker ") {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            let after = &rest[digits..];
            if let Some(body) = after.strip_prefix(':') {
                return body.trim_start();
            }
        }
    }
    text
}

/// Renders the timeline's audible clips into one 16 kHz mono buffer for speaker clustering.
fn timeline_mono_16k(session: &concat_host::session::Session, max_end: f64) -> Result<Vec<f32>, MobileError> {
    let span = max_end.clamp(0.1, MAX_DIARIZE_SECONDS);
    let total_samples = (span * f64::from(SAMPLE_RATE)).ceil() as usize;
    let mut mix = vec![0f32; total_samples];
    let mut any = false;

    for clip in session.flattened_clips() {
        if clip.muted {
            continue;
        }
        let audible = clip.kind == concat_export::ClipKind::Audio
            || (clip.kind == concat_export::ClipKind::Video && clip.has_audio.unwrap_or(true));
        if !audible || clip.start >= span {
            continue;
        }
        let dur = (clip.duration * clip.speed.max(0.01)).min(span - clip.start);
        let Ok(mut decoder) = AudioDecoder::open(
            &clip.path,
            &AudioOptions {
                start: Some(clip.source_start.max(0.0)),
                duration: Some(dur.max(0.05)),
                filters: Vec::new(),
                rate: SAMPLE_RATE,
                channels: 1,
                format: SampleFormat::F32,
                stream: clip.audio_stream.map(|i| i as usize),
            },
        ) else {
            continue;
        };
        let Ok(samples) = decoder.collect_f32() else {
            continue;
        };
        let dst_start = (clip.start.max(0.0) * f64::from(SAMPLE_RATE)).round() as usize;
        let gain = clip.volume.clamp(0.0, 4.0) as f32;
        for (i, &s) in samples.iter().enumerate() {
            let idx = dst_start + i;
            if idx >= mix.len() {
                break;
            }
            mix[idx] += s * gain;
        }
        any = true;
    }

    if !any {
        return Err(MobileError::Failed(
            "There is no audible clip on the timeline to detect speakers from.".to_owned(),
        ));
    }
    Ok(mix)
}

#[uniffi::export]
impl MobileSession {
    /// Clusters the timeline's caption clips by speaker voice timbre (classical MFCC clustering,
    /// no AI model download needed) and prefixes each caption with `"Speaker N: "` in one undo step.
    pub fn label_caption_speakers(
        self: Arc<Self>,
        max_speakers: u32,
    ) -> Result<DiarizeReport, MobileError> {
        let (targets, audio) = self.with_session(|session| {
            let timeline = session.project().active();
            let ids = crate::speech::caption_ids(timeline);
            let lane = if ids.is_empty() {
                srt::busiest_title_lane(timeline)
            } else {
                None
            };
            let mut targets: Vec<(String, f64, f64, concat_project::model::TextStyle)> = timeline
                .clips
                .iter()
                .filter(|c| {
                    c.kind == ClipKind::Text
                        && if !ids.is_empty() {
                            ids.contains(&c.id)
                        } else {
                            lane.as_deref() == Some(c.track_id.as_str())
                        }
                })
                .filter_map(|c| {
                    let style = c.text.clone()?;
                    Some((c.id.clone(), c.start, c.start + c.duration, style))
                })
                .collect();
            targets.sort_by(|a, b| a.1.total_cmp(&b.1));
            if targets.is_empty() {
                return Err(MobileError::Failed(
                    "Add or generate captions before labelling speakers.".to_owned(),
                ));
            }
            let max_end = targets
                .iter()
                .map(|t| t.2)
                .fold(0.0f64, f64::max);
            let audio = timeline_mono_16k(session, max_end)?;
            Ok((targets, audio))
        })?;

        let intervals: Vec<(f64, f64)> = targets.iter().map(|t| (t.1, t.2)).collect();
        let (speakers_found, labels) =
            cluster_segments(&audio, &intervals, max_speakers.clamp(1, 8) as usize, 0.4);

        let mut commands = Vec::with_capacity(targets.len());
        for ((clip_id, _, _, mut style), speaker_idx) in targets.into_iter().zip(labels) {
            let body = strip_speaker_prefix(&style.content).to_owned();
            style.content = format!("Speaker {}: {body}", speaker_idx + 1);
            commands.push(Command::UpdateClip {
                clip_id,
                patch: ClipPatch {
                    text: Some(Some(style)),
                    ..ClipPatch::default()
                },
            });
        }
        let labelled = commands.len() as u32;
        let view = self.run(commands)?;
        Ok(DiarizeReport {
            speakers_found: speakers_found as u32,
            captions_labelled: labelled,
            view,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_voice(f0: f32, f1: f32, secs: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                0.35 * (2.0 * std::f32::consts::PI * f0 * t).sin()
                    + 0.25 * (2.0 * std::f32::consts::PI * f1 * t).sin()
            })
            .collect()
    }

    #[test]
    fn separates_two_distinct_voices() {
        let mut audio = Vec::new();
        audio.extend(synth_voice(115.0, 520.0, 1.5));
        audio.extend(synth_voice(260.0, 1450.0, 1.5));
        audio.extend(synth_voice(115.0, 520.0, 1.5));
        let segs = [(0.05, 1.45), (1.55, 2.95), (3.05, 4.45)];
        let (count, labels) = cluster_segments(&audio, &segs, 4, 0.4);
        assert_eq!(count, 2);
        assert_eq!(labels[0], labels[2]);
        assert_ne!(labels[0], labels[1]);
    }

    #[test]
    fn prefix_strip_is_idempotent() {
        assert_eq!(strip_speaker_prefix("Speaker 2: Hello world"), "Hello world");
        assert_eq!(strip_speaker_prefix("Hello world"), "Hello world");
    }
}
