//! Per-pixel colour nodes: exposure, brightness and contrast, invert, hue/saturation/value,
//! colour balance, RGB curves, mixing, and alpha over. They work on premultiplied values as they
//! come, as the reference does (its colour nodes do not divide by alpha first).

use serde_json::Value;

use crate::Rgba;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_rgb(a: Rgba, b: Rgba, t: f32) -> Rgba {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t), a[3]]
}

/// Scales the light by 2 to the power `stops`.
pub fn exposure(pixel: Rgba, stops: f32) -> Rgba {
    let scale = stops.exp2();
    [pixel[0] * scale, pixel[1] * scale, pixel[2] * scale, pixel[3]]
}

/// Brightness (an offset, in hundredths) and contrast (in percent, about middle grey).
pub fn bright_contrast(pixel: Rgba, bright: f32, contrast: f32) -> Rgba {
    let slope = if contrast > 0.0 { 1.0 / (1.0 - contrast / 100.0).max(1e-6) } else { 1.0 + contrast / 100.0 };
    let offset = bright / 100.0;
    let f = |value: f32| slope * (value + offset) + (1.0 - slope) / 2.0;
    [f(pixel[0]), f(pixel[1]), f(pixel[2]), pixel[3]]
}

/// Inverts the colour and, if asked, the alpha, by `fac`.
pub fn invert(pixel: Rgba, fac: f32, rgb: bool, alpha: bool) -> Rgba {
    let mut out = pixel;
    if rgb {
        for channel in out.iter_mut().take(3) {
            *channel = 1.0 - *channel;
        }
    }
    if alpha {
        out[3] = 1.0 - out[3];
    }
    [lerp(pixel[0], out[0], fac), lerp(pixel[1], out[1], fac), lerp(pixel[2], out[2], fac), lerp(pixel[3], out[3], fac)]
}

/// RGB to hue, saturation, and value (each 0 to 1 for colours in 0 to 1).
pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let saturation = if max != 0.0 { delta / max } else { 0.0 };
    let hue = if saturation == 0.0 || delta == 0.0 {
        0.0
    } else {
        let h = if r == max {
            (g - b) / delta
        } else if g == max {
            2.0 + (b - r) / delta
        } else {
            4.0 + (r - g) / delta
        };
        let h = h / 6.0;
        if h < 0.0 { h + 1.0 } else { h }
    };
    [hue, saturation, max]
}

/// Hue, saturation, and value to RGB.
pub fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let channel = |offset: f32| {
        let k = (offset + h * 6.0).rem_euclid(6.0);
        v - v * s * (k.min(4.0 - k)).clamp(0.0, 1.0)
    };
    [channel(5.0), channel(3.0), channel(1.0)]
}

/// Turns the hue (0.5 keeps it), scales saturation and value, by `fac`. Saturation may go past
/// one; channels that would go below zero stop at zero.
pub fn hue_saturation(pixel: Rgba, hue: f32, saturation: f32, value: f32, fac: f32) -> Rgba {
    let [h, s, v] = rgb_to_hsv([pixel[0], pixel[1], pixel[2]]);
    let h = (h + hue - 0.5).rem_euclid(1.0);
    let [r, g, b] = hsv_to_rgb([h, s * saturation, v * value]);
    lerp_rgb(pixel, [r.max(0.0), g.max(0.0), b.max(0.0), pixel[3]], fac)
}

/// A colour balance: lift, gamma, and gain, the ASC CDL's offset, power, and slope, or a white
/// point change (a matrix on linear RGB, [`white_point`]).
#[derive(Debug, Clone, Copy)]
pub enum Balance {
    LiftGammaGain { lift: (f32, Rgba), gamma: (f32, Rgba), gain: (f32, Rgba) },
    OffsetPowerSlope { offset: (f32, Rgba), power: (f32, Rgba), slope: (f32, Rgba) },
    WhitePoint([[f32; 3]; 3]),
}

/// A base number added to a colour's channels.
fn sum((base, tint): (f32, Rgba), channel: usize) -> f32 {
    base + tint.get(channel).copied().unwrap_or(0.0)
}

