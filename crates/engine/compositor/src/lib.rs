//! The Scene kit's compositor: runs a compositing node group (`Tree::Compositor`) on a render's
//! picture. Pictures are linear RGB with premultiplied alpha, rows from the top; every picture in
//! one run has the render's size. A number or colour with nothing linked into it stands for a
//! picture of that value everywhere, and stays one value until a picture meets it.
//!
//! The nodes follow the reference application's compositor (version 5): their effects were
//! measured from its output on fixed pictures (`tests/compositor_blender.rs`), never read from
//! its source.

pub mod blur;
pub mod color;
pub mod convolve;
pub mod cryptomatte;
pub mod filter;
pub mod glare;
pub mod keying;
pub mod kuwahara;
pub mod shape;
pub mod transform;

use std::collections::BTreeMap;
use std::sync::Arc;

use kition_scene_model::Id;
use kition_scene_model::node_tree::{self, Node, NodeGroup, NodeKind, SocketType, Tree};
use serde_json::Value;

/// One RGBA pixel: linear, premultiplied.
pub type Rgba = [f32; 4];

/// A picture: linear RGB with premultiplied alpha, `width × height` pixels from the top row.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgba>,
}

impl Image {
    /// A picture of one colour.
    pub fn filled(width: usize, height: usize, color: Rgba) -> Self {
        Self { width, height, pixels: vec![color; width * height] }
    }

    /// The pixel at (`x`, `y`); `None` outside the picture.
    pub fn get(&self, x: isize, y: isize) -> Option<Rgba> {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return None;
        }
        self.pixels.get(y as usize * self.width + x as usize).copied()
    }

    /// The picture at (`u`, `v`) in pixels, between pixel centres (bilinear): beyond the edges the
    /// edge pixels repeat, or the picture is clear when `clear`.
    pub fn sample(&self, u: f32, v: f32, clear: bool) -> Rgba {
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let pick = |x: isize, y: isize| if clear { self.get(x, y).unwrap_or([0.0; 4]) } else { self.at(x, y) };
        let (a, b, c, d) = (pick(x0, y0), pick(x0 + 1, y0), pick(x0, y0 + 1), pick(x0 + 1, y0 + 1));
        let mut out = [0.0; 4];
        for ((((value, a), b), c), d) in out.iter_mut().zip(a).zip(b).zip(c).zip(d) {
            let top = a + (b - a) * fx;
            let bottom = c + (d - c) * fx;
            *value = top + (bottom - top) * fy;
        }
        out
    }

    /// The pixel at (`x`, `y`), clamped to the picture's edges.
    pub fn at(&self, x: isize, y: isize) -> Rgba {
        let x = x.clamp(0, self.width as isize - 1).max(0) as usize;
        let y = y.clamp(0, self.height as isize - 1).max(0) as usize;
        self.pixels.get(y * self.width + x).copied().unwrap_or([0.0; 4])
    }
}

/// What a render hands the compositor: its picture, when the renderer knows it each pixel's
/// distance from the camera (infinity where nothing was drawn), the pictures Image nodes name
/// (by the scene image's hash), the render's other passes, and its camera's lens.
#[derive(Clone, Copy)]
pub struct Passes<'a> {
    pub image: &'a Image,
    pub depth: Option<&'a [f32]>,
    pub images: &'a dyn Fn(&str) -> Option<Image>,
    pub layers: Option<&'a Layers>,
    pub lens: Option<Lens>,
}

/// The passes a render gives beyond its picture and depth, each one value per pixel of the
/// render's size, by the Render Layers output that shows it (`Normal`, `Vector`,
/// `Diffuse Direct`, ...), and its Cryptomatte layers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layers {
    pub passes: BTreeMap<String, Vec<Rgba>>,
    pub cryptomatte: Option<cryptomatte::Layers>,
}

/// The camera a render was made through, for Defocus: its focal length and the sensor's width
/// across the picture (millimetres), and the distance it is focused at (metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    pub focal_length: f32,
    pub sensor_width: f32,
    pub focus_distance: f32,
}

impl Lens {
    /// The reference's default camera: 50 mm on a 36 mm sensor, focused at 10 m.
    pub const DEFAULT: Self = Self { focal_length: 50.0, sensor_width: 36.0, focus_distance: 10.0 };
}

/// No pictures for Image nodes.
pub fn no_images(_hash: &str) -> Option<Image> {
    None
}

/// What a compositing group makes: the group's output, and what its Viewer node shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Composited {
    pub image: Option<Image>,
    pub viewer: Option<Image>,
}

/// A value flowing between nodes: one value everywhere, or a value per pixel.
#[derive(Debug, Clone)]
pub enum Plane {
    Single(Rgba),
    Pixels(Arc<Vec<Rgba>>),
}

impl Plane {
    /// The value at pixel `index`.
    pub fn get(&self, index: usize) -> Rgba {
        match self {
            Self::Single(value) => *value,
            Self::Pixels(pixels) => pixels.get(index).copied().unwrap_or([0.0; 4]),
        }
    }

