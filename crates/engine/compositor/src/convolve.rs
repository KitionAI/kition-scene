//! Convolution with a picture as the kernel (the Glare node's Kernel glare): the kernel is cut
//! to what it covers, each colour channel normalised to sum to one, and laid on every pixel with
//! its origin there, the picture clear beyond its edges. Small kernels are applied directly,
//! large ones through the fast Fourier transform.

use crate::{Image, Rgba};

/// A convolution kernel: per colour channel, `width × height` weights from the top left, and
/// the weight that lies on the pixel itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Kernel {
    pub width: usize,
    pub height: usize,
    pub origin: (usize, usize),
    pub weights: [Vec<f32>; 3],
}

impl Kernel {
    /// The kernel that changes nothing.
    pub fn identity() -> Self {
        Self { width: 1, height: 1, origin: (0, 0), weights: [vec![1.0], vec![1.0], vec![1.0]] }
    }

    /// A kernel from a picture of the run's size whose origin is the pixel at its middle (as
    /// the reference places a kernel image at its own size): `channels` picks one weight per
    /// colour channel from each pixel. The picture is cut to the pixels with any weight.
    pub fn from_picture(picture: &Image, channels: impl Fn(Rgba) -> [f32; 3]) -> Self {
        let (width, height) = (picture.width, picture.height);
        let origin = (width.saturating_sub(1) / 2, height.saturating_sub(1) / 2);
        let values: Vec<[f32; 3]> =
            picture.pixels.iter().map(|pixel| channels(*pixel).map(|value| if value.is_finite() { value } else { 0.0 })).collect();
        let used = |index: usize| values.get(index).is_some_and(|value| value.iter().any(|weight| *weight != 0.0));
        let (mut left, mut top, mut right, mut bottom) = (origin.0, origin.1, origin.0, origin.1);
        let mut any = false;
        for y in 0..height {
            for x in 0..width {
                if used(y * width + x) {
                    any = true;
                    left = left.min(x);
                    right = right.max(x);
                    top = top.min(y);
                    bottom = bottom.max(y);
                }
            }
        }
        if !any {
            return Self { width: 1, height: 1, origin: (0, 0), weights: [vec![0.0], vec![0.0], vec![0.0]] };
        }
        let (kernel_width, kernel_height) = (right - left + 1, bottom - top + 1);
        let mut weights: [Vec<f32>; 3] = std::array::from_fn(|_| Vec::with_capacity(kernel_width * kernel_height));
        for y in top..=bottom {
            for x in left..=right {
                let value = values.get(y * width + x).copied().unwrap_or([0.0; 3]);
                for (channel, weight) in weights.iter_mut().zip(value) {
                    channel.push(weight);
                }
            }
        }
        for channel in &mut weights {
            let total: f32 = channel.iter().sum();
            for weight in channel.iter_mut() {
                *weight = if total.abs() > 1e-12 { *weight / total } else { 0.0 };
            }
        }
        Self { width: kernel_width, height: kernel_height, origin: (origin.0 - left, origin.1 - top), weights }
    }
}

/// Kernels with at most this many weights are applied directly.
const DIRECT: usize = 32 * 32;

/// `image`'s colour convolved with `kernel` (alpha 1).
pub fn convolve(image: &Image, kernel: &Kernel) -> Image {
    if kernel.width * kernel.height <= DIRECT {
        return direct(image, kernel);
    }
    let mut out = Image::filled(image.width, image.height, [0.0, 0.0, 0.0, 1.0]);
    let (width, height) = ((image.width + kernel.width).next_power_of_two(), (image.height + kernel.height).next_power_of_two());
    for (channel, weights) in kernel.weights.iter().enumerate() {
        let mut picture = vec![Complex::ZERO; width * height];
        for y in 0..image.height {
            for x in 0..image.width {
                if let (Some(slot), Some(pixel)) = (picture.get_mut(y * width + x), image.pixels.get(y * image.width + x)) {
                    *slot = Complex::real(pixel.get(channel).copied().unwrap_or(0.0));
                }
            }
        }
        // The kernel turned about its origin so the product is a convolution lined up on it.
        let mut spread = vec![Complex::ZERO; width * height];
        for y in 0..kernel.height {
            for x in 0..kernel.width {
                let (dx, dy) = (x as isize - kernel.origin.0 as isize, y as isize - kernel.origin.1 as isize);
                let (tx, ty) = (dx.rem_euclid(width as isize) as usize, dy.rem_euclid(height as isize) as usize);
                if let (Some(slot), Some(weight)) = (spread.get_mut(ty * width + tx), weights.get(y * kernel.width + x)) {
                    *slot = Complex::real(*weight);
                }
            }
        }
        fft_2d(&mut picture, width, height, false);
        fft_2d(&mut spread, width, height, false);
        for (value, other) in picture.iter_mut().zip(&spread) {
            *value = value.mul(*other);
        }
        fft_2d(&mut picture, width, height, true);
        for y in 0..image.height {
            for x in 0..image.width {
                if let (Some(pixel), Some(value)) = (out.pixels.get_mut(y * image.width + x), picture.get(y * width + x))
                    && let Some(slot) = pixel.get_mut(channel)
                {
                    *slot = value.re;
                }
            }
        }
    }
    out
}

