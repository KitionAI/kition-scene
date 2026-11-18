//! Colour management for final renders: exposure, a view transform, and a look, turning the
//! scene-linear light a render keeps into display-linear values between 0 and 1, which are then
//! encoded as sRGB (the reference's Render Properties ▸ Color Management panel: View Transform,
//! Look, Exposure).
//!
//! - **Standard** clips at white, as the reference's Standard view does. With no look and no
//!   exposure it is exactly what renders did before colour management existed.
//! - **AgX** rolls highlights off towards white and desaturates them, after Troy Sobotka's AgX.
//!   The curve is Benjamin Wrensch's published polynomial fit with its inset and outset matrices
//!   ("Minimal AgX Implementation", 2024; MIT License, Copyright (c) 2024 Missing Deadlines
//!   (Benjamin Wrensch); permission is hereby granted, free of charge, to any person obtaining a
//!   copy of this software and associated documentation files, to deal in the software without
//!   restriction, subject to including this copyright and permission notice; the software is
//!   provided "as is", without warranty of any kind). It approximates AgX's default contrast;
//!   it is not the reference's own configuration, so pictures are close in character, not equal.
//! - **Khronos PBR Neutral** is the Khronos Group's published tone mapper (KhronosGroup/
//!   ToneMapping, Apache-2.0): colours below 0.76 pass through unchanged and highlights compress
//!   towards white.
//!
//! Looks act on the display-encoded picture (a 2.2 power): Punchy (Wrensch's ASC CDL: power 1.35,
//! saturation 1.4), Greyscale, and High and Low Contrast (contrast of 1.3 and 0.75 about the
//! encoded middle grey), the reference's look names.

use serde::{Deserialize, Serialize};

/// How scene light becomes display light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ViewTransform {
    /// Clipped at white.
    #[default]
    Standard,
    /// Highlights roll off towards white.
    Agx,
    /// The Khronos PBR Neutral curve.
    KhronosPbrNeutral,
}

impl ViewTransform {
    pub const NAMES: [&'static str; 3] = ["standard", "agx", "khronos-pbr-neutral"];

    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Agx => "agx",
            Self::KhronosPbrNeutral => "khronos-pbr-neutral",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "standard" => Some(Self::Standard),
            "agx" => Some(Self::Agx),
            "khronos-pbr-neutral" => Some(Self::KhronosPbrNeutral),
            _ => None,
        }
    }
}

/// A grade after the view transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Look {
    #[default]
    None,
    Punchy,
    Greyscale,
    HighContrast,
    LowContrast,
}

impl Look {
    pub const NAMES: [&'static str; 5] = ["none", "punchy", "greyscale", "high-contrast", "low-contrast"];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Punchy => "punchy",
            Self::Greyscale => "greyscale",
            Self::HighContrast => "high-contrast",
            Self::LowContrast => "low-contrast",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "punchy" => Some(Self::Punchy),
            "greyscale" => Some(Self::Greyscale),
            "high-contrast" => Some(Self::HighContrast),
            "low-contrast" => Some(Self::LowContrast),
            _ => None,
        }
    }
}

/// A render's colour management.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorManagement {
    pub view: ViewTransform,
    pub look: Look,
    /// Stops: light is scaled by 2 to this power before the view transform.
    pub exposure: f32,
}

impl Default for ColorManagement {
    fn default() -> Self {
        Self { view: ViewTransform::Standard, look: Look::None, exposure: 0.0 }
    }
}

/// Rec. 709 luminance weights.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// The power the looks' display encoding uses.
const DISPLAY_GAMMA: f32 = 2.2;
/// Scene-linear middle grey, the contrast looks' pivot.
const MIDDLE_GREY: f32 = 0.18;

impl ColorManagement {
    /// The largest exposure either way, in stops (the reference's limit).
    pub const MAX_EXPOSURE: f32 = 32.0;

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Whether this is plain clipping at white (Standard, no look, no exposure).
    pub fn is_standard(&self) -> bool {
        self.is_default()
    }