/// A base number times a colour's channels.
fn product((base, tint): (f32, Rgba), channel: usize) -> f32 {
    base * tint.get(channel).copied().unwrap_or(1.0)
}

/// Balances the colour, by `fac`. Lift moves the dark end ((x − 1)(1 − lift) + 1, with the
/// colour's lift 1 and the base 0 changing nothing), gain scales, and gamma bends what is above
/// zero; offset, power, and slope are (x × slope + offset) to the power.
pub fn balance(pixel: Rgba, fac: f32, balance: &Balance) -> Rgba {
    if let Balance::WhitePoint(matrix) = balance {
        let row = |r: [f32; 3]| r[0] * pixel[0] + r[1] * pixel[1] + r[2] * pixel[2];
        return lerp_rgb(pixel, [row(matrix[0]), row(matrix[1]), row(matrix[2]), pixel[3]], fac);
    }
    let mut out = pixel;
    for (channel, value) in out.iter_mut().enumerate().take(3) {
        *value = match *balance {
            Balance::LiftGammaGain { lift, gamma, gain } => {
                let lifted = (*value - 1.0) * (2.0 - sum(lift, channel)) + 1.0;
                let gained = lifted * product(gain, channel);
                let gamma = product(gamma, channel);
                if gained > 0.0 && gamma > 0.0 { gained.powf(1.0 / gamma) } else { gained.max(0.0) }
            }
            Balance::OffsetPowerSlope { offset, power, slope } => {
                let sloped = *value * product(slope, channel) + sum(offset, channel);
                if sloped > 0.0 { sloped.powf(product(power, channel)) } else { 0.0 }
            }
            Balance::WhitePoint(_) => *value,
        };
    }
    lerp_rgb(pixel, out, fac)
}

/// Robertson's isotherms (1968): reciprocal megakelvin, the Planckian locus's CIE 1960 u and v
/// there, and the isotherm's slope.
const ISOTHERMS: [[f64; 4]; 31] = [
    [0.0, 0.18006, 0.26352, -0.24341],
    [10.0, 0.18066, 0.26589, -0.25479],
    [20.0, 0.18133, 0.26846, -0.26876],
    [30.0, 0.18208, 0.27119, -0.28539],
    [40.0, 0.18293, 0.27407, -0.30470],
    [50.0, 0.18388, 0.27709, -0.32675],
    [60.0, 0.18494, 0.28021, -0.35156],
    [70.0, 0.18611, 0.28342, -0.37915],
    [80.0, 0.18740, 0.28668, -0.40955],
    [90.0, 0.18880, 0.28997, -0.44278],
    [100.0, 0.19032, 0.29326, -0.47888],
    [125.0, 0.19462, 0.30141, -0.58204],
    [150.0, 0.19962, 0.30921, -0.70471],
    [175.0, 0.20525, 0.31647, -0.84901],
    [200.0, 0.21142, 0.32312, -1.0182],
    [225.0, 0.21807, 0.32909, -1.2168],
    [250.0, 0.22511, 0.33439, -1.4512],
    [275.0, 0.23247, 0.33904, -1.7298],
    [300.0, 0.24010, 0.34308, -2.0637],
    [325.0, 0.24792, 0.34655, -2.4681],
    [350.0, 0.25591, 0.34951, -2.9641],
    [375.0, 0.26400, 0.35200, -3.5814],
    [400.0, 0.27218, 0.35407, -4.3633],
    [425.0, 0.28039, 0.35577, -5.3762],
    [450.0, 0.28863, 0.35714, -6.7262],
    [475.0, 0.29685, 0.35823, -8.5955],
    [500.0, 0.30505, 0.35907, -11.324],
    [525.0, 0.31320, 0.35968, -15.628],
    [550.0, 0.32129, 0.36011, -23.325],
    [575.0, 0.32931, 0.36038, -40.770],
    [600.0, 0.33724, 0.36051, -116.45],
];

