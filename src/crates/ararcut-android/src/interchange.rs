// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors
//
// Desktop NLE interchange exporter (CMX 3600 EDL, Final Cut Pro 7 xmeml v5 for
// Premiere Pro / DaVinci Resolve, FCPXML v1.11, and OpenTimelineIO .otio JSON),
// inspired by FilmCraft's `crates/interchange` (MIT OR Apache-2.0).

//! Exports the active ArAr Cut timeline to desktop NLE project formats so mobile rough cuts
//! can be continued in Adobe Premiere Pro, DaVinci Resolve, Final Cut Pro, or OpenTimelineIO.

use std::collections::HashMap;
use std::fmt::Write as _;

use concat_project::model::{Clip, ClipKind, MediaItem, Timeline};
use serde_json::{Value, json};

use crate::MobileError;
use crate::session::MobileSession;

/// Supported desktop NLE timeline interchange formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NleFormat {
    /// CMX 3600 Edit Decision List (`.edl`).
    Edl,
    /// Final Cut Pro 7 XML (`xmeml` v5 `.xml`, native for Adobe Premiere Pro & DaVinci Resolve).
    PremiereXml,
    /// Final Cut Pro XML v1.11 (`.fcpxml`, native for Final Cut Pro & DaVinci Resolve).
    Fcpxml,
    /// OpenTimelineIO JSON (`.otio`).
    Otio,
}

/// Generated NLE project document ready to save through Android Storage Access Framework.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NleExportResult {
    /// The format generated.
    pub format: NleFormat,
    /// Suggested filename with extension.
    pub filename: String,
    /// MIME type for Android's document picker.
    pub mime_type: String,
    /// UTF-8 document contents.
    pub content: String,
    /// Number of media clips exported.
    pub clip_count: u32,
}

fn fps_of(timeline: &Timeline) -> (i64, i64, f64) {
    let num = timeline.video.rate_num.max(1);
    let den = timeline.video.rate_den.max(1);
    (num, den, num as f64 / den as f64)
}

fn frames_of(seconds: f64, fps: f64) -> i64 {
    (seconds.max(0.0) * fps).round() as i64
}

fn smpte(seconds: f64, fps_int: i64) -> String {
    let f = fps_int.max(1);
    let total = (seconds.max(0.0) * f as f64).round() as i64;
    let ff = total % f;
    let secs = total / f;
    let ss = secs % 60;
    let mins = secs / 60;
    let mm = mins % 60;
    let hh = (mins / 60) % 24;
    format!("{hh:02}:{mm:02}:{ss:02}:{ff:02}")
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn file_url(path: &str) -> String {
    let mut out = String::from("file://localhost");
    if !path.starts_with('/') {
        out.push('/');
    }
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn safe_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let trimmed = s.trim_matches('_');
    if trimmed.is_empty() {
        "ArArCut_Timeline".to_owned()
    } else {
        trimmed.chars().take(60).collect()
    }
}

fn media_clips<'a>(timeline: &'a Timeline) -> Vec<&'a Clip> {
    let mut clips: Vec<&Clip> = timeline
        .clips
        .iter()
        .filter(|c| matches!(c.kind, ClipKind::Video | ClipKind::Image | ClipKind::Audio))
        .collect();
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    clips
}