    /// Display-linear RGB, each between 0 and 1, for scene-linear `rgb`.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let exposure = if self.exposure.is_finite() { self.exposure.clamp(-Self::MAX_EXPOSURE, Self::MAX_EXPOSURE) } else { 0.0 };
        let scale = exposure.exp2();
        let rgb = rgb.map(|value| if value.is_finite() { (value * scale).max(0.0) } else { 0.0 });
        let out = match self.view {
            ViewTransform::Standard => {
                let clipped = rgb.map(|value| value.min(1.0));
                if self.look == Look::None { clipped } else { self.graded(clipped) }
            }
            ViewTransform::KhronosPbrNeutral => {
                let mapped = khronos_pbr_neutral(rgb);
                if self.look == Look::None { mapped } else { self.graded(mapped) }
            }
            ViewTransform::Agx => {
                let encoded = agx_encode(rgb);
                let pivot = agx_encode([MIDDLE_GREY; 3]);
                agx_decode(look(self.look, encoded, pivot.iter().sum::<f32>() / 3.0))
            }
        };
        out.map(|value| if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 })
    }

    /// Display-linear `rgb` through the look, by way of the display encoding.
    fn graded(&self, rgb: [f32; 3]) -> [f32; 3] {
        let encode = |value: f32| value.clamp(0.0, 1.0).powf(1.0 / DISPLAY_GAMMA);
        let pivot = encode(match self.view {
            ViewTransform::KhronosPbrNeutral => khronos_pbr_neutral([MIDDLE_GREY; 3])[0],
            _ => MIDDLE_GREY,
        });
        look(self.look, rgb.map(encode), pivot).map(|value| value.max(0.0).powf(DISPLAY_GAMMA))
    }
}

/// A look applied to display-encoded `rgb`; `pivot` is middle grey's encoded value.
fn look(look: Look, rgb: [f32; 3], pivot: f32) -> [f32; 3] {
    let luma = |rgb: [f32; 3]| rgb.iter().zip(LUMA).map(|(value, weight)| value * weight).sum::<f32>();
    match look {
        Look::None => rgb,
        Look::Punchy => {
            let before = luma(rgb);
            rgb.map(|value| before + 1.4 * (value.max(0.0).powf(1.35) - before))
        }
        Look::Greyscale => [luma(rgb); 3],
        Look::HighContrast => rgb.map(|value| pivot + (value - pivot) * 1.3),
        Look::LowContrast => rgb.map(|value| pivot + (value - pivot) * 0.75),
    }
}