/// The white of a light at `temperature` kelvin, moved off the Planckian locus by `tint` (along
/// the isotherm, 3000 tint to a unit of CIE 1960 uv, positive towards magenta), as CIE XYZ with
/// Y = 1.
pub fn white_of(temperature: f32, tint: f32) -> [f64; 3] {
    let mired = (1.0e6 / f64::from(temperature.max(1.0))).clamp(0.0, 600.0);
    let index = ISOTHERMS.iter().rposition(|row| row[0] <= mired).unwrap_or(0).min(ISOTHERMS.len() - 2);
    let (Some(a), Some(b)) = (ISOTHERMS.get(index), ISOTHERMS.get(index + 1)) else { return [0.9505, 1.0, 1.089] };
    let f = (mired - a[0]) / (b[0] - a[0]);
    let mut u = a[1] + (b[1] - a[1]) * f;
    let mut v = a[2] + (b[2] - a[2]) * f;
    let unit = |slope: f64| {
        let length = (1.0 + slope * slope).sqrt();
        (1.0 / length, slope / length)
    };
    let ((ua, va), (ub, vb)) = (unit(a[3]), unit(b[3]));
    let (du, dv) = (ua + (ub - ua) * f, va + (vb - va) * f);
    let length = du.hypot(dv).max(1e-12);
    let offset = -f64::from(tint) / 3000.0;
    u += du / length * offset;
    v += dv / length * offset;
    let d = 2.0 * u - 8.0 * v + 4.0;
    let (x, y) = (3.0 * u / d, 2.0 * v / d);
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Linear Rec. 709 RGB to CIE XYZ (D65).
const RGB_TO_XYZ: [[f64; 3]; 3] =
    [[0.412_390_80, 0.357_584_34, 0.180_480_79], [0.212_639_01, 0.715_168_68, 0.072_192_32], [0.019_330_82, 0.119_194_78, 0.950_532_15]];
/// The Bradford cone response.
const BRADFORD: [[f64; 3]; 3] = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let column = |j: usize| b.map(|row| row.get(j).copied().unwrap_or(0.0));
    let columns = [column(0), column(1), column(2)];
    a.map(|row| columns.map(|column| row[0] * column[0] + row[1] * column[1] + row[2] * column[2]))
}