    /// The value if it is one everywhere, else the mean of its pixels.
    pub fn single(&self) -> Rgba {
        match self {
            Self::Single(value) => *value,
            Self::Pixels(pixels) => {
                let mut sum = [0.0f64; 4];
                for pixel in pixels.iter() {
                    for (total, value) in sum.iter_mut().zip(pixel) {
                        *total += f64::from(*value);
                    }
                }
                let count = pixels.len().max(1) as f64;
                sum.map(|value| (value / count) as f32)
            }
        }
    }
}

/// What a value is: a number (in every channel), a colour, or a vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Float,
    Color,
    Vector,
}

#[derive(Debug, Clone)]
struct Val {
    kind: Kind,
    plane: Plane,
}

/// The weights the reference turns a colour into a number with (its scene's luminance).
pub const LUMINANCE: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn convert_pixel(value: Rgba, from: Kind, to: Kind) -> Rgba {
    match (from, to) {
        (a, b) if a == b => value,
        (Kind::Color, Kind::Float) => {
            let luma = value[0] * LUMINANCE[0] + value[1] * LUMINANCE[1] + value[2] * LUMINANCE[2];
            [luma, luma, luma, 1.0]
        }
        (Kind::Vector, Kind::Float) => {
            let mean = (value[0] + value[1] + value[2]) / 3.0;
            [mean, mean, mean, 1.0]
        }
        (Kind::Float, Kind::Color) | (Kind::Vector, Kind::Color) => [value[0], value[1], value[2], 1.0],
        (_, Kind::Vector) => [value[0], value[1], value[2], 0.0],
        _ => value,
    }
}

fn kind_of(socket: SocketType) -> Kind {
    match socket {
        SocketType::Color => Kind::Color,
        SocketType::Vector | SocketType::Rotation => Kind::Vector,
        _ => Kind::Float,
    }
}

/// The value of JSON `value` (a socket's stored value or default) as a pixel of `kind`.
fn json_pixel(value: &Value, kind: Kind) -> Rgba {
    let number = |value: &Value| value.as_f64().map(|number| number as f32).filter(|number| number.is_finite());
    let item = |index: usize, fallback: f32| value.as_array().and_then(|items| items.get(index)).and_then(number).unwrap_or(fallback);
    match (kind, value) {
        (_, Value::Bool(flag)) => {
            let value = if *flag { 1.0 } else { 0.0 };
            [value, value, value, 1.0]
        }
        (Kind::Float, _) => {
            let value = number(value).unwrap_or(0.0);
            [value, value, value, 1.0]
        }
        // Colours stop at zero, as the reference's colour sockets do.
        (Kind::Color, Value::Array(_)) => [item(0, 0.0).max(0.0), item(1, 0.0).max(0.0), item(2, 0.0).max(0.0), item(3, 1.0).clamp(0.0, 1.0)],
        (Kind::Vector, Value::Array(_)) => [item(0, 0.0), item(1, 0.0), item(2, 0.0), 0.0],
        (_, other) => {
            let value = number(other).unwrap_or(0.0);
            [value, value, value, if kind == Kind::Color { 1.0 } else { 0.0 }]
        }
    }
}

/// The largest picture the compositor works on (pixels).
pub const MAX_PIXELS: usize = 8192 * 8192;

/// Runs `group` on a render's passes.
pub fn composite(group: &NodeGroup, passes: &Passes<'_>) -> Result<Composited, String> {
    if group.tree != Tree::Compositor {
        return Err(format!("{} is not a compositing node group", group.name));
    }
    let (width, height) = (passes.image.width, passes.image.height);
    if width == 0 || height == 0 || width * height > MAX_PIXELS || passes.image.pixels.len() != width * height {
        return Err("the render's picture has no pixels or the wrong number of them".into());
    }
    let mut run = Run { group, passes, width, height, done: BTreeMap::new(), pending: Vec::new() };
    let mut out = Composited { image: None, viewer: None };
    // The active output: the last Group Output (the reference uses its active one), and the
    // first Viewer.
    if let Some(node) = group.nodes.iter().rev().find(|node| node.kind == "compositorOutput") {
        out.image = Some(run.picture(node, "Image")?);
    }
    if let Some(node) = group.nodes.iter().find(|node| node.kind == "viewer") {
        out.viewer = Some(run.picture(node, "Image")?);
    }
    Ok(out)
}

struct Run<'a> {
    group: &'a NodeGroup,
    passes: &'a Passes<'a>,
    width: usize,
    height: usize,
    done: BTreeMap<Id, BTreeMap<String, Val>>,
    pending: Vec<Id>,
}

/// A node's inputs as the node code reads them.
pub struct Inputs<'a, 'b> {
    run: &'b mut Run<'a>,
    node: &'a Node,
    kind: &'static NodeKind,
}

impl Inputs<'_, '_> {
    /// The width and height of the pictures of this run.
    pub fn size(&self) -> (usize, usize) {
        (self.run.width, self.run.height)
    }

    /// Input `id` as a plane of `kind`.
    pub fn plane(&mut self, id: &str, kind: Kind) -> Result<Plane, String> {
        let value = self.run.input(self.node, self.kind, id)?;
        Ok(match value.plane {
            Plane::Single(pixel) => Plane::Single(convert_pixel(pixel, value.kind, kind)),
            Plane::Pixels(pixels) if value.kind == kind => Plane::Pixels(pixels),
            Plane::Pixels(pixels) => Plane::Pixels(Arc::new(pixels.iter().map(|pixel| convert_pixel(*pixel, value.kind, kind)).collect())),
        })
    }