/// `matrix` (rows) times `rgb`.
fn mul(matrix: [[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    matrix.map(|row| row.iter().zip(rgb).map(|(weight, value)| weight * value).sum())
}

/// AgX's inset: primaries pulled towards white so bright saturated colours desaturate.
const AGX_INSET: [[f32; 3]; 3] =
    [[0.842_479_06, 0.078_433_6, 0.079_223_745], [0.042_328_242, 0.878_468_6, 0.079_166_13], [0.042_375_655, 0.078_433_6, 0.879_143]];
/// The inverse of the inset, applied after the curve.
const AGX_OUTSET: [[f32; 3]; 3] =
    [[1.196_879, -0.098_020_88, -0.099_029_74], [-0.052_896_852, 1.151_903_1, -0.098_961_18], [-0.052_971_636, -0.098_043_45, 1.151_073_7]];
/// The stops the log encoding spans (about 16.5 around middle grey).
const AGX_MIN_EV: f32 = -12.473_93;
const AGX_MAX_EV: f32 = 4.026_069;

/// Scene-linear light as AgX's display encoding: inset, log2 between the two exposures, and the
/// sigmoid (a sixth-degree polynomial fit).
fn agx_encode(rgb: [f32; 3]) -> [f32; 3] {
    mul(AGX_INSET, rgb).map(|value| {
        let stops = value.max(1e-10).log2().clamp(AGX_MIN_EV, AGX_MAX_EV);
        let x = (stops - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);
        let (x2, x4) = (x * x, x * x * x * x);
        15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232
    })
}

/// AgX's display encoding back to display-linear light: outset, then the 2.2 power.
fn agx_decode(rgb: [f32; 3]) -> [f32; 3] {
    mul(AGX_OUTSET, rgb).map(|value| value.max(0.0).powf(DISPLAY_GAMMA))
}

/// The Khronos PBR Neutral tone mapper.
fn khronos_pbr_neutral(rgb: [f32; 3]) -> [f32; 3] {
    const START_COMPRESSION: f32 = 0.8 - 0.04;
    const DESATURATION: f32 = 0.15;
    let lowest = rgb[0].min(rgb[1]).min(rgb[2]);
    let offset = if lowest < 0.08 { lowest - 6.25 * lowest * lowest } else { 0.04 };
    let rgb = rgb.map(|value| value - offset);
    let peak = rgb[0].max(rgb[1]).max(rgb[2]);
    if peak < START_COMPRESSION {
        return rgb;
    }
    let d = 1.0 - START_COMPRESSION;
    let new_peak = 1.0 - d * d / (peak + d - START_COMPRESSION);
    let scaled = rgb.map(|value| value * new_peak / peak);
    let g = 1.0 - 1.0 / (DESATURATION * (peak - new_peak) + 1.0);
    scaled.map(|value| value + (new_peak - value) * g)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tolerance)
    }

    #[test]
    fn standard_without_exposure_clips_at_white() {
        let standard = ColorManagement::default();
        assert!(standard.is_standard());
        assert_eq!(standard.apply([0.25, 1.0, 3.0]), [0.25, 1.0, 1.0]);
        assert_eq!(standard.apply([-1.0, f32::NAN, f32::INFINITY]), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn exposure_is_in_stops() {
        let brighter = ColorManagement { exposure: 1.0, ..ColorManagement::default() };
        assert_eq!(brighter.apply([0.25, 0.1, 0.6]), [0.5, 0.2, 1.0]);
        let darker = ColorManagement { exposure: -2.0, ..ColorManagement::default() };
        assert_eq!(darker.apply([1.0, 2.0, 8.0]), [0.25, 0.5, 1.0]);
    }

    #[test]
    fn agx_rolls_highlights_off_without_clipping() {
        let agx = ColorManagement { view: ViewTransform::Agx, ..ColorManagement::default() };
        let grey = agx.apply([0.18; 3]);
        // Middle grey lands near the middle of the display (about 0.2 linear, 0.48 encoded).
        assert!(grey[0] > 0.12 && grey[0] < 0.3, "{grey:?}");
        // Brighter light stays brighter all the way up: 1, 4, and 16 are told apart.
        let steps: Vec<f32> = [1.0, 4.0, 16.0].iter().map(|value| agx.apply([*value; 3])[1]).collect();
        assert!(steps.windows(2).all(|pair| pair[0] < pair[1]) && steps[2] <= 1.0, "{steps:?}");
        // A bright saturated red goes towards white rather than clipping to pure red.
        let red = agx.apply([8.0, 0.2, 0.2]);
        assert!(red[1] > 0.2 && red[0] > red[1], "{red:?}");
    }

    #[test]
    fn khronos_neutral_keeps_low_values_and_compresses_highlights() {
        let neutral = ColorManagement { view: ViewTransform::KhronosPbrNeutral, ..ColorManagement::default() };
        // Above 0.08 every channel is offset by 0.04 and nothing else happens below 0.76.
        assert!(near(neutral.apply([0.5, 0.3, 0.2]), [0.46, 0.26, 0.16], 1e-6));
        let white = neutral.apply([10.0; 3]);
        assert!(white[0] > 0.97 && white[0] <= 1.0, "{white:?}");
    }

    #[test]
    fn looks_grade_the_display_picture() {
        let grey = ColorManagement { look: Look::Greyscale, ..ColorManagement::default() };
        let out = grey.apply([0.8, 0.1, 0.1]);
        assert!(out[0] == out[1] && out[1] == out[2]);
        let high = ColorManagement { look: Look::HighContrast, ..ColorManagement::default() };
        let low = ColorManagement { look: Look::LowContrast, ..ColorManagement::default() };
        let (dark, bright) = ([0.03; 3], [0.7; 3]);
        assert!(high.apply(dark)[0] < low.apply(dark)[0] && high.apply(bright)[0] > low.apply(bright)[0]);
        // Middle grey is the pivot.
        assert!(near(high.apply([0.18; 3]), [0.18; 3], 1e-4));
        let punchy = ColorManagement { view: ViewTransform::Agx, look: Look::Punchy, ..ColorManagement::default() };
        let plain = ColorManagement { view: ViewTransform::Agx, ..ColorManagement::default() };
        let (a, b) = (punchy.apply([0.6, 0.1, 0.05]), plain.apply([0.6, 0.1, 0.05]));
        assert!(a[0] - a[2] > b[0] - b[2], "punchy is more saturated: {a:?} {b:?}");
    }

    #[test]
    fn settings_save_by_name() {
        let settings = ColorManagement { view: ViewTransform::KhronosPbrNeutral, look: Look::HighContrast, exposure: 0.5 };
        let text = serde_json::to_string(&settings).unwrap_or_default();
        assert_eq!(text, r#"{"view":"khronos-pbr-neutral","look":"high-contrast","exposure":0.5}"#);
        for name in ViewTransform::NAMES {
            assert_eq!(ViewTransform::from_name(name).map(ViewTransform::name), Some(name));
        }
        for name in Look::NAMES {
            assert_eq!(Look::from_name(name).map(Look::name), Some(name));
        }
    }
}
