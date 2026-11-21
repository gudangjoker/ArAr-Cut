// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors
//
// Integrates FilmCraft's pure-Rust `filmcraft-audio-dsp` engine (MIT OR Apache-2.0)
// for broadcast EBU R128 loudness metering/normalization, GCC-PHAT sample-accurate
// audio sync, and pro audio mastering/restoration chains.

//! Broadcast-grade audio DSP powered by `filmcraft-audio-dsp`:
//! - ITU-R BS.1770-4 / EBU R128 loudness measurement & true-peak-safe normalization
//! - Curated pro mastering/restoration presets (Compressor, Multiband, EQ, DeNoise, DeHum, DeReverb, DeEsser, Limiter)
//! - GCC-PHAT sample-accurate audio sync offset measurement between two clips.

use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use concat_media::{AudioDecoder, AudioOptions, SampleFormat};
use concat_project::Command;
use filmcraft_audio_dsp::effects::{Category, create_effect, effects};
use filmcraft_audio_dsp::loudness::{measure, normalize_gain_db_peak_limited};
use filmcraft_audio_dsp::sync::{SyncOptions, find_offset};
use serde_json::{Value, json};

use crate::session::{MobileSession, TimelineView};
use crate::{MobileError, micros};

/// Timeline extra key tracking processed audio copies so clips can be reverted cleanly.
pub const PRO_AUDIO_KEY: &str = "ararcutProAudio";

const PROCESS_RATE: u32 = 48_000;
const SYNC_RATE: u32 = 16_000;
const BLOCK_FRAMES: usize = 4096;

/// One broadcast audio effect exposed by the FilmCraft DSP engine.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ProAudioEffectInfo {
    /// Stable effect id (e.g. `"multiband_compressor"`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Effect category name.
    pub category: String,
}

/// Curated pro audio processing preset combining DSP effects and loudness targeting.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ProAudioPreset {
    /// Stable preset id (`"ararcut.broadcast_voice"`, etc.).
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Short description of what the chain does.
    pub summary: String,
    /// Recommended loudness target in LUFS.
    pub default_lufs: f64,
}

/// ITU-R BS.1770-4 / EBU R128 loudness measurement of a media clip.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LoudnessReport {
    /// Integrated programme loudness (LUFS).
    pub integrated_lufs: f64,
    /// Loudness range (LU).
    pub loudness_range_lu: f64,
    /// Maximum momentary loudness (LUFS).
    pub max_momentary_lufs: f64,
    /// Maximum short-term loudness (LUFS).
    pub max_short_term_lufs: f64,
    /// Sample peak (dBFS).
    pub sample_peak_dbfs: f64,
    /// 4x oversampled true peak (dBTP).
    pub true_peak_dbtp: f64,
}

/// Sample-accurate GCC-PHAT alignment offset between a reference clip and a target clip.
#[derive(Debug, Clone, uniffi::Record)]
pub struct AudioSyncReport {
    /// Offset in microseconds: positive means target should start later relative to reference.
    pub offset_us: i64,
    /// Correlation peak over RMS (reliable when >= 8.0).
    pub confidence: f32,
    /// Peak height over second-highest peak > 50ms away (reliable when >= 1.5).
    pub distinct: f32,
    /// Whether the match meets FilmCraft's reliability threshold.
    pub reliable: bool,
    /// Whether one recording has inverted polarity.
    pub inverted: bool,
}

fn category_name(cat: Category) -> &'static str {
    match cat {
        Category::Filter => "Filter",
        Category::Dynamics => "Dynamics",
        Category::Time => "Time",
        Category::Channel => "Channel",
        Category::Restoration => "Restoration",
        Category::Pitch => "Pitch",
        Category::Modulation => "Modulation",
        Category::Reverb => "Reverb",
        Category::Special => "Special",
    }
}