    /// Input `id` as it comes, every channel kept (a speed pass's four components).
    pub fn raw(&mut self, id: &str) -> Result<Plane, String> {
        Ok(self.run.input(self.node, self.kind, id)?.plane)
    }

    /// Input `id` as a picture (a single value fills the picture).
    pub fn image(&mut self, id: &str) -> Result<Image, String> {
        let (width, height) = self.size();
        Ok(match self.plane(id, Kind::Color)? {
            Plane::Single(value) => Image::filled(width, height, value),
            Plane::Pixels(pixels) => Image { width, height, pixels: Arc::unwrap_or_clone(pixels) },
        })
    }

    /// Input `id` as one number (a picture gives its mean).
    pub fn number(&mut self, id: &str) -> Result<f32, String> {
        Ok(self.plane(id, Kind::Float)?.single()[0])
    }

    /// Input `id` as one colour.
    pub fn color(&mut self, id: &str) -> Result<Rgba, String> {
        Ok(self.plane(id, Kind::Color)?.single())
    }

    /// Input `id` as one vector.
    pub fn vector(&mut self, id: &str) -> Result<[f32; 3], String> {
        let value = self.plane(id, Kind::Vector)?.single();
        Ok([value[0], value[1], value[2]])
    }

    /// Input `id` as on or off.
    pub fn flag(&mut self, id: &str) -> Result<bool, String> {
        Ok(self.number(id)? > 0.5)
    }

    /// Input `id` as a whole number (rounded; zero when not finite).
    pub fn integer(&mut self, id: &str) -> Result<i64, String> {
        let value = self.number(id)?;
        Ok(if value.is_finite() { value.round().clamp(-1.0e9, 1.0e9) as i64 } else { 0 })
    }

    /// Input `id` as one number per pixel.
    pub fn numbers(&mut self, id: &str) -> Result<Vec<f32>, String> {
        let (width, height) = self.size();
        Ok(match self.plane(id, Kind::Float)? {
            Plane::Single(value) => vec![value[0]; width * height],
            Plane::Pixels(pixels) => pixels.iter().map(|pixel| pixel[0]).collect(),
        })
    }

    /// How the node reads its picture between pixels and past its edges.
    pub fn sampler(&self) -> transform::Sampler {
        transform::Sampler {
            interpolation: transform::Interpolation::parse(self.setting("Interpolation")),
            x: transform::Extension::parse(self.setting("Extension X")),
            y: transform::Extension::parse(self.setting("Extension Y")),
        }
    }

    /// Setting `id`'s choice.
    pub fn setting(&self, id: &str) -> &str {
        node_tree::setting(self.node, self.kind, id)
    }

    /// The node's stored value `id` (data inputs such as curves).
    pub fn data(&self, id: &str) -> Option<&Value> {
        self.node.values.get(id)
    }

    /// The render's depth, if the renderer gave it.
    pub fn depth(&self) -> Option<&[f32]> {
        self.run.passes.depth
    }

    /// The render's picture.
    pub fn render(&self) -> &Image {
        self.run.passes.image
    }

    /// The picture an Image node names, placed at the middle of this run's pictures at its own
    /// size (clear around it), as the reference places an image of another size: across, an
    /// odd difference rounds down from the left, as the reference does; down, it rounds towards
    /// the top (the reference rounds from the bottom: a smaller picture of an odd height sits a
    /// row lower there; kept as it is, the case `an_image_node_lays_a_picture_over_the_render`
    /// says so).
    pub fn named_image(&self, hash: &str) -> Option<Image> {
        let source = (self.run.passes.images)(hash)?;
        let (width, height) = self.size();
        if source.width == width && source.height == height {
            return Some(source);
        }
        let left = (width as isize - source.width as isize).div_euclid(2);
        let top = (height as isize - source.height as isize) / 2;
        let mut out = Image::filled(width, height, [0.0; 4]);
        for (index, pixel) in out.pixels.iter_mut().enumerate() {
            let (sx, sy) = ((index % width) as isize - left, (index / width) as isize - top);
            if sx >= 0 && sy >= 0 && (sx as usize) < source.width && (sy as usize) < source.height {
                *pixel = source.pixels.get(sy as usize * source.width + sx as usize).copied().unwrap_or([0.0; 4]);
            }
        }
        Some(out)
    }
}

/// The values of several planes at one pixel, by the planes' order.
pub struct Values(Vec<Rgba>);

impl Values {
    /// Plane `index`'s value as a colour.
    pub fn pixel(&self, index: usize) -> Rgba {
        self.0.get(index).copied().unwrap_or([0.0; 4])
    }

    /// Plane `index`'s value as a number.
    pub fn number(&self, index: usize) -> f32 {
        self.pixel(index)[0]
    }
}

/// Applies `f` to the pixels of `planes` (one value when every plane is one).
pub fn per_pixel(planes: &[&Plane], count: usize, f: impl Fn(&Values) -> Rgba) -> Plane {
    if planes.iter().all(|plane| matches!(plane, Plane::Single(_))) {
        return Plane::Single(f(&Values(planes.iter().map(|plane| plane.get(0)).collect())));
    }
    let mut values = Values(vec![[0.0; 4]; planes.len()]);
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        for (slot, plane) in values.0.iter_mut().zip(planes) {
            *slot = plane.get(index);
        }
        out.push(f(&values));
    }
    Plane::Pixels(Arc::new(out))
}

