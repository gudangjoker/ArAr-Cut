// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors
//
// Integrates FilmCraft's pure-Rust `filmcraft-color` and `filmcraft-scopes` crates
// (MIT OR Apache-2.0) for live broadcast video scopes and GPU-accelerated Camera Log
// color transforms via 33x33x33 `.cube` 3D LUT baking.

//! Pro color science and live broadcast scopes powered by `filmcraft-color` + `filmcraft-scopes`:
//! - Live Waveform (RGB/Luma), RGB Parade, YUV Vectorscope, and 256-bin Histogram rendered from preview frames.
//! - 12 pro Camera Log -> Rec.709 color transforms (Sony S-Log3, Panasonic V-Log, Canon Log 2/3,
//!   ARRI LogC3/LogC4, Apple Log, DJI D-Log, Fujifilm F-Log/F-Log2/F-Log2 C) baked into 3D `.cube`
//!   LUTs so they execute at 60 fps on the `wgpu` GPU compositor.

use std::fmt::Write as _;

use filmcraft_color::transform::InputTransform;
use filmcraft_color::{ColorPipeline, ColorSpace, Range, linear_to_srgb};
use filmcraft_scopes::{Matrix, ParadeType, Params, Signal, WaveformType, histogram, paint, parade, vectorscope_yuv, waveform};

use crate::{MobileError, Picture, context};

/// Supported live video scope display modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoScopeKind {
    /// RGB overlay waveform.
    WaveformRgb,
    /// Luma (Y') waveform.
    WaveformLuma,
    /// Side-by-side R, G, B parade.
    ParadeRgb,
    /// Colorized YUV chroma vectorscope.
    VectorscopeYuv,
    /// 256-bin RGB + Luma histogram.
    Histogram,
}

/// One pro camera log color space profile supported by `filmcraft-color`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CameraLogProfile {
    /// Stable profile id (e.g. `"slog3-sgamut3cine"`).
    pub id: String,
    /// Display name (e.g. `"Sony S-Log3 (S-Gamut3.Cine)"`).
    pub name: String,
    /// Manufacturer / camera family.
    pub camera: String,
    /// Wide source gamut name.
    pub gamut: String,
}

struct LogDef {
    id: &'static str,
    name: &'static str,
    camera: &'static str,
    gamut: &'static str,
    space: ColorSpace,
}

const LOG_PROFILES: &[LogDef] = &[
    LogDef { id: "slog3-sgamut3cine", name: "S-Log3 / S-Gamut3.Cine", camera: "Sony", gamut: "S-Gamut3.Cine", space: ColorSpace::SLog3SGamut3Cine },
    LogDef { id: "slog3-sgamut3", name: "S-Log3 / S-Gamut3", camera: "Sony", gamut: "S-Gamut3", space: ColorSpace::SLog3SGamut3 },
    LogDef { id: "apple-log", name: "Apple Log (iPhone Pro)", camera: "Apple", gamut: "BT.2020", space: ColorSpace::AppleLog },
    LogDef { id: "vlog-vgamut", name: "V-Log / V-Gamut", camera: "Panasonic", gamut: "V-Gamut", space: ColorSpace::VLogVGamut },
    LogDef { id: "clog2-cinema", name: "Canon Log 2", camera: "Canon", gamut: "Cinema Gamut", space: ColorSpace::CLog2CinemaGamut },
    LogDef { id: "clog3-cinema", name: "Canon Log 3", camera: "Canon", gamut: "Cinema Gamut", space: ColorSpace::CLog3CinemaGamut },
    LogDef { id: "logc3-awg3", name: "ARRI LogC3", camera: "ARRI", gamut: "AWG3", space: ColorSpace::LogC3Awg3 },
    LogDef { id: "logc4-awg4", name: "ARRI LogC4", camera: "ARRI", gamut: "AWG4", space: ColorSpace::LogC4Awg4 },
    LogDef { id: "dlog-dgamut", name: "DJI D-Log", camera: "DJI", gamut: "D-Gamut", space: ColorSpace::DLogDGamut },
    LogDef { id: "flog-fgamut", name: "Fujifilm F-Log", camera: "Fujifilm", gamut: "BT.2020", space: ColorSpace::FLogFGamut },
    LogDef { id: "flog2-fgamut", name: "Fujifilm F-Log2", camera: "Fujifilm", gamut: "BT.2020", space: ColorSpace::FLog2FGamut },
    LogDef { id: "flog2c-fgamutc", name: "Fujifilm F-Log2 C", camera: "Fujifilm", gamut: "F-Gamut C", space: ColorSpace::FLog2CFGamutC },
];

/// Lists all supported pro Camera Log -> Rec.709 color transform profiles.
#[uniffi::export]
pub fn camera_log_profiles() -> Vec<CameraLogProfile> {
    LOG_PROFILES
        .iter()
        .map(|d| CameraLogProfile {
            id: d.id.to_owned(),
            name: d.name.to_owned(),
            camera: d.camera.to_owned(),
            gamut: d.gamut.to_owned(),
        })
        .collect()
}

