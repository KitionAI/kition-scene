//! Transitions between two pictures of the same size at a factor from 0 (all the first) to 1
//! (all the second). Cross and Gamma Cross mix them as the blend modes of those names do; a wipe
//! shows the second inside a shape that grows with the factor:
//!
//! - Single: an edge sweeps the frame from the side `angle` points to (0° is the top, turning
//!   clockwise; In comes from the other side).
//! - Double: a band through the middle widens (In: the first's band narrows).
//! - Iris: a circle grows from the middle to the corners (In: shrinks, showing the second
//!   outside it).
//! - Clock: a hand sweeps clockwise from twelve o'clock (In: from three o'clock).
//!
//! `blur` (0 to 1) makes the edge a ramp `blur × the frame's shorter side` wide; the ramp starts
//! wholly outside the frame at 0 and ends wholly past it at 1.

use std::f32::consts::{PI, TAU};

use kition_scene_model::sequencer::{Blend, Transition, WipeDirection, WipeShape};

use crate::Picture;
use crate::blend::{self, Pixel};

fn lerp_pixel(a: Pixel, b: Pixel, t: f32) -> Pixel {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// How much of the second picture shows at pixel centre (`x`, `y`), y up from the bottom.
pub fn wipe_weight(shape: WipeShape, direction: WipeDirection, angle: f32, blur: f32, factor: f32, width: f32, height: f32, x: f32, y: f32) -> f32 {
    let f = factor.clamp(0.0, 1.0);
    let ramp = (blur.clamp(0.0, 1.0) * width.min(height)).max(1e-3);
    let (dx, dy) = (x - width / 2.0, y - height / 2.0);
    // The signed distance into the second picture's side of the edge, in pixels, and how far
    // the edge travels from the factor's 0 to its 1; `inside(edge)` ramps across the edge.
    let soft = |distance: f32| (distance / ramp + 0.5).clamp(0.0, 1.0);
    let into = |distance: f32, travel: f32, t: f32| soft(distance - (travel + ramp) * (1.0 - t) + ramp / 2.0);
    match shape {
        WipeShape::Single => {
            let radians = angle.to_radians();
            let (nx, ny) = (radians.sin(), radians.cos());
            let extent = (nx.abs() * width + ny.abs() * height) / 2.0;
            let along = dx * nx + dy * ny;
            let along = if direction == WipeDirection::Out { along } else { -along };
            // From the far edge (`extent`) back across the whole frame.
            into(along + extent, 2.0 * extent, f)
        }
        WipeShape::Double => {
            let radians = angle.to_radians();
            let (nx, ny) = (radians.sin(), radians.cos());
            let extent = (nx.abs() * width + ny.abs() * height) / 2.0;
            let across = (dx * nx + dy * ny).abs();
            match direction {
                WipeDirection::Out => into(extent - across, extent, f),
                WipeDirection::In => 1.0 - into(extent - across, extent, 1.0 - f),
            }
        }
        WipeShape::Iris => {
            let reach = (width * width + height * height).sqrt() / 2.0;
            let inward = reach - (dx * dx + dy * dy).sqrt();
            match direction {
                WipeDirection::Out => into(inward, reach, f),
                WipeDirection::In => 1.0 - into(inward, reach, 1.0 - f),
            }
        }
        WipeShape::Clock => {
            let start = if direction == WipeDirection::Out { 0.0 } else { PI / 2.0 };
            // Clockwise from twelve o'clock, 0 to 2π.
            let turned = (dx.atan2(dy) - start).rem_euclid(TAU);
            let radius = (dx * dx + dy * dy).sqrt().max(1.0);
            // The edge's ramp, as an angle at this distance from the middle.
            let ramp_angle = (ramp / radius).min(TAU);
            let sweep = f * (TAU + ramp_angle) - ramp_angle / 2.0;
            ((sweep - turned) / ramp_angle + 0.5).clamp(0.0, 1.0)
        }
    }
}

/// `transition` from `a` to `b` at `factor`.
pub fn apply(transition: &Transition, a: &Picture, b: &Picture, factor: f32) -> Picture {
    let (width, height) = (a.width.min(b.width), a.height.min(b.height));
    let mut out = Picture::empty(width, height);
    for row in 0..height {
        for x in 0..width {
            let (Some(pa), Some(pb)) = (a.get(x, row), b.get(x, row)) else { continue };
            let value = match transition {
                Transition::Cross => blend::pixel(Blend::Cross, pa, pb, factor),
                Transition::GammaCross => blend::pixel(Blend::GammaCross, pa, pb, factor),
                Transition::Wipe { shape, direction, angle, blur } => {
                    let y = (height - 1 - row) as f32 + 0.5;
                    let weight = wipe_weight(*shape, *direction, *angle, *blur, factor, width as f32, height as f32, x as f32 + 0.5, y);
                    lerp_pixel(pa, pb, weight)
                }
            };
            out.set(x, row, value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(shape: WipeShape, direction: WipeDirection, factor: f32) -> f32 {
        let (width, height) = (100, 100);
        let mut sum = 0.0;
        for y in 0..height {
            for x in 0..width {
                sum += wipe_weight(shape, direction, 0.0, 0.0, factor, width as f32, height as f32, x as f32 + 0.5, y as f32 + 0.5);
            }
        }
        sum / (width * height) as f32
    }

    #[test]
    fn wipes_uncover_as_much_as_the_factor_says() {
        for shape in [WipeShape::Single, WipeShape::Double, WipeShape::Clock] {
            for direction in [WipeDirection::Out, WipeDirection::In] {
                assert!(share(shape, direction, 0.0) < 0.01, "{shape:?} {direction:?} starts with the first");
                assert!(share(shape, direction, 1.0) > 0.99, "{shape:?} {direction:?} ends with the second");
                assert!((share(shape, direction, 0.5) - 0.5).abs() < 0.03, "{shape:?} {direction:?} at the middle");
            }
        }
        // An iris's circle reaches the corners at the end: a quarter of the way it covers π/16.
        assert!((share(WipeShape::Iris, WipeDirection::Out, 0.5) - PI / 8.0).abs() < 0.02);
    }
}