impl<'a> Run<'a> {
    fn picture(&mut self, node: &'a Node, socket: &str) -> Result<Image, String> {
        let kind = node_tree::node_kind(&node.kind).ok_or_else(|| format!("unknown node kind {}", node.kind))?;
        let mut inputs = Inputs { run: self, node, kind };
        inputs.image(socket)
    }

    /// The value flowing into input `id` of `node`: from its link, else its own value or default.
    fn input(&mut self, node: &'a Node, kind: &'static NodeKind, id: &str) -> Result<Val, String> {
        let socket = kind.inputs.iter().find(|socket| socket.id == id).ok_or_else(|| format!("{} has no input {id}", kind.title))?;
        let wanted = kind_of(node_tree::socket_type(node, kind, socket));
        if let Some(link) = self.group.links.iter().find(|link| link.to == node.id && link.to_socket == id) {
            let from = self.group.node(link.from).ok_or("a link comes from a missing node")?;
            let outputs = self.outputs(from)?;
            if let Some(value) = outputs.get(&link.from_socket) {
                return Ok(value.clone());
            }
        }
        let value = node.values.get(id).cloned().unwrap_or_else(|| socket.default.json());
        Ok(Val { kind: wanted, plane: Plane::Single(json_pixel(&value, wanted)) })
    }

    fn outputs(&mut self, node: &'a Node) -> Result<BTreeMap<String, Val>, String> {
        if let Some(done) = self.done.get(&node.id) {
            return Ok(done.clone());
        }
        if self.pending.contains(&node.id) {
            return Err("the node group has a cycle".into());
        }
        self.pending.push(node.id);
        let kind = node_tree::node_kind(&node.kind).ok_or_else(|| format!("unknown node kind {}", node.kind))?;
        let outputs = {
            let mut inputs = Inputs { run: self, node, kind };
            evaluate(&mut inputs)?
        };
        let mut typed = BTreeMap::new();
        for (id, plane) in outputs {
            let socket_kind = kind.outputs.iter().find(|socket| socket.id == id).map_or(Kind::Color, |socket| kind_of(socket.kind));
            typed.insert(id, Val { kind: socket_kind, plane });
        }
        self.pending.retain(|id| *id != node.id);
        self.done.insert(node.id, typed.clone());
        Ok(typed)
    }
}

fn picture_plane(image: Image) -> Plane {
    Plane::Pixels(Arc::new(image.pixels))
}