fn mat_vec(a: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    a.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

fn mat_inverse(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let det = if det.abs() > 1e-300 { det } else { 1.0 };
    [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

/// The matrix on linear RGB that makes what was white under the input light white under the
/// output light (a Bradford adaptation between the two whites).
pub fn white_point(input: (f32, f32), output: (f32, f32)) -> [[f32; 3]; 3] {
    let (from, to) = (mat_vec(&BRADFORD, white_of(input.0, input.1)), mat_vec(&BRADFORD, white_of(output.0, output.1)));
    let scale = [[to[0] / from[0], 0.0, 0.0], [0.0, to[1] / from[1], 0.0], [0.0, 0.0, to[2] / from[2]]];
    let adapt = mat_mul(&mat_inverse(&BRADFORD), &mat_mul(&scale, &BRADFORD));
    let matrix = mat_mul(&mat_inverse(&RGB_TO_XYZ), &mat_mul(&adapt, &RGB_TO_XYZ));
    matrix.map(|row| row.map(|value| if value.is_finite() { value as f32 } else { 0.0 }))
}

/// One curve of RGB Curves: control points from left to right, joined smoothly.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    points: Vec<[f32; 2]>,
    /// The Bézier segments between the points: start, two handles, end.
    segments: Vec<[[f32; 2]; 4]>,
}

/// How far the reference's automatic handles reach (a tuning of its curves, found by fitting
/// its output).
const HANDLE: f32 = 2.5614;

impl Curve {
    /// A curve through `points` (sorted by x; at least two). Inner points take automatic
    /// handles; an end point's handle points at its neighbour's near handle (or the neighbour
    /// itself), so a curve of two points is a straight line.
    pub fn new(mut points: Vec<[f32; 2]>) -> Option<Self> {
        points.retain(|point| point.iter().all(|value| value.is_finite()));
        points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        if !(2..=64).contains(&points.len()) {
            return None;
        }
        // Inner points: automatic handles from their neighbours.
        let mut handles: Vec<Option<([f32; 2], [f32; 2])>> = vec![None];
        handles.extend(points.windows(3).map(|three| match three {
            [before, here, after] => Some(auto_handles(*before, *here, *after)),
            _ => None,
        }));
        handles.push(None);
        // End points: toward the neighbour's near handle, or the neighbour.
        let end = |here: [f32; 2], next: [f32; 2], near: Option<[f32; 2]>| {
            let toward = near.unwrap_or(next);
            let direction = [toward[0] - here[0], toward[1] - here[1]];
            let length = direction[0].hypot(direction[1]).max(1e-6);
            let span = (next[0] - here[0]).hypot(next[1] - here[1]);
            let scale = span / HANDLE / length;
            let handle = [here[0] + direction[0] * scale, here[1] + direction[1] * scale];
            (mirror(here, handle), handle)
        };
        let (first, second) = (*points.first()?, *points.get(1)?);
        let (last, before_last) = (*points.last()?, *points.get(points.len() - 2)?);
        let start = end(first, second, handles.get(1).copied().flatten().map(|handle| handle.0));
        let finish = end(last, before_last, handles.get(handles.len() - 2).copied().flatten().map(|handle| handle.1));
        if let Some(slot) = handles.first_mut() {
            *slot = Some(start);
        }
        if let Some(slot) = handles.last_mut() {
            *slot = Some((finish.1, finish.0));
        }
        let handles: Vec<([f32; 2], [f32; 2])> = handles.into_iter().map(|handle| handle.unwrap_or(([0.0; 2], [0.0; 2]))).collect();
        let segments = points
            .windows(2)
            .zip(handles.windows(2))
            .filter_map(|(pair, pair_handles)| match (pair, pair_handles) {
                ([from, to], [from_handles, to_handles]) => Some([*from, from_handles.1, to_handles.0, *to]),
                _ => None,
            })
            .collect();
        Some(Self { points, segments })
    }

    /// The curve's value at `x`; beyond the end points it goes on straight along the end
    /// handles.
    pub fn at(&self, x: f32) -> f32 {
        let (Some(first), Some(last)) = (self.segments.first(), self.segments.last()) else { return x };
        if x <= first[0][0] {
            let run = first[1][0] - first[0][0];
            let slope = if run.abs() > 1e-6 { (first[1][1] - first[0][1]) / run } else { 0.0 };
            return first[0][1] + (x - first[0][0]) * slope;
        }
        if x >= last[3][0] {
            let run = last[3][0] - last[2][0];
            let slope = if run.abs() > 1e-6 { (last[3][1] - last[2][1]) / run } else { 0.0 };
            return last[3][1] + (x - last[3][0]) * slope;
        }
        for segment in &self.segments {
            if x <= segment[3][0] {
                // Solve x(t) = x by bisection (x grows with t for these handles).
                let (mut low, mut high) = (0.0f32, 1.0f32);
                for _ in 0..32 {
                    let mid = 0.5 * (low + high);
                    if bezier(segment, mid)[0] < x {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                return bezier(segment, 0.5 * (low + high))[1];
            }
        }
        last[3][1]
    }
}

fn mirror(point: [f32; 2], other: [f32; 2]) -> [f32; 2] {
    [2.0 * point[0] - other[0], 2.0 * point[1] - other[1]]
}

/// The left and right handles of a point between `before` and `after`.
fn auto_handles(before: [f32; 2], here: [f32; 2], after: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let a = [here[0] - before[0], here[1] - before[1]];
    let b = [after[0] - here[0], after[1] - here[1]];
    let len_a = a[0].hypot(a[1]).max(1e-6);
    let len_b = b[0].hypot(b[1]).max(1e-6);
    let t = [a[0] / len_a + b[0] / len_b, a[1] / len_a + b[1] / len_b];
    let len = t[0].hypot(t[1]) * HANDLE;
    if len <= 1e-6 {
        return (here, here);
    }
    let left = [here[0] - t[0] * len_a / len, here[1] - t[1] * len_a / len];
    let right = [here[0] + t[0] * len_b / len, here[1] + t[1] * len_b / len];
    (left, right)
}

fn bezier(segment: &[[f32; 2]; 4], t: f32) -> [f32; 2] {
    let s = 1.0 - t;
    let [a, b, c, d] = *segment;
    let w = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
    [w[0] * a[0] + w[1] * b[0] + w[2] * c[0] + w[3] * d[0], w[0] * a[1] + w[1] * b[1] + w[2] * c[1] + w[3] * d[1]]
}

/// RGB Curves' curves: the combined one and one per channel (straight lines when not set).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Curves {
    pub combined: Option<Curve>,
    pub channels: [Option<Curve>; 3],
}

impl Curves {
    /// The curves a node's `curves` value gives: `{ "c": [[x, y], ...], "r": ..., "g": ..., "b": ... }`.
    pub fn from_json(value: Option<&Value>) -> Self {
        let curve = |key: &str| {
            let points: Vec<[f32; 2]> = value?
                .get(key)?
                .as_array()?
                .iter()
                .filter_map(|point| {
                    let items = point.as_array()?;
                    Some([items.first()?.as_f64()? as f32, items.get(1)?.as_f64()? as f32])
                })
                .collect();
            Curve::new(points)
        };
        Self { combined: curve("c"), channels: [curve("r"), curve("g"), curve("b")] }
    }

    /// Maps the black and white levels to 0 and 1, then the channel's curve, then the combined
    /// one, by `fac`.
    pub fn apply(&self, pixel: Rgba, fac: f32, black: Rgba, white: Rgba) -> Rgba {
        let mut out = pixel;
        for (((value, black), white), curve) in out.iter_mut().zip(black).zip(white).zip(&self.channels) {
            let span = white - black;
            let mut x = if span.abs() > 1e-6 { (*value - black) / span } else { *value - black };
            if let Some(curve) = curve {
                x = curve.at(x);
            }
            if let Some(curve) = &self.combined {
                x = curve.at(x);
            }
            *value = x;
        }
        lerp_rgb(pixel, out, fac)
    }
}

/// The Mix node's blend modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Mix,
    Darken,
    Multiply,
    Burn,
    Lighten,
    Screen,
    Dodge,
    Add,
    Overlay,
    SoftLight,
    LinearLight,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Value,
}

impl Blend {
    pub fn parse(text: &str) -> Self {
        match text {
            "darken" => Self::Darken,
            "multiply" => Self::Multiply,
            "burn" => Self::Burn,
            "lighten" => Self::Lighten,
            "screen" => Self::Screen,
            "dodge" => Self::Dodge,
            "add" => Self::Add,
            "overlay" => Self::Overlay,
            "softLight" => Self::SoftLight,
            "linearLight" => Self::LinearLight,
            "difference" => Self::Difference,
            "exclusion" => Self::Exclusion,
            "subtract" => Self::Subtract,
            "divide" => Self::Divide,
            "hue" => Self::Hue,
            "saturation" => Self::Saturation,
            "color" => Self::Color,
            "value" => Self::Value,
            _ => Self::Mix,
        }
    }
}

/// Mixes `b` into `a` by `fac` with `blend`.
pub fn mix(blend: Blend, fac: f32, a: Rgba, b: Rgba) -> Rgba {
    let keep = 1.0 - fac;
    let mut out = a;
    let per = |f: &dyn Fn(f32, f32) -> f32| [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])];
    let rgb: [f32; 3] = match blend {
        Blend::Mix => per(&|a, b| lerp(a, b, fac)),
        Blend::Add => per(&|a, b| a + fac * b),
        Blend::Subtract => per(&|a, b| a - fac * b),
        Blend::Multiply => per(&|a, b| a * (keep + fac * b)),
        Blend::Screen => per(&|a, b| 1.0 - (keep + fac * (1.0 - b)) * (1.0 - a)),
        Blend::Divide => per(&|a, b| if b != 0.0 { keep * a + fac * a / b } else { a }),
        Blend::Difference => per(&|a, b| lerp(a, (a - b).abs(), fac)),
        Blend::Exclusion => per(&|a, b| lerp(a, a + b - 2.0 * a * b, fac).max(0.0)),
        Blend::Darken => per(&|a, b| lerp(a, a.min(b), fac)),
        Blend::Lighten => per(&|a, b| lerp(a, a.max(b), fac)),
        Blend::Overlay => per(&|a, b| if a < 0.5 { a * (keep + 2.0 * fac * b) } else { 1.0 - (keep + 2.0 * fac * (1.0 - b)) * (1.0 - a) }),
        Blend::Dodge => per(&|a, b| {
            if a == 0.0 {
                return a;
            }
            let rest = 1.0 - fac * b;
            if rest <= 0.0 { 1.0 } else { (a / rest).min(1.0) }
        }),
        Blend::Burn => per(&|a, b| {
            let rest = keep + fac * b;
            if rest <= 0.0 { 0.0 } else { (1.0 - (1.0 - a) / rest).clamp(0.0, 1.0) }
        }),
        Blend::SoftLight => per(&|a, b| {
            let screen = 1.0 - (1.0 - b) * (1.0 - a);
            keep * a + fac * ((1.0 - a) * b * a + a * screen)
        }),
        Blend::LinearLight => per(&|a, b| if b > 0.5 { a + fac * (2.0 * (b - 0.5)) } else { a + fac * (2.0 * b - 1.0) }),
        Blend::Hue | Blend::Saturation | Blend::Color | Blend::Value => {
            let [ah, as_, av] = rgb_to_hsv([a[0], a[1], a[2]]);
            let [bh, bs, bv] = rgb_to_hsv([b[0], b[1], b[2]]);
            let mixed = match blend {
                Blend::Hue if bs != 0.0 => hsv_to_rgb([bh, as_, av]),
                Blend::Saturation if as_ != 0.0 => hsv_to_rgb([ah, bs, av]),
                Blend::Color if bs != 0.0 => hsv_to_rgb([bh, bs, av]),
                Blend::Value => hsv_to_rgb([ah, as_, bv]),
                _ => [a[0], a[1], a[2]],
            };
            [lerp(a[0], mixed[0], fac), lerp(a[1], mixed[1], fac), lerp(a[2], mixed[2], fac)]
        }
    };
    out[0] = rgb[0];
    out[1] = rgb[1];
    out[2] = rgb[2];
    out[3] = if blend == Blend::Mix { lerp(a[3], b[3], fac) } else { a[3] };
    out
}

