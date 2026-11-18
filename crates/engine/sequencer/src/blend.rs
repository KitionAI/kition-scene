//! Laying one picture over another with a strip's blend mode and opacity. Pixels are straight
//! (not premultiplied) display values from 0 to 1, as the reference's sequencer keeps them, and
//! each mode treats alpha the way the reference does (checked in `tests/sequencer_blender.rs`):
//!
//! - Cross mixes all four channels by the opacity; Gamma Cross mixes their premultiplied square
//!   roots; Alpha Over and Under lay the premultiplied picture (scaled by the opacity) over or
//!   under; Replace takes the upper picture, its colour scaled by the opacity.
//! - Multiply scales all four channels below by `1 − o + o × above`.
//! - Add and Subtract add the upper colour times its alpha and the opacity; the alpha below stays.
//! - The other modes mix the colour below towards the mode's result by the upper alpha times the
//!   opacity; the alpha below stays (so over nothing they leave nothing).

use kition_scene_model::sequencer::Blend;

pub type Pixel = [f32; 4];

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn clamp01(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }
}

fn premultiply(pixel: Pixel) -> Pixel {
    [pixel[0] * pixel[3], pixel[1] * pixel[3], pixel[2] * pixel[3], pixel[3]]
}

fn straight(pixel: Pixel) -> Pixel {
    let alpha = pixel[3];
    if alpha <= 1e-6 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha, alpha]
}

/// Hue, saturation, and value of a colour (hue 0 to 1).
pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let saturation = if max > 0.0 { delta / max } else { 0.0 };
    let hue = if delta <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    [hue, saturation, max]
}

/// A colour from hue (0 to 1), saturation, and value; a saturation above 1 pushes the low
/// channels below zero, as the reference does.
pub fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> [f32; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let sector = h.floor();
    let fraction = h - sector;
    let low = value * (1.0 - saturation);
    let falling = value * (1.0 - saturation * fraction);
    let rising = value * (1.0 - saturation * (1.0 - fraction));
    match sector as i32 {
        0 => [value, rising, low],
        1 => [falling, value, low],
        2 => [low, value, rising],
        3 => [low, falling, value],
        4 => [rising, low, value],
        _ => [value, low, falling],
    }
}

fn burn(a: f32, b: f32) -> f32 {
    if b <= 0.0 { 0.0 } else { (1.0 - (1.0 - a) / b).max(0.0) }
}

fn dodge(a: f32, b: f32) -> f32 {
    if b >= 1.0 { 1.0 } else { (a / (1.0 - b)).min(1.0) }
}

/// A colour mode's result per channel.
fn channel(mode: Blend, a: f32, b: f32) -> f32 {
    match mode {
        Blend::Darken => a.min(b),
        Blend::Lighten => a.max(b),
        Blend::Screen => 1.0 - (1.0 - a) * (1.0 - b),
        Blend::ColorBurn => burn(a, b),
        Blend::LinearBurn => (a + b - 1.0).max(0.0),
        Blend::ColorDodge => dodge(a, b),
        Blend::Overlay => {
            if a > 0.5 {
                1.0 - 2.0 * (1.0 - a) * (1.0 - b)
            } else {
                2.0 * a * b
            }
        }
        Blend::SoftLight => (1.0 - 2.0 * b) * a * a + 2.0 * a * b,
        Blend::HardLight => {
            if b > 0.5 {
                1.0 - 2.0 * (1.0 - a) * (1.0 - b)
            } else {
                2.0 * a * b
            }
        }
        Blend::VividLight => {
            if b < 0.5 {
                burn(a, 2.0 * b)
            } else {
                dodge(a, 2.0 * (b - 0.5))
            }
        }
        Blend::LinearLight => a + 2.0 * b - 1.0,
        Blend::PinLight => {
            if b < 0.5 {
                a.min(2.0 * b)
            } else {
                a.max(2.0 * b - 1.0)
            }
        }
        Blend::Difference => (a - b).abs(),
        Blend::Exclusion => a + b - 2.0 * a * b,
        _ => a,
    }
}