/// One node's outputs.
fn evaluate(inputs: &mut Inputs<'_, '_>) -> Result<Vec<(String, Plane)>, String> {
    let (width, height) = inputs.size();
    let count = width * height;
    let one = |id: &str, plane: Plane| Ok(vec![(id.to_owned(), plane)]);
    match inputs.node.kind.as_str() {
        "renderLayers" => {
            let render = inputs.render().clone();
            let alpha = Plane::Pixels(Arc::new(render.pixels.iter().map(|pixel| [pixel[3], pixel[3], pixel[3], 1.0]).collect()));
            let depth = match inputs.depth() {
                Some(depth) => Plane::Pixels(Arc::new(
                    depth
                        .iter()
                        .map(|value| {
                            let value = if value.is_finite() { *value } else { BACKGROUND_DEPTH };
                            [value, value, value, 1.0]
                        })
                        .collect(),
                )),
                None => Plane::Single([BACKGROUND_DEPTH, BACKGROUND_DEPTH, BACKGROUND_DEPTH, 1.0]),
            };
            let mut out = vec![("Image".into(), picture_plane(render)), ("Alpha".into(), alpha), ("Depth".into(), depth)];
            let layers = inputs.run.passes.layers;
            for name in RENDER_PASSES {
                let plane = layers
                    .and_then(|layers| layers.passes.get(*name))
                    .filter(|pixels| pixels.len() == count)
                    .map_or(Plane::Single([0.0, 0.0, 0.0, 1.0]), |pixels| Plane::Pixels(Arc::new(pixels.clone())));
                out.push(((*name).to_owned(), plane));
            }
            for (prefix, which) in [("CryptoObject", "object"), ("CryptoMaterial", "material"), ("CryptoAsset", "asset")] {
                let pictures = layers.and_then(|layers| layers.cryptomatte.as_ref()).map(|crypto| crypto.layer(which).pictures());
                for index in 0..cryptomatte::DEPTH.div_ceil(2) {
                    let plane = pictures
                        .as_ref()
                        .and_then(|pictures| pictures.get(index))
                        .filter(|pixels| pixels.len() == count)
                        .map_or(Plane::Single([0.0; 4]), |pixels| Plane::Pixels(Arc::new(pixels.clone())));
                    out.push((format!("{prefix}{index:02}"), plane));
                }
            }
            Ok(out)
        }
        "cryptomatte" => {
            let image = inputs.plane("Image", Kind::Color)?;
            let matte_id = inputs.data("Matte ID").and_then(Value::as_str).unwrap_or_default().to_owned();
            let which = inputs.setting("Layer").to_owned();
            let layer = inputs.run.passes.layers.and_then(|layers| layers.cryptomatte.as_ref()).map(|crypto| crypto.layer(&which));
            let (matte, pick) = match layer.filter(|layer| layer.ranks.len() == count) {
                Some(layer) => {
                    let matte = layer.matte(&cryptomatte::Layer::picked(&matte_id));
                    let pick: Vec<Rgba> = (0..count).map(|index| layer.pick(index)).collect();
                    (Plane::Pixels(Arc::new(matte.into_iter().map(|value| [value, value, value, 1.0]).collect())), Plane::Pixels(Arc::new(pick)))
                }
                None => (Plane::Single([0.0, 0.0, 0.0, 1.0]), Plane::Single([0.0, 0.0, 0.0, 1.0])),
            };
            let keyed = per_pixel(&[&image, &matte], count, |values| keying::keyed(values.pixel(0), values.number(1)));
            Ok(vec![("Image".into(), keyed), ("Matte".into(), matte), ("Pick".into(), pick)])
        }
        "rgb" => one("Color", inputs.plane("Color", Kind::Color)?),
        "image" => {
            let hash = inputs.data("Image").and_then(Value::as_str).unwrap_or_default().to_owned();
            match inputs.named_image(&hash) {
                Some(image) => {
                    let alpha = Plane::Pixels(Arc::new(image.pixels.iter().map(|pixel| [pixel[3], pixel[3], pixel[3], 1.0]).collect()));
                    Ok(vec![("Image".into(), picture_plane(image)), ("Alpha".into(), alpha)])
                }
                None => Ok(vec![("Image".into(), Plane::Single([0.0; 4])), ("Alpha".into(), Plane::Single([0.0, 0.0, 0.0, 1.0]))]),
            }
        }
        "value" => one("Value", inputs.plane("Value", Kind::Float)?),
        "compositorOutput" | "viewer" => Ok(Vec::new()),
        "denoise" => one("Image", inputs.plane("Image", Kind::Color)?),
        "exposure" => {
            let image = inputs.plane("Image", Kind::Color)?;
            let exposure = inputs.plane("Exposure", Kind::Float)?;
            one("Image", per_pixel(&[&image, &exposure], count, |values| color::exposure(values.pixel(0), values.number(1))))
        }
        "brightContrast" => {
            let image = inputs.plane("Image", Kind::Color)?;
            let bright = inputs.plane("Bright", Kind::Float)?;
            let contrast = inputs.plane("Contrast", Kind::Float)?;
            one(
                "Image",
                per_pixel(&[&image, &bright, &contrast], count, |values| color::bright_contrast(values.pixel(0), values.number(1), values.number(2))),
            )
        }
        "invert" => {
            let image = inputs.plane("Color", Kind::Color)?;
            let fac = inputs.plane("Fac", Kind::Float)?;
            let (rgb, alpha) = (inputs.flag("Invert Color")?, inputs.flag("Invert Alpha")?);
            one("Color", per_pixel(&[&image, &fac], count, |values| color::invert(values.pixel(0), values.number(1), rgb, alpha)))
        }
        "hueSaturation" => {
            let image = inputs.plane("Image", Kind::Color)?;
            let planes = [
                inputs.plane("Hue", Kind::Float)?,
                inputs.plane("Saturation", Kind::Float)?,
                inputs.plane("Value", Kind::Float)?,
                inputs.plane("Fac", Kind::Float)?,
            ];
            one(
                "Image",
                per_pixel(&[&image, &planes[0], &planes[1], &planes[2], &planes[3]], count, |values| {
                    color::hue_saturation(values.pixel(0), values.number(1), values.number(2), values.number(3), values.number(4))
                }),
            )
        }
        "colorBalance" => {
            let image = inputs.plane("Image", Kind::Color)?;
            let fac = inputs.plane("Fac", Kind::Float)?;
            let how = inputs.setting("Type").to_owned();
            let mut pair = |base: &str, tint: &str| -> Result<(f32, Rgba), String> { Ok((inputs.number(base)?, inputs.color(tint)?)) };
            let balance = match how.as_str() {
                "whitePoint" => {
                    let input = (inputs.number("Input Temperature")?, inputs.number("Input Tint")?);
                    let output = (inputs.number("Output Temperature")?, inputs.number("Output Tint")?);
                    color::Balance::WhitePoint(color::white_point(input, output))
                }
                "offsetPowerSlope" => color::Balance::OffsetPowerSlope {
                    offset: pair("Base Offset", "Color Offset")?,
                    power: pair("Base Power", "Color Power")?,
                    slope: pair("Base Slope", "Color Slope")?,
                },
                _ => color::Balance::LiftGammaGain {
                    lift: pair("Base Lift", "Color Lift")?,
                    gamma: pair("Base Gamma", "Color Gamma")?,
                    gain: pair("Base Gain", "Color Gain")?,
                },
            };
            one("Image", per_pixel(&[&image, &fac], count, |values| color::balance(values.pixel(0), values.number(1), &balance)))
        }
        "rgbCurves" => {
            let image = inputs.plane("Image", Kind::Color)?;
            let fac = inputs.plane("Fac", Kind::Float)?;
            let black = inputs.color("Black Level")?;
            let white = inputs.color("White Level")?;
            let curves = color::Curves::from_json(inputs.data("curves"));
            one("Image", per_pixel(&[&image, &fac], count, |values| curves.apply(values.pixel(0), values.number(1), black, white)))
        }
        "mixColor" => {
            let fac = inputs.plane("Factor", Kind::Float)?;
            let a = inputs.plane("A", Kind::Color)?;
            let b = inputs.plane("B", Kind::Color)?;
            let blend = color::Blend::parse(inputs.setting("blendType"));
            let clamp_factor = inputs.setting("clampFactor") == "true";
            let clamp_result = inputs.setting("clampResult") == "true";
            one(
                "Result",
                per_pixel(&[&fac, &a, &b], count, |values| {
                    let fac = if clamp_factor { values.number(0).clamp(0.0, 1.0) } else { values.number(0) };
                    let mixed = color::mix(blend, fac, values.pixel(1), values.pixel(2));
                    if clamp_result { [mixed[0].clamp(0.0, 1.0), mixed[1].clamp(0.0, 1.0), mixed[2].clamp(0.0, 1.0), mixed[3]] } else { mixed }
                }),
            )
        }
        "alphaOver" => {
            let background = inputs.plane("Background", Kind::Color)?;
            let foreground = inputs.plane("Foreground", Kind::Color)?;
            let fac = inputs.plane("Fac", Kind::Float)?;
            let straight = inputs.flag("Straight Alpha")?;
            let how = color::Over::parse(inputs.setting("Type"));
            one(
                "Image",
                per_pixel(&[&background, &foreground, &fac], count, |values| {
                    color::alpha_over(how, values.pixel(0), values.pixel(1), values.number(2), straight)
                }),
            )
        }
        "blur" => {
            let image = inputs.image("Image")?;
            let size = inputs.vector("Size")?;
            let filter = filter::Filter::parse(inputs.setting("Type"));
            let extend = inputs.flag("Extend Bounds")?;
            one("Image", picture_plane(filter::blur_bounded(&image, filter, size[0], size[1], extend)))
        }
        "defocus" => {
            let image = inputs.image("Image")?;
            let depth = inputs.numbers("Z")?;
            let most = inputs.number("Max Blur")?.clamp(0.0, blur::MOST);
            let radii: Vec<f32> = if inputs.flag("Use Z-Buffer")? {
                let lens = inputs.run.passes.lens.unwrap_or(Lens::DEFAULT);
                let f_stop = inputs.number("F-Stop")?;
                depth.iter().map(|z| blur::circle_of_confusion(*z, &lens, f_stop, width).min(most)).collect()
            } else {
                let scale = inputs.number("Z Scale")?;
                depth.iter().map(|z| (z * scale).clamp(0.0, most)).collect()
            };
            let angle = inputs.number("Angle")?;
            let bokeh = blur::Bokeh::parse(inputs.setting("Bokeh"), angle);
            one("Image", picture_plane(blur::defocus(&image, &radii, &bokeh)))
        }
        "vectorBlur" => {
            let image = inputs.image("Image")?;
            let speed = match inputs.raw("Speed")? {
                Plane::Single(value) => vec![value; count],
                Plane::Pixels(pixels) => Arc::unwrap_or_clone(pixels),
            };
            let depth = inputs.numbers("Z")?;
            let samples = inputs.integer("Samples")?.clamp(1, 256) as u32;
            let shutter = inputs.number("Shutter")?;
            one("Image", picture_plane(blur::vector_blur(&image, &speed, &depth, shutter, samples)))
        }
        "kuwahara" => {
            let image = inputs.image("Image")?;
            let size = inputs.number("Size")?;
            let filtered = if inputs.setting("Type") == "classic" {
                kuwahara::classic(&image, size)
            } else {
                let settings = kuwahara::Anisotropic {
                    size,
                    uniformity: inputs.number("Uniformity")?,
                    sharpness: inputs.number("Sharpness")?,
                    eccentricity: inputs.number("Eccentricity")?,
                };
                kuwahara::anisotropic(&image, &settings)
            };
            one("Image", picture_plane(filtered))
        }
        "translate" | "rotate" | "scale" | "imageTransform" => {
            let image = inputs.image("Image")?;
            let sampler = inputs.sampler();
            let mut placement = transform::Placement::IDENTITY;
            match inputs.node.kind.as_str() {
                "translate" => placement.offset = [inputs.number("X")?, inputs.number("Y")?],
                "rotate" => placement.angle = inputs.number("Angle")?,
                "scale" => {
                    let (x, y) = (inputs.number("X")?, inputs.number("Y")?);
                    placement.scale = match inputs.setting("Type") {
                        "absolute" => [x / width as f32, y / height as f32],
                        // Pictures already have the render's size.
                        "sceneSize" | "renderSize" => [1.0, 1.0],
                        _ => [x, y],
                    };
                }
                _ => {
                    let scale = inputs.number("Scale")?;
                    placement = transform::Placement {
                        offset: [inputs.number("X")?, inputs.number("Y")?],
                        angle: inputs.number("Angle")?,
                        scale: [scale, scale],
                    };
                }
            }
            one("Image", picture_plane(transform::place(&image, &placement, &sampler)))
        }
        "flip" => {
            let image = inputs.image("Image")?;
            let (x, y) = (inputs.flag("Flip X")?, inputs.flag("Flip Y")?);
            one("Image", picture_plane(transform::flip(&image, x, y)))
        }
        "crop" => {
            let image = inputs.image("Image")?;
            let crop = transform::Crop {
                x: inputs.integer("X")?,
                y: inputs.integer("Y")?,
                width: inputs.integer("Width")?,
                height: inputs.integer("Height")?,
                alpha: inputs.flag("Alpha Crop")?,
            };
            one("Image", picture_plane(transform::crop(&image, &crop)))
        }
        "luminanceKey" | "differenceKey" | "distanceKey" | "colorKey" | "chromaKey" => {
            let kind = inputs.node.kind.clone();
            let image = inputs.plane(if kind == "differenceKey" { "Image 1" } else { "Image" }, Kind::Color)?;
            let matte = match kind.as_str() {
                "luminanceKey" => {
                    let (minimum, maximum) = (inputs.number("Minimum")?, inputs.number("Maximum")?);
                    per_pixel(&[&image], count, |values| {
                        let value = keying::luminance(values.pixel(0), minimum, maximum);
                        [value, value, value, 1.0]
                    })
                }
                "differenceKey" => {
                    let other = inputs.plane("Image 2", Kind::Color)?;
                    let (tolerance, falloff) = (inputs.number("Tolerance")?, inputs.number("Falloff")?);
                    per_pixel(&[&image, &other], count, |values| {
                        let value = keying::difference(values.pixel(0), values.pixel(1), tolerance, falloff);
                        [value, value, value, 1.0]
                    })
                }
                "distanceKey" => {
                    let key = inputs.plane("Key Color", Kind::Color)?;
                    let (tolerance, falloff) = (inputs.number("Tolerance")?, inputs.number("Falloff")?);
                    let ycc = inputs.setting("Color Space") == "ycc";
                    per_pixel(&[&image, &key], count, |values| {
                        let value = keying::distance(values.pixel(0), values.pixel(1), ycc, tolerance, falloff);
                        [value, value, value, 1.0]
                    })
                }
                "colorKey" => {
                    let key = inputs.plane("Key Color", Kind::Color)?;
                    let (hue, saturation, value) = (inputs.number("Hue")?, inputs.number("Saturation")?, inputs.number("Value")?);
                    per_pixel(&[&image, &key], count, |values| {
                        let matte = keying::color(values.pixel(0), values.pixel(1), hue, saturation, value);
                        [matte, matte, matte, 1.0]
                    })
                }
                _ => {
                    let key = inputs.plane("Key Color", Kind::Color)?;
                    let (minimum, maximum, falloff) = (inputs.number("Minimum")?, inputs.number("Maximum")?, inputs.number("Falloff")?);
                    per_pixel(&[&image, &key], count, |values| {
                        let matte = keying::chroma(values.pixel(0), values.pixel(1), minimum, maximum, falloff);
                        [matte, matte, matte, 1.0]
                    })
                }
            };
            let keyed = per_pixel(&[&image, &matte], count, |values| keying::keyed(values.pixel(0), values.number(1)));
            Ok(vec![("Image".into(), keyed), ("Matte".into(), matte)])
        }
        "keying" => {
            let image = inputs.image("Image")?;
            let garbage = inputs.numbers("Garbage Matte")?;
            let core = inputs.numbers("Core Matte")?;
            let pixels = |value: i64| value.clamp(-(keying::MOST as i64), keying::MOST as i64) as i32;
            let settings = keying::Keying {
                key: inputs.color("Key Color")?,
                pre_blur: pixels(inputs.integer("Preprocess Blur Size")?),
                balance: inputs.number("Key Balance")?,
                black: inputs.number("Black Level")?,
                white: inputs.number("White Level")?,
                edge_size: pixels(inputs.integer("Edge Search Size")?),
                edge_tolerance: inputs.number("Edge Tolerance")?,
                post_blur: pixels(inputs.integer("Postprocess Blur Size")?),
                dilate: pixels(inputs.integer("Postprocess Dilate Size")?),
                feather: pixels(inputs.integer("Postprocess Feather Size")?),
                falloff: keying::Falloff::parse(inputs.setting("Feather Falloff")),
                despill: inputs.number("Despill Strength")?,
                despill_balance: inputs.number("Despill Balance")?,
            };
            let (keyed, matte, edges) = keying::keying(&image, &garbage, &core, &settings);
            let gray = |values: Vec<f32>| Plane::Pixels(Arc::new(values.into_iter().map(|value| [value, value, value, 1.0]).collect()));
            Ok(vec![("Image".into(), picture_plane(keyed)), ("Matte".into(), gray(matte)), ("Edges".into(), gray(edges))])
        }
        "glare" => {
            let image = inputs.image("Image")?;
            let settings = glare::Settings::read(inputs)?;
            let kernel = if settings.kind == glare::GlareKind::Kernel {
                let color = inputs.setting("Kernel Data Type") == "color";
                match inputs.plane(if color { "Color Kernel" } else { "Float Kernel" }, Kind::Color)? {
                    // One value everywhere: a kernel of one pixel.
                    Plane::Single(_) => convolve::Kernel::identity(),
                    Plane::Pixels(pixels) => {
                        let picture = Image { width, height, pixels: Arc::unwrap_or_clone(pixels) };
                        convolve::Kernel::from_picture(&picture, |pixel| {
                            if color {
                                [pixel[0], pixel[1], pixel[2]]
                            } else {
                                let luma = pixel[0] * LUMINANCE[0] + pixel[1] * LUMINANCE[1] + pixel[2] * LUMINANCE[2];
                                [luma; 3]
                            }
                        })
                    }
                }
            } else {
                convolve::Kernel::identity()
            };
            let (result, glare, highlights) = glare::glare(&image, &settings, &kernel);
            Ok(vec![
                ("Image".into(), picture_plane(result)),
                ("Glare".into(), picture_plane(glare)),
                ("Highlights".into(), picture_plane(highlights)),
            ])
        }
        "ellipseMask" => {
            let mask = inputs.plane("Mask", Kind::Float)?;
            let value = inputs.plane("Value", Kind::Float)?;
            let ellipse = shape::Ellipse {
                position: inputs.vector("Position")?,
                size: inputs.vector("Size")?,
                rotation: inputs.number("Rotation")?,
                operation: shape::Operation::parse(inputs.setting("Operation")),
            };
            one("Mask", shape::ellipse_mask(&ellipse, &mask, &value, width, height))
        }
        "lensDistortion" => {
            let image = inputs.image("Image")?;
            let horizontal = inputs.setting("Type") == "horizontal";
            let lens = shape::Lens {
                horizontal,
                distortion: if horizontal { 0.0 } else { inputs.number("Distortion")? },
                dispersion: inputs.number("Dispersion")?,
                fit: !horizontal && inputs.flag("Fit")?,
                jitter: inputs.flag("Jitter")?,
            };
            one("Image", picture_plane(shape::lens_distortion(&image, &lens)))
        }
        other => Err(format!("the compositor cannot run {other} nodes")),
    }
}

