//! A strip's picture into the frame: cropped, centred, flipped, scaled and turned about its
//! origin, moved; then its colours' saturation and strength scaled.

use kition_scene_model::sequencer::{Crop, Filter, StripTransform};

use crate::Picture;
use crate::blend::{Pixel, hsv_to_rgb, rgb_to_hsv};

/// Whether `transform` moves the picture by whole pixels only (no scale, no turn).
fn whole_pixels(transform: &StripTransform, offset: [f32; 2]) -> bool {
    transform.scale == [1.0, 1.0] && transform.rotation.rem_euclid(360.0) == 0.0 && offset.iter().all(|value| value.fract() == 0.0)
}

/// The source pixel of `source` at continuous point (`u`, `v`) (y up, pixel units), with
/// pixels outside it transparent.
fn sample(source: &Picture, crop: &Crop, u: f32, v: f32, bilinear: bool) -> Pixel {
    let (width, height) = (source.width as i64, source.height as i64);
    let at = |x: i64, y: i64| -> Pixel {
        let inside =
            x >= i64::from(crop.left) && x < width - i64::from(crop.right) && y >= i64::from(crop.bottom) && y < height - i64::from(crop.top);
        if !inside {
            return [0.0; 4];
        }
        let (Ok(x), Ok(row)) = (usize::try_from(x), usize::try_from(height - 1 - y)) else { return [0.0; 4] };
        source.get(x, row).unwrap_or([0.0; 4])
    };
    if !bilinear {
        return at(u.floor() as i64, v.floor() as i64);
    }
    // Bilinear on premultiplied values, so transparent neighbours do not darken the edge.
    let (fx, fy) = (u - 0.5, v - 0.5);
    let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
    let mut sum = [0.0f32; 4];
    for (dx, dy, weight) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
        let pixel = at(x0 + dx, y0 + dy);
        sum[0] += pixel[0] * pixel[3] * weight;
        sum[1] += pixel[1] * pixel[3] * weight;
        sum[2] += pixel[2] * pixel[3] * weight;
        sum[3] += pixel[3] * weight;
    }
    if sum[3] <= 1e-6 {
        return [0.0; 4];
    }
    [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3], sum[3]]
}

/// `source` placed in a `width` × `height` frame by `crop` and `transform`.
pub fn place(source: &Picture, crop: &Crop, transform: &StripTransform, width: usize, height: usize) -> Picture {
    let identity = crop.is_default() && transform.is_default() && (source.width, source.height) == (width, height);
    if identity {
        return source.clone();
    }
    let (sw, sh) = (source.width as f32, source.height as f32);
    // The picture's bottom-left corner sits so it is centred in the frame.
    let corner = [((width as f32 - sw) / 2.0).floor(), ((height as f32 - sh) / 2.0).floor()];
    let origin = [transform.origin[0] * sw, transform.origin[1] * sh];
    let bilinear = match transform.filter {
        Filter::Nearest => false,
        Filter::Bilinear => true,
        Filter::Auto => !whole_pixels(transform, transform.offset),
    };
    let radians = transform.rotation.to_radians();
    let (sin, cos) = radians.sin_cos();
    let scale = [
        if transform.scale[0].abs() > 1e-6 { transform.scale[0] } else { 1e-6 },
        if transform.scale[1].abs() > 1e-6 { transform.scale[1] } else { 1e-6 },
    ];
    let mut out = Picture::empty(width, height);
    for row in 0..height {
        for x in 0..width {
            // The frame point, then back through the move, the turn, and the scale.
            let (px, py) = (x as f32 + 0.5, (height - 1 - row) as f32 + 0.5);
            let (rx, ry) = (px - corner[0] - transform.offset[0] - origin[0], py - corner[1] - transform.offset[1] - origin[1]);
            let (ux, uy) = (cos * rx + sin * ry, -sin * rx + cos * ry);
            let (mut u, mut v) = (ux / scale[0] + origin[0], uy / scale[1] + origin[1]);
            if transform.flip_x {
                u = sw - u;
            }
            if transform.flip_y {
                v = sh - v;
            }
            out.set(x, row, sample(source, crop, u, v, bilinear));
        }
    }
    out
}