/// Lists every audio DSP effect registered in `filmcraft-audio-dsp`.
#[uniffi::export]
pub fn pro_audio_effects() -> Vec<ProAudioEffectInfo> {
    effects()
        .iter()
        .map(|info| ProAudioEffectInfo {
            id: info.id.to_owned(),
            name: info.name.to_owned(),
            category: category_name(info.category).to_owned(),
        })
        .collect()
}

/// Curated pro audio mastering & restoration presets for mobile creators.
#[uniffi::export]
pub fn pro_audio_presets() -> Vec<ProAudioPreset> {
    vec![
        ProAudioPreset {
            id: "ararcut.social_punch".to_owned(),
            name: "Social Punch (-14 LUFS)".to_owned(),
            summary: "Multiband compression + mastering limiter tuned for Reels/Shorts/TikTok.".to_owned(),
            default_lufs: -14.0,
        },
        ProAudioPreset {
            id: "ararcut.broadcast_voice".to_owned(),
            name: "Broadcast Voice (-16 LUFS)".to_owned(),
            summary: "Speech enhancement, DeEsser, vocal compressor, and -1 dBTP limiter.".to_owned(),
            default_lufs: -16.0,
        },
        ProAudioPreset {
            id: "ararcut.clean_restore".to_owned(),
            name: "Clean & Restore".to_owned(),
            summary: "DeHum + DeNoise + Click Remover + DeReverb for noisy room recordings.".to_owned(),
            default_lufs: -16.0,
        },
        ProAudioPreset {
            id: "ararcut.podcast_master".to_owned(),
            name: "Podcast Master (-16 LUFS)".to_owned(),
            summary: "Vocal enhancer, tube compressor, and loudness leveling.".to_owned(),
            default_lufs: -16.0,
        },
        ProAudioPreset {
            id: "ararcut.ebu_r128".to_owned(),
            name: "EBU R128 Broadcast (-23 LUFS)".to_owned(),
            summary: "Pure broadcast loudness normalization with -1.0 dBTP true-peak ceiling.".to_owned(),
            default_lufs: -23.0,
        },
        ProAudioPreset {
            id: "ararcut.stereo_wide".to_owned(),
            name: "Wide Cinema Mix".to_owned(),
            summary: "Stereo expander + subtle mastering polish and loudness balance.".to_owned(),
            default_lufs: -14.0,
        },
    ]
}

/// Decodes a media file into stereo planar `f32` buffers at `rate`.
fn decode_stereo_planar(path: &str, rate: u32) -> Result<(Vec<f32>, Vec<f32>), MobileError> {
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            start: None,
            duration: None,
            filters: Vec::new(),
            rate,
            channels: 2,
            format: SampleFormat::F32,
            stream: None,
        },
    )
    .map_err(|error| MobileError::Failed(error.to_string()))?;
    let interleaved = decoder
        .collect_f32()
        .map_err(|error| MobileError::Failed(error.to_string()))?;
    if interleaved.len() < 2 {
        return Err(MobileError::Failed("The clip has no audible sound.".to_owned()));
    }
    let frames = interleaved.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for pair in interleaved.chunks_exact(2) {
        left.push(pair[0]);
        right.push(pair[1]);
    }
    Ok((left, right))
}

/// Decodes mono `f32` audio at `rate` for fast correlation / analysis.
fn decode_mono(path: &str, rate: u32) -> Result<Vec<f32>, MobileError> {
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            start: None,
            duration: None,
            filters: Vec::new(),
            rate,
            channels: 1,
            format: SampleFormat::F32,
            stream: None,
        },
    )
    .map_err(|error| MobileError::Failed(error.to_string()))?;
    let samples = decoder
        .collect_f32()
        .map_err(|error| MobileError::Failed(error.to_string()))?;
    if samples.is_empty() {
        return Err(MobileError::Failed("The clip has no audible sound.".to_owned()));
    }
    Ok(samples)
}

fn sanitize_db(v: f64) -> f64 {
    if v.is_finite() { v } else { -120.0 }
}