fn direct(image: &Image, kernel: &Kernel) -> Image {
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height as isize {
        for x in 0..image.width as isize {
            let mut sum = [0.0f32, 0.0, 0.0, 1.0];
            for ky in 0..kernel.height {
                for kx in 0..kernel.width {
                    let (sx, sy) = (x - (kx as isize - kernel.origin.0 as isize), y - (ky as isize - kernel.origin.1 as isize));
                    let Some(pixel) = image.get(sx, sy) else { continue };
                    for (channel, weights) in kernel.weights.iter().enumerate() {
                        let weight = weights.get(ky * kernel.width + kx).copied().unwrap_or(0.0);
                        if let Some(slot) = sum.get_mut(channel) {
                            *slot += weight * pixel.get(channel).copied().unwrap_or(0.0);
                        }
                    }
                }
            }
            pixels.push(sum);
        }
    }
    Image { width: image.width, height: image.height, pixels }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    const ZERO: Self = Self { re: 0.0, im: 0.0 };

    fn real(re: f32) -> Self {
        Self { re, im: 0.0 }
    }

    fn mul(self, other: Self) -> Self {
        Self { re: self.re * other.re - self.im * other.im, im: self.re * other.im + self.im * other.re }
    }
}

/// An in-place radix-2 FFT of `values` (a power of two long), inverse (and scaled) when asked.
fn fft(values: &mut [Complex], inverse: bool) {
    let count = values.len();
    if count <= 1 {
        return;
    }
    let mut j = 0usize;
    for i in 1..count {
        let mut bit = count >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            values.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= count {
        let angle = std::f64::consts::TAU / length as f64 * if inverse { 1.0 } else { -1.0 };
        for start in (0..count).step_by(length) {
            for k in 0..length / 2 {
                let (sin, cos) = (angle * k as f64).sin_cos();
                let twiddle = Complex { re: cos as f32, im: sin as f32 };
                let (Some(a), Some(b)) = (values.get(start + k).copied(), values.get(start + k + length / 2).copied()) else { continue };
                let b = b.mul(twiddle);
                if let Some(slot) = values.get_mut(start + k) {
                    *slot = Complex { re: a.re + b.re, im: a.im + b.im };
                }
                if let Some(slot) = values.get_mut(start + k + length / 2) {
                    *slot = Complex { re: a.re - b.re, im: a.im - b.im };
                }
            }
        }
        length <<= 1;
    }
    if inverse {
        let scale = 1.0 / count as f32;
        for value in values.iter_mut() {
            value.re *= scale;
            value.im *= scale;
        }
    }
}

fn fft_2d(values: &mut [Complex], width: usize, height: usize, inverse: bool) {
    for row in values.chunks_mut(width) {
        fft(row, inverse);
    }
    let mut column = vec![Complex::ZERO; height];
    for x in 0..width {
        for (y, slot) in column.iter_mut().enumerate() {
            *slot = values.get(y * width + x).copied().unwrap_or(Complex::ZERO);
        }
        fft(&mut column, inverse);
        for (y, value) in column.iter().enumerate() {
            if let Some(slot) = values.get_mut(y * width + x) {
                *slot = *value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Fourier path gives what the direct one does.
    #[test]
    fn both_ways_agree() {
        let image = Image {
            width: 40,
            height: 36,
            pixels: (0..40 * 36).map(|index| [((index * 7) % 13) as f32 / 13.0, ((index * 3) % 5) as f32, 0.5, 1.0]).collect(),
        };
        let weights: Vec<f32> = (0..33 * 34).map(|index| ((index * 11) % 17) as f32).collect();
        let total: f32 = weights.iter().sum();
        let weights: Vec<f32> = weights.iter().map(|weight| weight / total).collect();
        let kernel = Kernel { width: 33, height: 34, origin: (16, 20), weights: [weights.clone(), weights.clone(), weights] };
        let (fast, slow) = (convolve(&image, &kernel), direct(&image, &kernel));
        for (a, b) in fast.pixels.iter().zip(&slow.pixels) {
            for (a, b) in a.iter().zip(b) {
                assert!((a - b).abs() < 1e-4, "{a} {b}");
            }
        }
    }
}