/// A colour mode's result for a whole colour (the HSV modes work on all three at once).
fn colour(mode: Blend, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    match mode {
        Blend::Hue | Blend::Saturation | Blend::Color | Blend::Value => {
            let [ha, sa, va] = rgb_to_hsv(a);
            let [hb, sb, vb] = rgb_to_hsv(b);
            match mode {
                Blend::Hue if sb > 0.0 => hsv_to_rgb([hb, sa, va]),
                Blend::Saturation if sa > 0.0 => hsv_to_rgb([ha, sb, va]),
                Blend::Color if sb > 0.0 => hsv_to_rgb([hb, sb, va]),
                Blend::Value => hsv_to_rgb([ha, sa, vb]),
                _ => a,
            }
        }
        _ => [channel(mode, a[0], b[0]), channel(mode, a[1], b[1]), channel(mode, a[2], b[2])],
    }
}

/// Square-root mix of premultiplied pixels (Gamma Cross).
fn gamma_cross(a: Pixel, b: Pixel, t: f32) -> Pixel {
    let (pa, pb) = (premultiply(a), premultiply(b));
    let mix = |x: f32, y: f32| {
        let value = lerp(x.max(0.0).sqrt(), y.max(0.0).sqrt(), t);
        value * value
    };
    straight([mix(pa[0], pb[0]), mix(pa[1], pb[1]), mix(pa[2], pb[2]), mix(pa[3], pb[3])])
}

/// `above` laid over `below` with `mode` and `opacity`.
pub fn pixel(mode: Blend, below: Pixel, above: Pixel, opacity: f32) -> Pixel {
    let o = clamp01(opacity);
    let out = match mode {
        Blend::Replace => [above[0] * o, above[1] * o, above[2] * o, above[3]],
        Blend::Cross => [lerp(below[0], above[0], o), lerp(below[1], above[1], o), lerp(below[2], above[2], o), lerp(below[3], above[3], o)],
        Blend::GammaCross => gamma_cross(below, above, o),
        Blend::AlphaOver | Blend::AlphaUnder => {
            let (lower, upper) = (premultiply(below), premultiply(above).map(|value| value * o));
            let (back, front) = if mode == Blend::AlphaOver { (lower, upper) } else { (upper, lower) };
            let keep = 1.0 - front[3];
            straight([front[0] + back[0] * keep, front[1] + back[1] * keep, front[2] + back[2] * keep, front[3] + back[3] * keep])
        }
        Blend::Multiply => {
            let scale = |a: f32, b: f32| a * (1.0 - o + o * b);
            [scale(below[0], above[0]), scale(below[1], above[1]), scale(below[2], above[2]), scale(below[3], above[3])]
        }
        Blend::Add | Blend::Subtract => {
            let sign = if mode == Blend::Add { 1.0 } else { -1.0 };
            let amount = o * above[3] * sign;
            [below[0] + amount * above[0], below[1] + amount * above[1], below[2] + amount * above[2], below[3]]
        }
        _ => {
            let fac = o * above[3];
            let result = colour(mode, [below[0], below[1], below[2]], [above[0], above[1], above[2]]);
            [lerp(below[0], result[0], fac), lerp(below[1], result[1], fac), lerp(below[2], result[2], fac), below[3]]
        }
    };
    out.map(clamp01)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for colour in [[0.8, 0.3, 0.1], [0.2, 0.6, 0.9], [0.5, 0.5, 0.5], [0.1, 0.9, 0.2]] {
            let back = hsv_to_rgb(rgb_to_hsv(colour));
            for (a, b) in colour.iter().zip(back) {
                assert!((a - b).abs() < 1e-5, "{colour:?} came back as {back:?}");
            }
        }
    }

    #[test]
    fn alpha_over_nothing_is_the_picture() {
        let out = pixel(Blend::AlphaOver, [0.0; 4], [0.2, 0.6, 0.9, 0.5], 1.0);
        assert!((out[0] - 0.2).abs() < 1e-6 && (out[3] - 0.5).abs() < 1e-6);
        // Colour modes over nothing leave nothing.
        assert_eq!(pixel(Blend::Screen, [0.0; 4], [0.2, 0.6, 0.9, 1.0], 1.0)[3], 0.0);
    }
}