/// The Render Layers outputs a renderer's passes fill, by name (`Layers::passes`).
pub const RENDER_PASSES: &[&str] = &[
    "Normal",
    "Vector",
    "Diffuse Direct",
    "Diffuse Indirect",
    "Diffuse Color",
    "Glossy Direct",
    "Glossy Indirect",
    "Glossy Color",
    "Emission",
    "Environment",
    "Ambient Occlusion",
];

/// Whether `group` reads any of the render's passes beyond its picture, alpha, and depth (the
/// renderer then makes them), or has a Cryptomatte node.
pub fn uses_passes(group: &NodeGroup) -> bool {
    group.nodes.iter().any(|node| node.kind == "cryptomatte")
        || group.links.iter().any(|link| {
            group.node(link.from).is_some_and(|node| node.kind == "renderLayers") && !matches!(link.from_socket.as_str(), "Image" | "Alpha" | "Depth")
        })
}

/// Whether `group` reads one of the light passes only a path-traced render makes.
pub fn uses_light_passes(group: &NodeGroup) -> bool {
    group.links.iter().any(|link| {
        group.node(link.from).is_some_and(|node| node.kind == "renderLayers")
            && (link.from_socket.starts_with("Diffuse") || link.from_socket.starts_with("Glossy") || link.from_socket == "Emission")
    })
}

