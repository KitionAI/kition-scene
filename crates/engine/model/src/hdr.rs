//! Radiance RGBE (`.hdr`) images: high dynamic range pictures, as world images come. Reads the
//! text header, the `-Y height +X width` resolution line (rows top to bottom; other orders are
//! flipped into it), and scanlines that are flat or run-length encoded per channel. Writes flat
//! files (for tests and exports).

/// The largest side read.
const MAX_SIDE: usize = 16384;
/// The most pixels read (64 M).
const MAX_PIXELS: usize = 64 * 1024 * 1024;

/// A decoded image: width, height, and linear RGB per pixel, top row first.
pub type Decoded = (usize, usize, Vec<[f32; 3]>);

/// Decodes an RGBE file; `None` when it is not one or is damaged.
pub fn decode(bytes: &[u8]) -> Option<Decoded> {
    if !(bytes.starts_with(b"#?RADIANCE") || bytes.starts_with(b"#?RGBE")) {
        return None;
    }
    // The header ends at an empty line; the resolution line follows.
    let mut at = 0;
    let mut format_ok = true;
    loop {
        let end = at + bytes.get(at..)?.iter().position(|byte| *byte == b'\n')?;
        let line = bytes.get(at..end)?;
        at = end + 1;
        if line.is_empty() {
            break;
        }
        if let Some(format) = line.strip_prefix(b"FORMAT=") {
            format_ok = format == b"32-bit_rle_rgbe";
        }
        if at > 64 * 1024 {
            return None;
        }
    }
    if !format_ok {
        return None;
    }
    let end = at + bytes.get(at..)?.iter().position(|byte| *byte == b'\n')?;
    let resolution = std::str::from_utf8(bytes.get(at..end)?).ok()?;
    at = end + 1;
    let parts: Vec<&str> = resolution.split_whitespace().collect();
    let [first_axis, first, second_axis, second] = parts.as_slice() else { return None };
    let (first, second): (usize, usize) = (first.parse().ok()?, second.parse().ok()?);
    // Only row-major layouts (Y first): `-Y` is top to bottom, `+Y` bottom to top; `+X` left to right.
    let (height, width, flip_y, flip_x) = match (*first_axis, *second_axis) {
        ("-Y", "+X") => (first, second, false, false),
        ("+Y", "+X") => (first, second, true, false),
        ("-Y", "-X") => (first, second, false, true),
        ("+Y", "-X") => (first, second, true, true),
        _ => return None,
    };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || width.saturating_mul(height) > MAX_PIXELS {
        return None;
    }
    let data = bytes.get(at..)?;
    let mut reader = 0usize;
    let mut rows: Vec<Vec<[u8; 4]>> = Vec::with_capacity(height);
    let mut scanline = vec![[0u8; 4]; width];
    for _ in 0..height {
        let head = data.get(reader..reader + 4)?;
        let run_length =
            (8..32768).contains(&width) && head.first() == Some(&2) && head.get(1) == Some(&2) && head.get(2).is_some_and(|byte| byte & 0x80 == 0);
        if run_length {
            let stated = (usize::from(*head.get(2)?) << 8) | usize::from(*head.get(3)?);
            if stated != width {
                return None;
            }
            reader += 4;
            for channel in 0..4 {
                let mut x = 0;
                while x < width {
                    let count = usize::from(*data.get(reader)?);
                    reader += 1;
                    if count > 128 {
                        let count = count - 128;
                        let value = *data.get(reader)?;
                        reader += 1;
                        if x + count > width {
                            return None;
                        }
                        for pixel in scanline.get_mut(x..x + count)? {
                            if let Some(slot) = pixel.get_mut(channel) {
                                *slot = value;
                            }
                        }
                        x += count;
                    } else {
                        if count == 0 || x + count > width {
                            return None;
                        }
                        for offset in 0..count {
                            let value = *data.get(reader + offset)?;
                            if let Some(slot) = scanline.get_mut(x + offset).and_then(|pixel| pixel.get_mut(channel)) {
                                *slot = value;
                            }
                        }
                        reader += count;
                        x += count;
                    }
                }
            }
        } else {
            // Flat pixels.
            for pixel in &mut scanline {
                let raw = data.get(reader..reader + 4)?;
                *pixel = [*raw.first()?, *raw.get(1)?, *raw.get(2)?, *raw.get(3)?];
                reader += 4;
            }
        }
        let mut row = scanline.clone();
        if flip_x {
            row.reverse();
        }
        rows.push(row);
    }
    if flip_y {
        rows.reverse();
    }
    let pixels = rows.into_iter().flatten().map(rgbe_to_linear).collect();
    Some((width, height, pixels))
}