/// Bakes `space` -> Rec.709 (with BT.2390 highlight tone mapping and ACES gamut compression)
/// into a standard `.cube` 3D LUT string of size `size^3`.
fn bake_cube_lut(title: &str, space: ColorSpace, size: usize) -> String {
    let xform = InputTransform::new(space, Range::Full, &ColorPipeline::REC709, None);
    let mut out = String::with_capacity(size * size * size * 28 + 128);
    let _ = writeln!(out, "TITLE \"{title}\"");
    let _ = writeln!(out, "LUT_3D_SIZE {size}");
    let _ = writeln!(out, "DOMAIN_MIN 0.0 0.0 0.0");
    let _ = writeln!(out, "DOMAIN_MAX 1.0 1.0 1.0");
    let denom = (size - 1).max(1) as f32;
    // Standard Adobe .cube ordering: red changes fastest, blue slowest.
    for bi in 0..size {
        let b = bi as f32 / denom;
        for gi in 0..size {
            let g = gi as f32 / denom;
            for ri in 0..size {
                let r = ri as f32 / denom;
                let lin = xform.convert([r, g, b]);
                let ro = linear_to_srgb(lin[0].clamp(0.0, 1.0)).clamp(0.0, 1.0);
                let go = linear_to_srgb(lin[1].clamp(0.0, 1.0)).clamp(0.0, 1.0);
                let bo = linear_to_srgb(lin[2].clamp(0.0, 1.0)).clamp(0.0, 1.0);
                let _ = writeln!(out, "{ro:.6} {go:.6} {bo:.6}");
            }
        }
    }
    out
}

/// Generates (if needed) and registers a Camera Log -> Rec.709 3D LUT as a GPU shader filter
/// via [`crate::looks::import_look`], returning the filter package id ready to apply to any clip.
#[uniffi::export]
pub fn install_camera_log_look(profile_id: String) -> Result<String, MobileError> {
    let def = LOG_PROFILES
        .iter()
        .find(|d| d.id == profile_id)
        .ok_or_else(|| MobileError::Failed(format!("Unknown camera log profile: {profile_id}")))?;
    let folder = context()?.cache.join("camera-log-luts");
    std::fs::create_dir_all(&folder)
        .map_err(|error| MobileError::Failed(format!("{}: {error}", folder.display())))?;
    let safe_name = format!("{} - {} to Rec709.cube", def.camera, def.name.replace('/', "-"));
    let file = folder.join(safe_name);
    if !file.is_file() {
        let cube = bake_cube_lut(def.name, def.space, 33);
        let part = file.with_extension("cube.part");
        std::fs::write(&part, cube)
            .and_then(|()| std::fs::rename(&part, &file))
            .map_err(|error| MobileError::Failed(format!("{}: {error}", file.display())))?;
    }
    crate::looks::import_look(file.to_string_lossy().into_owned())
}

/// Composites three parade grids side by side into one wide RGBA8 image.
fn paint_parade_side_by_side(
    grids: &[filmcraft_scopes::Grid],
    per_column: usize,
    gain: f32,
) -> (usize, usize, Vec<u8>) {
    let colors: [[f32; 3]; 3] = [[1.0, 0.25, 0.25], [0.25, 1.0, 0.35], [0.3, 0.55, 1.0]];
    let reference = paint::waveform_reference(per_column);
    let panels: Vec<(usize, usize, Vec<u8>)> = grids
        .iter()
        .zip(colors.iter())
        .map(|(g, &col)| paint::grids(&[(g, col)], reference, gain))
        .collect();
    let Some(&(_, h, _)) = panels.first() else {
        return (0, 0, Vec::new());
    };
    let gap = 2usize;
    let total_w: usize = panels.iter().map(|p| p.0).sum::<usize>() + gap * panels.len().saturating_sub(1);
    let mut out = vec![0u8; total_w * h * 4];
    let mut x_off = 0usize;
    for (pw, ph, data) in panels {
        if ph == h {
            for y in 0..h {
                let src_start = y * pw * 4;
                let dst_start = (y * total_w + x_off) * 4;
                out[dst_start..dst_start + pw * 4].copy_from_slice(&data[src_start..src_start + pw * 4]);
            }
        }
        x_off += pw + gap;
    }
    (total_w, h, out)
}

