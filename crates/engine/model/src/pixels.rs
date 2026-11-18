//! PNG and JPEG pictures decoded to straight RGBA8 (top row first), for whatever reads an image's
//! pixels: textures on surfaces and the Displace modifier.

use crate::scene::{Image, ImageFormat};

/// The largest image side decoded.
const MAX_SIDE: usize = 8192;
/// The most pixels decoded (64 M).
const MAX_PIXELS: usize = 64 * 1024 * 1024;

/// Decodes a PNG or JPEG image to straight RGBA8 (as stored, so sRGB for colour pictures) with
/// its size; `None` for other formats and damaged files.
pub fn decode_rgba8(image: &Image) -> Option<(usize, usize, Vec<u8>)> {
    match image.format {
        ImageFormat::Png => decode_png(&image.bytes),
        ImageFormat::Jpeg => decode_jpeg(&image.bytes),
        ImageFormat::Hdr => None,
    }
}

fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = (reader.info().width as usize, reader.info().height as usize);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || width.saturating_mul(height) > MAX_PIXELS {
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let data = buffer.get(..frame.buffer_size())?;
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    Some((width, height, to_rgba(data, channels, width * height)?))
}

fn decode_jpeg(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = zune_jpeg::JpegDecoder::new(bytes);
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    if width == 0 || height == 0 || width.saturating_mul(height) > MAX_PIXELS {
        return None;
    }
    let data = decoder.decode().ok()?;
    let channels = data.len() / (width * height);
    Some((width, height, to_rgba(&data, channels, width * height)?))
}

fn to_rgba(data: &[u8], channels: usize, pixels: usize) -> Option<Vec<u8>> {
    if !(1..=4).contains(&channels) || data.len() < pixels * channels {
        return None;
    }
    let mut out = Vec::with_capacity(pixels * 4);
    for pixel in data.chunks_exact(channels).take(pixels) {
        let at = |index: usize| pixel.get(index).copied().unwrap_or(255);
        out.extend(match channels {
            1 => [at(0), at(0), at(0), 255],
            2 => [at(0), at(0), at(0), at(1)],
            3 => [at(0), at(1), at(2), 255],
            _ => [at(0), at(1), at(2), at(3)],
        });
    }
    Some(out)
}