fn rgbe_to_linear([r, g, b, e]: [u8; 4]) -> [f32; 3] {
    if e == 0 {
        return [0.0; 3];
    }
    let scale = 2f32.powi(i32::from(e) - 136);
    [(f32::from(r) + 0.5) * scale, (f32::from(g) + 0.5) * scale, (f32::from(b) + 0.5) * scale]
}

fn linear_to_rgbe([r, g, b]: [f32; 3]) -> [u8; 4] {
    let peak = r.max(g).max(b);
    if !(peak.is_finite() && peak > 1e-32) {
        return [0; 4];
    }
    // peak = mantissa × 2^exponent with mantissa in [0.5, 1).
    let exponent = peak.log2().floor() as i32 + 1;
    let scale = 2f32.powi(8 - exponent);
    let byte = |value: f32| (value.max(0.0) * scale).min(255.0) as u8;
    [byte(r), byte(g), byte(b), (exponent + 128).clamp(0, 255) as u8]
}

/// Encodes linear RGB pixels (top row first) as a flat RGBE file.
pub fn encode(width: usize, height: usize, pixels: &[[f32; 3]]) -> Vec<u8> {
    let mut out = format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {height} +X {width}\n").into_bytes();
    for pixel in pixels.iter().take(width * height) {
        out.extend(linear_to_rgbe(*pixel));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_files_round_trip_within_rgbe_precision() {
        let pixels: Vec<[f32; 3]> = (0..12).map(|index| [index as f32 * 0.37, 100.0 / (index as f32 + 1.0), 0.001 * index as f32]).collect();
        let bytes = encode(4, 3, &pixels);
        let (width, height, back) = decode(&bytes).unwrap();
        assert_eq!((width, height), (4, 3));
        for (a, b) in pixels.iter().zip(&back) {
            for channel in 0..3 {
                let tolerance = a[0].max(a[1]).max(a[2]) / 128.0 + 1e-6;
                assert!((a[channel] - b[channel]).abs() <= tolerance, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn run_length_scanlines_decode() {
        // Width 8, one row: R runs of 8 × 128, G literal 0..8, B run 8 × 64, E run 8 × 129.
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 8\n".to_vec();
        bytes.extend([2, 2, 0, 8]);
        bytes.extend([128 + 8, 128]);
        bytes.push(8);
        bytes.extend(0..8u8);
        bytes.extend([128 + 8, 64]);
        bytes.extend([128 + 8, 129]);
        let (width, height, pixels) = decode(&bytes).unwrap();
        assert_eq!((width, height), (8, 1));
        // Exponent 129: scale 2^(129-136) = 1/128.
        assert!((pixels[0][0] - 128.5 / 128.0).abs() < 1e-6);
        assert!((pixels[3][1] - 3.5 / 128.0).abs() < 1e-6);
        assert!((pixels[7][2] - 64.5 / 128.0).abs() < 1e-6);
    }

    #[test]
    fn damaged_files_are_refused_without_panics() {
        let bytes = encode(9, 9, &vec![[1.0, 2.0, 3.0]; 81]);
        for cut in 0..bytes.len() {
            let _ = decode(&bytes[..cut]);
        }
        assert!(decode(b"#?RADIANCE\nFORMAT=32-bit_rle_xyze\n\n-Y 1 +X 1\n\0\0\0\0").is_none());
        assert!(decode(b"#?RADIANCE\n\n+X 1 -Y 1\n\0\0\0\0").is_none());
        assert!(decode(b"#?RADIANCE\n\n-Y 99999 +X 99999\n").is_none());
    }
}