/// Renders a 256x128 RGB + Luma histogram into an RGBA8 buffer.
fn paint_histogram(h: &filmcraft_scopes::Histogram) -> (usize, usize, Vec<u8>) {
    let width = filmcraft_scopes::BINS;
    let height = 128usize;
    let peak = h
        .r
        .iter()
        .chain(h.g.iter())
        .chain(h.b.iter())
        .chain(h.y.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let mut out = vec![0u8; width * height * 4];
    for x in 0..width {
        let hr = ((h.r[x] as f32 / peak).sqrt() * (height - 1) as f32).round() as usize;
        let hg = ((h.g[x] as f32 / peak).sqrt() * (height - 1) as f32).round() as usize;
        let hb = ((h.b[x] as f32 / peak).sqrt() * (height - 1) as f32).round() as usize;
        let hy = ((h.y[x] as f32 / peak).sqrt() * (height - 1) as f32).round() as usize;
        for row in 0..height {
            let y = height - 1 - row;
            let idx = (y * width + x) * 4;
            let mut r = if row <= hr { 220u16 } else { 0 };
            let mut g = if row <= hg { 220u16 } else { 0 };
            let mut b = if row <= hb { 220u16 } else { 0 };
            if row <= hy {
                r = (r + 90).min(255);
                g = (g + 90).min(255);
                b = (b + 90).min(255);
            }
            let a = r.max(g).max(b);
            if a > 0 {
                out[idx] = r as u8;
                out[idx + 1] = g as u8;
                out[idx + 2] = b as u8;
                out[idx + 3] = a.min(240) as u8;
            }
        }
    }
    (width, height, out)
}

/// Computes and renders a live broadcast video scope from a preview RGBA8 frame.
#[uniffi::export]
pub fn render_video_scope(
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    kind: VideoScopeKind,
) -> Result<Picture, MobileError> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Err(MobileError::Failed("Invalid frame buffer for video scope.".to_owned()));
    }
    let signal = Signal::from_rgba8_max(w, h, &rgba, 256, 144);
    let params = Params {
        matrix: Matrix::Bt709,
        clamp: true,
        rows: 144,
        vector_size: 161,
    };
    let (sw, sh, pixels) = match kind {
        VideoScopeKind::WaveformRgb => {
            let wf = waveform(&signal, WaveformType::Rgb, &params);
            let reference = paint::waveform_reference(wf.per_column);
            let mut layers = Vec::new();
            if let Some(g) = wf.trace("R") {
                layers.push((g, [1.0, 0.25, 0.25]));
            }
            if let Some(g) = wf.trace("G") {
                layers.push((g, [0.25, 1.0, 0.35]));
            }
            if let Some(g) = wf.trace("B") {
                layers.push((g, [0.3, 0.55, 1.0]));
            }
            paint::grids(&layers, reference, 1.4)
        }
        VideoScopeKind::WaveformLuma => {
            let wf = waveform(&signal, WaveformType::Luma, &params);
            let reference = paint::waveform_reference(wf.per_column);
            let layers: Vec<_> = wf
                .trace("Y")
                .into_iter()
                .map(|g| (g, [0.25, 0.92, 1.0]))
                .collect();
            paint::grids(&layers, reference, 1.4)
        }
        VideoScopeKind::ParadeRgb => {
            let pr = parade(&signal, ParadeType::Rgb, &params);
            paint_parade_side_by_side(&pr.traces, pr.per_column, 1.4)
        }
        VideoScopeKind::VectorscopeYuv => {
            let vs = vectorscope_yuv(&signal, &params);
            paint::vectorscope(&vs, [0.25, 0.92, 1.0], 1.5, true)
        }
        VideoScopeKind::Histogram => {
            let hist = histogram(&signal, &params);
            paint_histogram(&hist)
        }
    };
    Ok(Picture {
        width: sw as u32,
        height: sh as u32,
        rgba: pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_log_lut_bakes_valid_cube() {
        assert_eq!(camera_log_profiles().len(), 12);
        let cube = bake_cube_lut("S-Log3 Test", ColorSpace::SLog3SGamut3Cine, 5);
        assert!(cube.contains("LUT_3D_SIZE 5"));
        let data_lines = cube
            .lines()
            .filter(|l| !l.starts_with("TITLE") && !l.starts_with("LUT_3D") && !l.starts_with("DOMAIN"))
            .count();
        assert_eq!(data_lines, 5 * 5 * 5);
    }

    #[test]
    fn all_video_scopes_render_non_empty() {
        let mut frame = vec![0u8; 32 * 18 * 4];
        for (i, px) in frame.chunks_exact_mut(4).enumerate() {
            px[0] = ((i * 7) % 256) as u8;
            px[1] = ((i * 13) % 256) as u8;
            px[2] = ((i * 23) % 256) as u8;
            px[3] = 255;
        }
        for kind in [
            VideoScopeKind::WaveformRgb,
            VideoScopeKind::WaveformLuma,
            VideoScopeKind::ParadeRgb,
            VideoScopeKind::VectorscopeYuv,
            VideoScopeKind::Histogram,
        ] {
            let pic = render_video_scope(frame.clone(), 32, 18, kind).expect("scope rendered");
            assert!(pic.width > 0 && pic.height > 0);
            assert_eq!(pic.rgba.len(), (pic.width * pic.height * 4) as usize);
        }
    }
}