/// How Alpha Over lays the foreground on the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    Over,
    Disjoint,
    Conjoint,
}

impl Over {
    pub fn parse(text: &str) -> Self {
        match text {
            "disjointOver" => Self::Disjoint,
            "conjointOver" => Self::Conjoint,
            _ => Self::Over,
        }
    }
}

/// Lays `foreground` over `background` by `fac` (premultiplied, unless `straight` says the
/// foreground is not).
pub fn alpha_over(how: Over, background: Rgba, foreground: Rgba, fac: f32, straight: bool) -> Rgba {
    let fg = if straight {
        [foreground[0] * foreground[3], foreground[1] * foreground[3], foreground[2] * foreground[3], foreground[3]]
    } else {
        foreground
    };
    let alpha = fac * fg[3];
    let behind = match how {
        Over::Over => 1.0 - alpha,
        Over::Disjoint => {
            if background[3] <= 0.0 {
                1.0
            } else {
                ((1.0 - alpha) / background[3]).min(1.0)
            }
        }
        Over::Conjoint => {
            if background[3] <= 0.0 {
                1.0
            } else {
                (1.0 - alpha / background[3]).max(0.0)
            }
        }
    };
    let mut out = [0.0; 4];
    for ((value, back), front) in out.iter_mut().zip(background).zip(fg) {
        *value = behind * back + fac * front;
    }
    out
}