fn export_edl(timeline: &Timeline, media: &HashMap<&str, &MediaItem>) -> (String, u32) {
    let (_, _, fps) = fps_of(timeline);
    let fps_int = fps.round() as i64;
    let mut out = String::new();
    let _ = writeln!(out, "TITLE: {}", timeline.name);
    let _ = writeln!(out, "FCM: NON-DROP FRAME\n");

    let mut count = 0u32;
    let mut event = 1u32;
    for clip in media_clips(timeline) {
        let item = media.get(clip.media_id.as_str()).copied();
        let track_code = match clip.kind {
            ClipKind::Audio => "A",
            _ => "V",
        };
        let speed = clip.speed.max(0.01);
        let src_in = clip.source_start.max(0.0);
        let src_out = src_in + clip.duration.max(0.01) * speed;
        let rec_in = clip.start.max(0.0);
        let rec_out = rec_in + clip.duration.max(0.01);
        let reel = format!("AX{:04}", event.min(9999));

        let _ = writeln!(
            out,
            "{event:03}  {reel:<8} {track_code:<5} C        {} {} {} {}",
            smpte(src_in, fps_int),
            smpte(src_out, fps_int),
            smpte(rec_in, fps_int),
            smpte(rec_out, fps_int),
        );
        if (speed - 1.0).abs() > 0.005 {
            let _ = writeln!(
                out,
                "M2   {reel:<8}          {:05.1} {}",
                fps * speed,
                smpte(src_in, fps_int),
            );
        }
        let label = if clip.name.is_empty() {
            item.map_or("Clip", |m| m.name.as_str())
        } else {
            clip.name.as_str()
        };
        let _ = writeln!(out, "* FROM CLIP NAME: {label}");
        if let Some(m) = item {
            let _ = writeln!(out, "* SOURCE FILE: {}", m.path);
        }
        out.push('\n');
        event += 1;
        count += 1;
    }
    (out, count)
}