/// The depth the reference gives pixels where nothing was drawn.
pub const BACKGROUND_DEPTH: f32 = 1.0e10;

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::node_tree::Link;

    /// A picture 53 × 37 in a 48 × 32 render: the reference's lower left pixel shows the
    /// picture's pixel (3, 3) from its lower left (measured).
    #[test]
    fn a_larger_odd_picture_rounds_down_from_the_lower_left() {
        let source =
            Image { width: 53, height: 37, pixels: (0..37).flat_map(|row| (0..53).map(move |x| [x as f32, (36 - row) as f32, 0.0, 1.0])).collect() };
        let nodes = vec![
            Node { id: 1, kind: "image".into(), location: [0.0, 0.0], values: BTreeMap::from([("Image".to_owned(), Value::from("Big"))]) },
            Node { id: 2, kind: "compositorOutput".into(), location: [0.0, 0.0], values: BTreeMap::new() },
        ];
        let links = vec![Link { from: 1, from_socket: "Image".into(), to: 2, to_socket: "Image".into() }];
        let group = NodeGroup { id: 3, name: "Case".into(), tree: Tree::Compositor, inputs: Vec::new(), outputs: Vec::new(), nodes, links };
        let render = Image::filled(48, 32, [0.0; 4]);
        let images = |_: &str| Some(source.clone());
        let passes = Passes { image: &render, depth: None, images: &images, layers: None, lens: None };
        let out = composite(&group, &passes).ok().and_then(|out| out.image).unwrap_or_else(|| render.clone());
        let lower_left = out.pixels.get(31 * 48).copied().unwrap_or_default();
        let upper_right = out.pixels.get(47).copied().unwrap_or_default();
        assert_eq!((lower_left[0], lower_left[1]), (3.0, 3.0));
        assert_eq!((upper_right[0], upper_right[1]), (50.0, 34.0));
    }
}
