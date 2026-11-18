//! Text strips: the words set in the engine's font (the same glyph outlines text objects use),
//! filled with 4 × 4 samples a pixel, on an optional box and over an optional shadow.

use kition_scene_model::sequencer::{Anchor, StripText};
use kition_scene_model::text::{Text, TextAlign};

use crate::Picture;
use crate::blend::{self, Pixel};

/// Samples a side of each pixel.
const SAMPLES: usize = 4;

/// The letters' triangles in pixels (y up), and their box.
fn triangles(strip: &StripText) -> (Vec<[[f32; 2]; 3]>, [f32; 4]) {
    let text = Text {
        body: strip.text.clone(),
        size: strip.size,
        align: match strip.align {
            Anchor::Start => TextAlign::Left,
            Anchor::Center => TextAlign::Center,
            Anchor::End => TextAlign::Right,
        },
        resolution: 6,
        ..Text::default()
    };
    let mesh = text.to_mesh();
    let mut out = Vec::new();
    let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for ([a, b, c], _) in mesh.triangles() {
        let point = |index: u32| mesh.positions.get(index as usize).map(|position| [position.x, position.y]);
        let (Some(a), Some(b), Some(c)) = (point(a), point(b), point(c)) else { continue };
        for [x, y] in [a, b, c] {
            bounds = [bounds[0].min(x), bounds[1].min(y), bounds[2].max(x), bounds[3].max(y)];
        }
        out.push([a, b, c]);
    }
    (out, bounds)
}

/// Sample coverage of `triangles` moved by `shift`, per pixel of a `width` × `height` frame.
fn coverage(triangles: &[[[f32; 2]; 3]], shift: [f32; 2], width: usize, height: usize) -> Vec<u16> {
    let mut masks = vec![0u16; width * height];
    for triangle in triangles {
        let [a, b, c] = triangle.map(|[x, y]| [x + shift[0], y + shift[1]]);
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
        let max_x = (a[0].max(b[0]).max(c[0]).ceil().max(0.0) as usize).min(width);
        let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
        let max_y = (a[1].max(b[1]).max(c[1]).ceil().max(0.0) as usize).min(height);
        let edge = |p: [f32; 2], q: [f32; 2], x: f32, y: f32| ((q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0])) * area.signum();
        for y in min_y..max_y {
            for x in min_x..max_x {
                let mut mask = 0u16;
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let (px, py) = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32, y as f32 + (sy as f32 + 0.5) / SAMPLES as f32);
                        if edge(a, b, px, py) >= 0.0 && edge(b, c, px, py) >= 0.0 && edge(c, a, px, py) >= 0.0 {
                            mask |= 1 << (sy * SAMPLES + sx);
                        }
                    }
                }
                if let Some(slot) = masks.get_mut((height - 1 - y) * width + x) {
                    *slot |= mask;
                }
            }
        }
    }
    masks
}

fn lay(picture: &mut Picture, masks: &[u16], colour: [f32; 4]) {
    for (pixel, mask) in picture.pixels.iter_mut().zip(masks) {
        if *mask == 0 {
            continue;
        }
        let share = mask.count_ones() as f32 / (SAMPLES * SAMPLES) as f32;
        let above: Pixel = [colour[0], colour[1], colour[2], colour[3] * share];
        *pixel = blend::pixel(kition_scene_model::sequencer::Blend::AlphaOver, *pixel, above, 1.0);
    }
}

/// The text strip's picture for a `width` × `height` frame.
pub fn draw(strip: &StripText, width: usize, height: usize) -> Picture {
    let mut picture = Picture::empty(width, height);
    if strip.text.trim().is_empty() {
        return picture;
    }
    let (triangles, [x0, y0, x1, y1]) = triangles(strip);
    if triangles.is_empty() {
        return picture;
    }
    let place = |anchor: Anchor, location: f32, low: f32, high: f32| match anchor {
        Anchor::Start => location - low,
        Anchor::Center => location - (low + high) / 2.0,
        Anchor::End => location - high,
    };
    let shift = [place(strip.anchor[0], strip.location[0] * width as f32, x0, x1), place(strip.anchor[1], strip.location[1] * height as f32, y0, y1)];
    if strip.boxed {
        let margin = strip.box_margin.max(0.0);
        let (left, right) = (x0 + shift[0] - margin, x1 + shift[0] + margin);
        let (bottom, top) = (y0 + shift[1] - margin, y1 + shift[1] + margin);
        let rect = [[[left, bottom], [right, bottom], [right, top]], [[left, bottom], [right, top], [left, top]]];
        lay(&mut picture, &coverage(&rect, [0.0, 0.0], width, height), strip.box_color);
    }
    if strip.shadow {
        let offset = [shift[0] + strip.shadow_offset[0], shift[1] - strip.shadow_offset[1]];
        lay(&mut picture, &coverage(&triangles, offset, width, height), strip.shadow_color);
    }
    lay(&mut picture, &coverage(&triangles, shift, width, height), strip.color);
    picture
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_sits_centred_on_its_location() {
        let strip = StripText { text: "Hi".into(), size: 40.0, ..StripText::default() };
        let picture = draw(&strip, 200, 100);
        let covered: Vec<(usize, usize)> = (0..100)
            .flat_map(|row| (0..200).map(move |x| (x, row)))
            .filter(|(x, row)| picture.get(*x, *row).is_some_and(|pixel| pixel[3] > 0.5))
            .collect();
        assert!(!covered.is_empty());
        let mean_x = covered.iter().map(|(x, _)| *x as f32).sum::<f32>() / covered.len() as f32;
        let mean_row = covered.iter().map(|(_, row)| *row as f32).sum::<f32>() / covered.len() as f32;
        assert!((mean_x - 100.0).abs() < 8.0 && (mean_row - 50.0).abs() < 8.0, "{mean_x} {mean_row}");
    }
}