/// Measures EBU R128 / ITU-R BS.1770-4 loudness and true peak for `path`.
#[uniffi::export]
pub fn analyze_clip_loudness(path: String) -> Result<LoudnessReport, MobileError> {
    let (left, right) = decode_stereo_planar(&path, PROCESS_RATE)?;
    let summary = measure(&[left.as_slice(), right.as_slice()], f64::from(PROCESS_RATE));
    Ok(LoudnessReport {
        integrated_lufs: sanitize_db(summary.integrated_lufs),
        loudness_range_lu: if summary.loudness_range_lu.is_finite() {
            summary.loudness_range_lu
        } else {
            0.0
        },
        max_momentary_lufs: sanitize_db(summary.max_momentary_lufs),
        max_short_term_lufs: sanitize_db(summary.max_short_term_lufs),
        sample_peak_dbfs: sanitize_db(summary.sample_peak_dbfs),
        true_peak_dbtp: sanitize_db(summary.true_peak_dbtp),
    })
}

/// Measures the sample-accurate audio sync offset between `reference_path` and `target_path`
/// using FilmCraft's GCC-PHAT correlator.
#[uniffi::export]
pub fn measure_audio_sync(
    reference_path: String,
    target_path: String,
) -> Result<AudioSyncReport, MobileError> {
    let a = decode_mono(&reference_path, SYNC_RATE)?;
    let b = decode_mono(&target_path, SYNC_RATE)?;
    let result = find_offset(&a, &b, SYNC_RATE, &SyncOptions::default())
        .ok_or_else(|| MobileError::Failed("Could not find matching audio between the two clips.".to_owned()))?;
    let offset_sec = result.lag_fine / f64::from(SYNC_RATE);
    Ok(AudioSyncReport {
        offset_us: micros(offset_sec),
        confidence: result.confidence,
        distinct: result.distinct,
        reliable: result.reliable(),
        inverted: result.inverted,
    })
}

fn effect_chain_for_preset(preset_id: &str) -> &'static [&'static str] {
    match preset_id {
        "ararcut.social_punch" => &["multiband_compressor", "mastering", "limiter"],
        "ararcut.broadcast_voice" => &["speech_enhance", "deesser", "compressor", "limiter"],
        "ararcut.clean_restore" => &["dehum", "denoise", "click_remover", "dereverb", "limiter"],
        "ararcut.podcast_master" => &["vocal_enhancer", "tube_compressor", "limiter"],
        "ararcut.ebu_r128" => &["limiter"],
        "ararcut.stereo_wide" => &["stereo_expander", "mastering", "limiter"],
        _ => &["compressor", "limiter"],
    }
}

/// Processes stereo planar buffers in-place through a preset's FilmCraft DSP chain and
/// normalizes integrated loudness toward `target_lufs` with a `-1.0 dBTP` true-peak ceiling.
fn process_planar_chain(
    left: &mut [f32],
    right: &mut [f32],
    sample_rate: u32,
    preset_id: &str,
    target_lufs: f64,
) {
    let sr = sample_rate as f32;
    let mut chain: Vec<_> = effect_chain_for_preset(preset_id)
        .iter()
        .filter_map(|id| create_effect(id, sr, 2))
        .collect();

    let total = left.len().min(right.len());
    let mut offset = 0usize;
    while offset < total {
        let end = (offset + BLOCK_FRAMES).min(total);
        for fx in &mut chain {
            let mut slices: [&mut [f32]; 2] = [&mut left[offset..end], &mut right[offset..end]];
            fx.process(&mut slices);
        }
        offset = end;
    }

    // Broadcast true-peak-safe EBU R128 loudness normalization (-1.0 dBTP ceiling).
    let clamped_target = target_lufs.clamp(-30.0, -6.0);
    let summary = measure(&[&left[..total], &right[..total]], f64::from(sample_rate));
    let gain_db = normalize_gain_db_peak_limited(
        summary.integrated_lufs,
        clamped_target,
        summary.true_peak_dbtp,
        -1.0,
    );
    if gain_db.is_finite() && gain_db.abs() > 0.05 {
        let gain = filmcraft_audio_dsp::db_to_gain(gain_db.clamp(-24.0, 24.0) as f32);
        for s in &mut left[..total] {
            *s = (*s * gain).clamp(-1.0, 1.0);
        }
        for s in &mut right[..total] {
            *s = (*s * gain).clamp(-1.0, 1.0);
        }
    }
}