/// Scales the colours' saturation (as HSV saturation) and strength, as the reference's strip
/// colour settings do.
pub fn adjust(picture: &mut Picture, saturation: f32, multiply: f32) {
    if saturation == 1.0 && multiply == 1.0 {
        return;
    }
    for pixel in &mut picture.pixels {
        let mut rgb = [pixel[0], pixel[1], pixel[2]];
        if saturation != 1.0 {
            let [hue, s, value] = rgb_to_hsv(rgb);
            rgb = hsv_to_rgb([hue, s * saturation, value]);
        }
        for (channel, value) in pixel.iter_mut().zip(rgb) {
            *channel = (value * multiply).clamp(0.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Picture {
        // 8 × 4: x + 1 in red, y + 1 (from the bottom) in green.
        let mut picture = Picture::empty(8, 4);
        for row in 0..4 {
            for x in 0..8 {
                picture.set(x, row, [(x + 1) as f32 / 8.0, (4 - row) as f32 / 4.0, 0.25, 1.0]);
            }
        }
        picture
    }

    fn label(picture: &Picture, x: usize, y_up: usize) -> Option<(u32, u32)> {
        let pixel = picture.get(x, picture.height - 1 - y_up)?;
        (pixel[3] > 0.5).then(|| ((pixel[0] * 8.0).round() as u32, (pixel[1] * 4.0).round() as u32))
    }

    #[test]
    fn a_picture_sits_in_the_middle_and_moves_up_and_right() {
        let placed = place(&grid(), &Crop::default(), &StripTransform::default(), 16, 8);
        assert_eq!(label(&placed, 4, 2), Some((1, 1)));
        assert_eq!(label(&placed, 3, 2), None);
        let moved = StripTransform { offset: [3.0, 1.0], ..StripTransform::default() };
        let placed = place(&grid(), &Crop::default(), &moved, 16, 8);
        assert_eq!(label(&placed, 7, 3), Some((1, 1)));
    }

    #[test]
    fn crops_turns_and_scales_match_the_reference() {
        let crop = Crop { left: 1, right: 2, bottom: 1, top: 0 };
        let placed = place(&grid(), &crop, &StripTransform::default(), 16, 8);
        assert_eq!(label(&placed, 4, 2), None);
        assert_eq!(label(&placed, 5, 3), Some((2, 2)));
        assert_eq!(label(&placed, 10, 3), None);
        let turned = StripTransform { rotation: 90.0, filter: Filter::Nearest, ..StripTransform::default() };
        let placed = place(&grid(), &Crop::default(), &turned, 16, 8);
        assert_eq!(label(&placed, 6, 7), Some((8, 4)));
        assert_eq!(label(&placed, 9, 0), Some((1, 1)));
        let shrunk = StripTransform { origin: [0.0, 0.0], scale: [0.5, 0.5], filter: Filter::Nearest, ..StripTransform::default() };
        let placed = place(&grid(), &Crop::default(), &shrunk, 16, 8);
        assert_eq!(label(&placed, 4, 2), Some((2, 2)));
        assert_eq!(label(&placed, 7, 3), Some((8, 4)));
        assert_eq!(label(&placed, 8, 2), None);
    }

    #[test]
    fn saturation_and_multiply_follow_the_reference() {
        let mut picture = Picture { width: 1, height: 1, pixels: vec![[0.8, 0.3, 0.1, 1.0]] };
        adjust(&mut picture, 0.5, 1.0);
        let [r, g, b, _] = picture.pixels[0];
        assert!((r - 0.8).abs() < 1e-5 && (g - 0.55).abs() < 1e-3 && (b - 0.45).abs() < 1e-3);
        let mut picture = Picture { width: 1, height: 1, pixels: vec![[0.8, 0.3, 0.1, 1.0]] };
        adjust(&mut picture, 1.5, 2.0);
        let [r, g, b, _] = picture.pixels[0];
        assert!((r - 1.0).abs() < 1e-5 && (g - 0.1).abs() < 1e-3 && b == 0.0, "{r} {g} {b}");
    }
}