fn export_premiere_xml(timeline: &Timeline, media: &HashMap<&str, &MediaItem>) -> (String, u32) {
    let (rate_num, rate_den, fps) = fps_of(timeline);
    let timebase = fps.round().max(1.0) as i64;
    let ntsc = if rate_den == 1001 { "TRUE" } else { "FALSE" };
    let clips = media_clips(timeline);
    let total_end = clips
        .iter()
        .map(|c| c.start + c.duration)
        .fold(0.0f64, f64::max);
    let seq_frames = frames_of(total_end, fps).max(1);

    let mut out = String::new();
    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(out, "<!DOCTYPE xmeml>");
    let _ = writeln!(out, "<xmeml version=\"5\">");
    let _ = writeln!(out, "  <sequence id=\"sequence-1\">");
    let _ = writeln!(out, "    <name>{}</name>", xml_escape(&timeline.name));
    let _ = writeln!(out, "    <duration>{seq_frames}</duration>");
    let _ = writeln!(out, "    <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>");
    let _ = writeln!(out, "    <media>");

    // Video tracks
    let _ = writeln!(out, "      <video>");
    let _ = writeln!(out, "        <format><samplecharacteristics>");
    let _ = writeln!(out, "          <width>{}</width><height>{}</height>", timeline.video.width, timeline.video.height);
    let _ = writeln!(out, "          <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>");
    let _ = writeln!(out, "        </samplecharacteristics></format>");

    let mut count = 0u32;
    let mut item_idx = 1u32;
    for track in &timeline.tracks {
        let mut tclips: Vec<&Clip> = timeline
            .clips
            .iter()
            .filter(|c| c.track_id == track.id && matches!(c.kind, ClipKind::Video | ClipKind::Image))
            .collect();
        if tclips.is_empty() {
            continue;
        }
        tclips.sort_by(|a, b| a.start.total_cmp(&b.start));
        let _ = writeln!(out, "        <track>");
        for clip in tclips {
            let start_f = frames_of(clip.start, fps);
            let end_f = frames_of(clip.start + clip.duration, fps).max(start_f + 1);
            let in_f = frames_of(clip.source_start, fps);
            let out_f = in_f + frames_of(clip.duration * clip.speed.max(0.01), fps).max(1);
            let item = media.get(clip.media_id.as_str()).copied();
            let file_id = format!("file-{}", clip.media_id);
            let name = xml_escape(if clip.name.is_empty() { "Clip" } else { &clip.name });
            let _ = writeln!(out, "          <clipitem id=\"clipitem-{item_idx}\">");
            let _ = writeln!(out, "            <name>{name}</name>");
            let _ = writeln!(out, "            <start>{start_f}</start><end>{end_f}</end>");
            let _ = writeln!(out, "            <in>{in_f}</in><out>{out_f}</out>");
            let _ = writeln!(out, "            <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>");
            if let Some(m) = item {
                let dur_f = frames_of(m.duration.unwrap_or(clip.duration), fps).max(out_f);
                let _ = writeln!(out, "            <file id=\"{}\">", xml_escape(&file_id));
                let _ = writeln!(out, "              <name>{}</name>", xml_escape(&m.name));
                let _ = writeln!(out, "              <pathurl>{}</pathurl>", xml_escape(&file_url(&m.path)));
                let _ = writeln!(out, "              <duration>{dur_f}</duration>");
                let _ = writeln!(out, "              <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>");
                let _ = writeln!(out, "            </file>");
            }
            let _ = writeln!(out, "          </clipitem>");
            item_idx += 1;
            count += 1;
        }
        let _ = writeln!(out, "        </track>");
    }
    let _ = writeln!(out, "      </video>");

    // Audio tracks
    let _ = writeln!(out, "      <audio>");
    for track in &timeline.tracks {
        let mut tclips: Vec<&Clip> = timeline
            .clips
            .iter()
            .filter(|c| c.track_id == track.id && c.kind == ClipKind::Audio)
            .collect();
        if tclips.is_empty() {
            continue;
        }
        tclips.sort_by(|a, b| a.start.total_cmp(&b.start));
        let _ = writeln!(out, "        <track>");
        for clip in tclips {
            let start_f = frames_of(clip.start, fps);
            let end_f = frames_of(clip.start + clip.duration, fps).max(start_f + 1);
            let in_f = frames_of(clip.source_start, fps);
            let out_f = in_f + frames_of(clip.duration * clip.speed.max(0.01), fps).max(1);
            let item = media.get(clip.media_id.as_str()).copied();
            let file_id = format!("file-{}", clip.media_id);
            let name = xml_escape(if clip.name.is_empty() { "Audio" } else { &clip.name });
            let _ = writeln!(out, "          <clipitem id=\"clipitem-{item_idx}\">");
            let _ = writeln!(out, "            <name>{name}</name>");
            let _ = writeln!(out, "            <start>{start_f}</start><end>{end_f}</end>");
            let _ = writeln!(out, "            <in>{in_f}</in><out>{out_f}</out>");
            let _ = writeln!(out, "            <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>");
            if let Some(m) = item {
                let dur_f = frames_of(m.duration.unwrap_or(clip.duration), fps).max(out_f);
                let _ = writeln!(out, "            <file id=\"{}\">", xml_escape(&file_id));
                let _ = writeln!(out, "              <name>{}</name>", xml_escape(&m.name));
                let _ = writeln!(out, "              <pathurl>{}</pathurl>", xml_escape(&file_url(&m.path)));
                let _ = writeln!(out, "              <duration>{dur_f}</duration>");
                let _ = writeln!(out, "            </file>");
            }
            let _ = writeln!(out, "          </clipitem>");
            item_idx += 1;
            count += 1;
        }
        let _ = writeln!(out, "        </track>");
    }
    let _ = writeln!(out, "      </audio>");
    let _ = writeln!(out, "    </media>");
    let _ = writeln!(out, "  </sequence>");
    let _ = writeln!(out, "</xmeml>");
    let _ = (rate_num, rate_den);
    (out, count)
}

