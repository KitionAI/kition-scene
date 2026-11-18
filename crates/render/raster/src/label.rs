//! A short label burned into a picture (a preview sheet's view names and frame times): light text
//! on a dark box in the picture's bottom-left corner, sized to the picture, in the bundled font
//! (Inter, the one text objects are set in), so it draws the same everywhere.

use glam::Vec2;
use kition_scene_model::text::Text;

/// The box behind the text: black at this opacity.
const BOX_ALPHA: f32 = 0.72;
/// Samples per pixel side when filling the letters.
const SAMPLES: usize = 4;

/// Burns `text` into the bottom-left corner of `rgba`, an 8-bit sRGB `width` × `height` picture.
/// A picture too small for legible text (under 24 pixels on a side) is left as it is.
pub fn burn_label(rgba: &mut [u8], width: u32, height: u32, text: &str) {
    let pixels = (width as usize).saturating_mul(height as usize);
    if width < 24 || height < 24 || text.trim().is_empty() || rgba.len() < pixels.saturating_mul(4) {
        return;
    }
    // The glyphs at size 1: the font's box (ascender to descender) is one unit high.
    let contours = Text { body: text.to_owned(), ..Text::default() }.contours();
    let mut points = contours.iter().flatten();
    let Some(first) = points.next().copied() else { return };
    let (low, high) = points.fold((first, first), |(low, high), point| (low.min(*point), high.max(*point)));
    let (w, h) = (width as f32, height as f32);
    let margin = (h * 0.04).max(2.0);
    let inner = (h * 0.03).max(2.0);
    let mut size = (h * 0.12).clamp(9.0, 48.0);
    let room = w - 2.0 * (margin + inner);
    let wide = (high.x - low.x) * size;
    if wide > room && wide > 0.0 {
        size *= room / wide;
    }
    let (tw, th) = ((high.x - low.x) * size, (high.y - low.y) * size);
    let (x1, y1) = ((margin + 2.0 * inner + tw).min(w), h - margin);
    let (x0, y0) = (margin, (y1 - th - 2.0 * inner).max(0.0));
    // The box, with partly covered edge pixels blended by how much of them it covers.
    for y in (y0.floor() as u32)..(y1.ceil() as u32).min(height) {
        for x in (x0.floor() as u32)..(x1.ceil() as u32).min(width) {
            let cover_x = (x1.min(x as f32 + 1.0) - x0.max(x as f32)).clamp(0.0, 1.0);
            let cover_y = (y1.min(y as f32 + 1.0) - y0.max(y as f32)).clamp(0.0, 1.0);
            paint(rgba, width, x, y, 0.0, cover_x * cover_y * BOX_ALPHA);
        }
    }
    // The letters: y up in the font, down in the picture.
    let (left, top) = ((x0 + inner).round(), (y0 + inner).round());
    let place = |point: Vec2| Vec2::new(left + (point.x - low.x) * size, top + (high.y - point.y) * size);
    let edges: Vec<(Vec2, Vec2)> = contours
        .iter()
        .flat_map(|contour| {
            let placed: Vec<Vec2> = contour.iter().map(|point| place(*point)).collect();
            let next = placed.iter().skip(1).chain(placed.first()).copied().collect::<Vec<_>>();
            placed.into_iter().zip(next)
        })
        .collect();
    let (px0, py0) = (left.max(0.0) as u32, top.max(0.0) as u32);
    let (px1, py1) = (((left + tw).ceil() as u32 + 1).min(width), ((top + th).ceil() as u32 + 1).min(height));
    let span = px1.saturating_sub(px0) as usize;
    let mut coverage = vec![0.0f32; span];
    let step = 1.0 / SAMPLES as f32;
    for y in py0..py1 {
        coverage.iter_mut().for_each(|cover| *cover = 0.0);
        for row in 0..SAMPLES {
            let sy = y as f32 + (row as f32 + 0.5) * step;
            // Where the edges cross this sample row, with their direction (non-zero winding).
            let mut crossings: Vec<(f32, i32)> = edges
                .iter()
                .filter(|(a, b)| (a.y <= sy) != (b.y <= sy))
                .map(|(a, b)| (a.x + (sy - a.y) / (b.y - a.y) * (b.x - a.x), if b.y > a.y { 1 } else { -1 }))
                .collect();
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for pair in crossings.windows(2) {
                let [(from, turn), (to, _)] = pair else { continue };
                winding += turn;
                if winding == 0 {
                    continue;
                }
                // The sample columns from `from` to `to`.
                let first = ((from - px0 as f32) / step - 0.5).ceil().max(0.0) as usize;
                let last = ((to - px0 as f32) / step - 0.5).ceil().max(0.0) as usize;
                for column in first..last.min(span * SAMPLES) {
                    if let Some(cover) = coverage.get_mut(column / SAMPLES) {
                        *cover += step * step;
                    }
                }
            }
        }
        for (offset, cover) in coverage.iter().enumerate() {
            if *cover > 0.0 {
                paint(rgba, width, px0 + offset as u32, y, 255.0, cover.min(1.0));
            }
        }
    }
}

/// Blends grey `level` (0 to 255) at `alpha` over pixel (`x`, `y`).
fn paint(rgba: &mut [u8], width: u32, x: u32, y: u32, level: f32, alpha: f32) {
    let at = (y as usize * width as usize + x as usize) * 4;
    if let Some(pixel) = rgba.get_mut(at..at + 3) {
        for channel in pixel {
            *channel = (f32::from(*channel) * (1.0 - alpha) + level * alpha).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burns_light_text_on_a_dark_box_in_the_bottom_left_corner() {
        let (w, h) = (320u32, 180u32);
        let red: Vec<u8> = [255u8, 0, 0, 255].repeat((w * h) as usize);
        let mut img = red.clone();
        burn_label(&mut img, w, h, "1.5 s, frame 37");
        let px = |x: u32, y: u32| -> [u8; 4] {
            let at = ((y * w + x) * 4) as usize;
            [img[at], img[at + 1], img[at + 2], img[at + 3]]
        };
        assert_eq!(px(300, 20), [255, 0, 0, 255], "the rest of the picture is untouched");
        let corner: Vec<[u8; 4]> = (0..200).flat_map(|x| (120..175).map(move |y| (x, y))).map(|(x, y)| px(x, y)).collect();
        assert!(corner.iter().any(|p| p[0] < 100 && p[1] < 10), "a dark box");
        assert!(corner.iter().filter(|p| p[..3].iter().all(|c| *c > 240)).count() > 30, "white text");
        let mut again = red.clone();
        burn_label(&mut again, w, h, "1.5 s, frame 37");
        assert_eq!(again, img, "the same label burns the same pixels");
        let mut small = [0u8, 0, 255, 255].repeat(400);
        burn_label(&mut small, 20, 20, "1 s");
        assert_eq!(small, [0u8, 0, 255, 255].repeat(400), "too small for text");
    }
}