/// Writes stereo 16-bit PCM WAV from planar `left` and `right` channels.
fn write_stereo_wav(path: &Path, left: &[f32], right: &[f32], rate: u32) -> Result<(), String> {
    let frames = left.len().min(right.len());
    let data = (frames * 4) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for i in 0..frames {
        let l = (left[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let r = (right[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    std::fs::File::create(path)
        .and_then(|mut file| file.write_all(&out))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn target_for(project: &Path, media: &str, preset_id: &str, target_lufs: f64, video: bool) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    media.hash(&mut hasher);
    preset_id.hash(&mut hasher);
    ((target_lufs * 10.0).round() as i64).hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(media) {
        meta.len().hash(&mut hasher);
    }
    let ext = if video { "mp4" } else { "m4a" };
    project
        .join("cache")
        .join(format!("proaudio-{:016x}.{ext}", hasher.finish()))
}

#[uniffi::export]
impl MobileSession {
    /// Returns whether `clip_id` currently points at a Pro Audio DSP processed copy.
    pub fn has_pro_audio(&self, clip_id: String) -> bool {
        let Ok(current) = self.with_session(|session| {
            let project = session.project();
            let clip = project
                .active()
                .clips
                .iter()
                .find(|c| c.id == clip_id)
                .ok_or_else(|| MobileError::Failed("Clip not found.".to_owned()))?;
            let media = project
                .media
                .iter()
                .find(|m| m.id == clip.media_id)
                .ok_or_else(|| MobileError::Failed("Media not found.".to_owned()))?;
            Ok(media.path.clone())
        }) else {
            return false;
        };
        match serde_json::from_str::<Value>(&self.extra(PRO_AUDIO_KEY.to_owned())) {
            Ok(Value::Object(map)) => map.contains_key(&current),
            _ => false,
        }
    }

    /// Processes `clip_id`'s audio through a FilmCraft DSP mastering/restoration preset +
    /// EBU R128 loudness normalization, keeping the original video stream bit-exact (no re-encode).
    pub fn apply_pro_audio(
        self: Arc<Self>,
        clip_id: String,
        preset_id: String,
        target_lufs: f64,
    ) -> Result<TimelineView, MobileError> {
        let (current, video, project_path) = self.with_session(|session| {
            let project = session.project();
            let clip = project
                .active()
                .clips
                .iter()
                .find(|c| c.id == clip_id)
                .ok_or_else(|| MobileError::Failed("That clip is gone.".to_owned()))?;
            let media = project
                .media
                .iter()
                .find(|m| m.id == clip.media_id)
                .ok_or_else(|| MobileError::Failed("The clip's file is gone.".to_owned()))?;
            let video = !matches!(media.kind, concat_project::model::MediaKind::Audio);
            Ok((media.path.clone(), video, session.path().to_owned()))
        })?;

        let mut made = match serde_json::from_str::<Value>(&self.extra(PRO_AUDIO_KEY.to_owned())) {
            Ok(Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        // Always process from the clean original file so switching presets never stacks distortion.
        let source = made
            .get(&current)
            .and_then(|entry| entry.get("original"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| current.clone());

        let target = target_for(Path::new(&project_path), &source, &preset_id, target_lufs, video);
        if !target.is_file() {
            let (mut left, mut right) = decode_stereo_planar(&source, PROCESS_RATE)?;
            process_planar_chain(&mut left, &mut right, PROCESS_RATE, &preset_id, target_lufs);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| MobileError::Failed(format!("{}: {error}", parent.display())))?;
            }
            let wav = target.with_extension("proaudio.wav");
            let aac = target.with_extension("proaudio.m4a");
            write_stereo_wav(&wav, &left, &right, PROCESS_RATE).map_err(MobileError::Failed)?;
            let seconds = left.len() as f64 / f64::from(PROCESS_RATE);
            let audio_clip = concat_media::audio::AudioClip {
                path: wav.clone(),
                stream: None,
                start: 0.0,
                duration: seconds,
                source_start: 0.0,
                speed: 1.0,
                preserve_pitch: false,
                volume: 1.0,
                volume_curve: Default::default(),
                fade_in: 0.0,
                fade_out: 0.0,
                filter_chain: String::new(),
            };
            let encoded = concat_media::audio::mix_to_file(&[audio_clip], seconds, &aac)
                .map_err(|error| MobileError::Failed(error.to_string()));
            let _ = std::fs::remove_file(&wav);
            encoded?;
            if video {
                let muxed = concat_media::audio::mux(Path::new(&source), &aac, &target)
                    .map_err(|error| MobileError::Failed(error.to_string()));
                let _ = std::fs::remove_file(&aac);
                muxed?;
            } else {
                std::fs::rename(&aac, &target)
                    .map_err(|error| MobileError::Failed(format!("{}: {error}", target.display())))?;
            }
        }

        let copy = target.to_string_lossy().into_owned();
        let info = concat_host::media::probe(&copy)?;
        made.insert(copy, json!({ "original": source, "preset": preset_id, "lufs": target_lufs }));
        self.run(vec![
            Command::ReplaceClipMedia {
                clip_id,
                item: info.to_new_media(),
                source_start: None,
                enhanced: false,
                offset: 0.0,
            },
            Command::SetTimelineExtra {
                key: PRO_AUDIO_KEY.to_owned(),
                value: Some(Value::Object(made)),
            },
        ])
    }

    /// Reverts a Pro Audio processed clip back to its untouched original media file.
    pub fn revert_pro_audio(self: Arc<Self>, clip_id: String) -> Result<TimelineView, MobileError> {
        let current = self.with_session(|session| {
            let project = session.project();
            let clip = project
                .active()
                .clips
                .iter()
                .find(|c| c.id == clip_id)
                .ok_or_else(|| MobileError::Failed("That clip is gone.".to_owned()))?;
            let media = project
                .media
                .iter()
                .find(|m| m.id == clip.media_id)
                .ok_or_else(|| MobileError::Failed("The clip's file is gone.".to_owned()))?;
            Ok(media.path.clone())
        })?;
        let made = match serde_json::from_str::<Value>(&self.extra(PRO_AUDIO_KEY.to_owned())) {
            Ok(Value::Object(map)) => map,
            _ => return Err(MobileError::Failed("This clip has no Pro Audio processing applied.".to_owned())),
        };
        let original = made
            .get(&current)
            .and_then(|entry| entry.get("original"))
            .and_then(Value::as_str)
            .ok_or_else(|| MobileError::Failed("Original clip audio not found.".to_owned()))?;
        let info = concat_host::media::probe(original)?;
        self.run(vec![Command::ReplaceClipMedia {
            clip_id,
            item: info.to_new_media(),
            source_start: None,
            enhanced: false,
            offset: 0.0,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_dsp_chain_process_cleanly() {
        assert!(!pro_audio_effects().is_empty());
        assert!(pro_audio_presets().len() >= 5);
        let n = 48_000;
        let mut left: Vec<f32> = (0..n).map(|i| (i as f32 * 0.05).sin() * 0.1).collect();
        let mut right = left.clone();
        process_planar_chain(&mut left, &mut right, PROCESS_RATE, "ararcut.broadcast_voice", -16.0);
        assert!(left.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(right.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }
}