fn export_fcpxml(timeline: &Timeline, media: &HashMap<&str, &MediaItem>) -> (String, u32) {
    let (rate_num, rate_den, fps) = fps_of(timeline);
    let clips = media_clips(timeline);
    let total_end = clips
        .iter()
        .map(|c| c.start + c.duration)
        .fold(0.0f64, f64::max);
    let rational = |sec: f64| -> String {
        let frames = frames_of(sec, fps);
        format!("{}/{rate_num}s", frames * rate_den)
    };

    let mut out = String::new();
    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(out, "<!DOCTYPE fcpxml>");
    let _ = writeln!(out, "<fcpxml version=\"1.11\">");
    let _ = writeln!(out, "  <resources>");
    let _ = writeln!(
        out,
        "    <format id=\"r1\" name=\"FFVideoFormat{}x{}p{}\" frameDuration=\"{rate_den}/{rate_num}s\" width=\"{}\" height=\"{}\"/>",
        timeline.video.width,
        timeline.video.height,
        fps.round() as i64,
        timeline.video.width,
        timeline.video.height,
    );

    let mut asset_ids: HashMap<&str, String> = HashMap::new();
    let mut next_r = 2u32;
    for clip in &clips {
        if let Some(m) = media.get(clip.media_id.as_str()).copied() {
            if !asset_ids.contains_key(m.id.as_str()) {
                let rid = format!("r{next_r}");
                next_r += 1;
                let dur = rational(m.duration.unwrap_or(clip.duration).max(0.1));
                let _ = writeln!(
                    out,
                    "    <asset id=\"{rid}\" name=\"{}\" start=\"0s\" duration=\"{dur}\" hasVideo=\"{}\" hasAudio=\"1\">",
                    xml_escape(&m.name),
                    if matches!(clip.kind, ClipKind::Audio) { 0 } else { 1 },
                );
                let _ = writeln!(
                    out,
                    "      <media-rep kind=\"original-media\" src=\"{}\"/>",
                    xml_escape(&file_url(&m.path)),
                );
                let _ = writeln!(out, "    </asset>");
                asset_ids.insert(m.id.as_str(), rid);
            }
        }
    }
    let _ = writeln!(out, "  </resources>");
    let _ = writeln!(out, "  <library>");
    let _ = writeln!(out, "    <event name=\"ArAr Cut\">");
    let _ = writeln!(out, "      <project name=\"{}\">", xml_escape(&timeline.name));
    let _ = writeln!(
        out,
        "        <sequence format=\"r1\" duration=\"{}\" tcStart=\"0s\" tcFormat=\"NDF\">",
        rational(total_end.max(0.1)),
    );
    let _ = writeln!(out, "          <spine>");

    let mut count = 0u32;
    for clip in &clips {
        let Some(rid) = asset_ids.get(clip.media_id.as_str()) else {
            continue;
        };
        let _ = writeln!(
            out,
            "            <asset-clip ref=\"{rid}\" offset=\"{}\" name=\"{}\" start=\"{}\" duration=\"{}\"/>",
            rational(clip.start),
            xml_escape(if clip.name.is_empty() { "Clip" } else { &clip.name }),
            rational(clip.source_start),
            rational(clip.duration),
        );
        count += 1;
    }

    let _ = writeln!(out, "          </spine>");
    let _ = writeln!(out, "        </sequence>");
    let _ = writeln!(out, "      </project>");
    let _ = writeln!(out, "    </event>");
    let _ = writeln!(out, "  </library>");
    let _ = writeln!(out, "</fcpxml>");
    (out, count)
}

fn export_otio(timeline: &Timeline, media: &HashMap<&str, &MediaItem>) -> (String, u32) {
    let (_, _, fps) = fps_of(timeline);
    let rt = |sec: f64| -> Value {
        json!({
            "OTIO_SCHEMA": "RationalTime.1",
            "rate": fps,
            "value": (sec.max(0.0) * fps).round(),
        })
    };

    let mut tracks_json = Vec::new();
    let mut count = 0u32;
    for (idx, track) in timeline.tracks.iter().enumerate() {
        let mut tclips: Vec<&Clip> = timeline
            .clips
            .iter()
            .filter(|c| {
                c.track_id == track.id
                    && matches!(c.kind, ClipKind::Video | ClipKind::Image | ClipKind::Audio)
            })
            .collect();
        if tclips.is_empty() {
            continue;
        }
        tclips.sort_by(|a, b| a.start.total_cmp(&b.start));
        let is_audio = tclips.iter().all(|c| c.kind == ClipKind::Audio);
        let mut children = Vec::new();
        let mut cursor = 0.0f64;
        for clip in tclips {
            if clip.start > cursor + 0.001 {
                children.push(json!({
                    "OTIO_SCHEMA": "Gap.1",
                    "name": "",
                    "source_range": {
                        "OTIO_SCHEMA": "TimeRange.1",
                        "start_time": rt(0.0),
                        "duration": rt(clip.start - cursor),
                    }
                }));
            }
            let item = media.get(clip.media_id.as_str()).copied();
            let media_ref = match item {
                Some(m) => json!({
                    "OTIO_SCHEMA": "ExternalReference.1",
                    "target_url": file_url(&m.path),
                    "available_range": {
                        "OTIO_SCHEMA": "TimeRange.1",
                        "start_time": rt(0.0),
                        "duration": rt(m.duration.unwrap_or(clip.duration)),
                    }
                }),
                None => json!({ "OTIO_SCHEMA": "MissingReference.1" }),
            };
            children.push(json!({
                "OTIO_SCHEMA": "Clip.2",
                "name": if clip.name.is_empty() { "Clip" } else { &clip.name },
                "media_references": { "DEFAULT_MEDIA": media_ref },
                "active_media_reference_key": "DEFAULT_MEDIA",
                "source_range": {
                    "OTIO_SCHEMA": "TimeRange.1",
                    "start_time": rt(clip.source_start),
                    "duration": rt(clip.duration * clip.speed.max(0.01)),
                },
                "metadata": {
                    "ararcut": {
                        "speed": clip.speed,
                        "volume": clip.volume,
                        "opacity": clip.opacity,
                    }
                }
            }));
            cursor = (clip.start + clip.duration).max(cursor);
            count += 1;
        }
        tracks_json.push(json!({
            "OTIO_SCHEMA": "Track.1",
            "name": format!("{}{}", if is_audio { "A" } else { "V" }, idx + 1),
            "kind": if is_audio { "Audio" } else { "Video" },
            "children": children,
        }));
    }

    let doc = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": timeline.name,
        "global_start_time": rt(0.0),
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "children": tracks_json,
        },
        "metadata": {
            "generator": "ArAr Cut",
            "width": timeline.video.width,
            "height": timeline.video.height,
            "fps": fps,
        }
    });
    let text = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_owned());
    (text, count)
}

#[uniffi::export]
impl MobileSession {
    /// Exports the active timeline into a desktop NLE project format (`EDL`, `Premiere XML`,
    /// `FCPXML`, or `OpenTimelineIO .otio`).
    pub fn export_nle(&self, format: NleFormat) -> Result<NleExportResult, MobileError> {
        self.with_session(|session| {
            let project = session.project();
            let timeline = project.active();
            let media: HashMap<&str, &MediaItem> =
                project.media.iter().map(|m| (m.id.as_str(), m)).collect();
            let stem = safe_stem(&timeline.name);
            let (content, clip_count, ext, mime) = match format {
                NleFormat::Edl => {
                    let (c, n) = export_edl(timeline, &media);
                    (c, n, "edl", "text/plain")
                }
                NleFormat::PremiereXml => {
                    let (c, n) = export_premiere_xml(timeline, &media);
                    (c, n, "xml", "application/xml")
                }
                NleFormat::Fcpxml => {
                    let (c, n) = export_fcpxml(timeline, &media);
                    (c, n, "fcpxml", "application/xml")
                }
                NleFormat::Otio => {
                    let (c, n) = export_otio(timeline, &media);
                    (c, n, "otio", "application/json")
                }
            };
            if clip_count == 0 {
                return Err(MobileError::Failed(
                    "Add at least one video, image, or audio clip before exporting a project."
                        .to_owned(),
                ));
            }
            Ok(NleExportResult {
                format,
                filename: format!("ArArCut_{stem}.{ext}"),
                mime_type: mime.to_owned(),
                content,
                clip_count,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smpte_and_xml_escape_work() {
        assert_eq!(smpte(0.0, 30), "00:00:00:00");
        assert_eq!(smpte(3661.5, 30), "01:01:01:15");
        assert_eq!(xml_escape("A & B <C> \"D\""), "A &amp; B &lt;C&gt; &quot;D&quot;");
        assert_eq!(file_url("/storage/emulated/0/My Clip.mp4"), "file://localhost/storage/emulated/0/My%20Clip.mp4");
    }
}
