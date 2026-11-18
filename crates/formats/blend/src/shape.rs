//! Curve and text objects: their splines (Bézier, poly, and NURBS) with handles, radius, and
//! tilt, the bevel, extrusion, offset, and fill settings, and a text's body and layout. Fonts
//! other than the bundled one set in it.

use kition_scene_model::curve::{Dimensions, FillMode};
use kition_scene_model::glam::Vec3;
use kition_scene_model::{BezierPoint, ControlPoint, Curve, HandleType, ObjectData, SceneError, Spline, Text, TextAlign};

use crate::file::{File, View, invalid};

/// The most splines and points read from one curve.
const MAX_SPLINES: usize = 10_000;
const MAX_POINTS: usize = 1_000_000;

/// The curve flags: 3D, front, back, and caps.
const FLAG_3D: i64 = 1;
const FLAG_FRONT: i64 = 2;
const FLAG_BACK: i64 = 4;
const FLAG_CAPS: i64 = 1 << 14;

fn float(view: &View<'_>, field: &str, default: f32) -> f32 {
    view.f32(field).filter(|value| value.is_finite()).unwrap_or(default)
}

fn resolution(value: Option<i64>) -> u32 {
    u32::try_from(value.unwrap_or(12).clamp(1, 1024)).unwrap_or(12)
}

fn handle(value: Option<i64>) -> HandleType {
    match value.unwrap_or(1) {
        0 => HandleType::Free,
        2 => HandleType::Vector,
        3 => HandleType::Aligned,
        _ => HandleType::Auto,
    }
}

/// The splines of a curve's `nurb` list.
fn splines(file: &File, curve: &View<'_>, default_resolution: u32) -> Result<Vec<Spline>, SceneError> {
    let mut out = Vec::new();
    let mut points_read = 0usize;
    let mut next = curve.nested("nurb").and_then(|list| list.target("first"));
    while let Some(block) = next {
        if out.len() >= MAX_SPLINES {
            return Err(invalid("a curve has too many splines"));
        }
        let nurb = file.view_in("Nurb", block, 0).ok_or_else(|| invalid("a spline has no layout"))?;
        next = nurb.target("next");
        let count = usize::try_from(nurb.int("pntsu").unwrap_or(0)).unwrap_or(0);
        points_read = points_read.saturating_add(count);
        if count == 0 {
            continue;
        }
        if points_read > MAX_POINTS {
            return Err(invalid("a curve has too many points"));
        }
        let flags = nurb.int("flagu").unwrap_or(0);
        let cyclic = flags & 1 != 0;
        let spline_resolution = nurb.int("resolu").map_or(default_resolution, |value| resolution(Some(value)));
        let spline = match nurb.int("type").unwrap_or(0) {
            1 => {
                let block = nurb.target("bezt").ok_or_else(|| invalid("a Bézier spline has no points"))?;
                let mut points = Vec::with_capacity(count);
                for slot in 0..count {
                    let point = file.view_in("BezTriple", block, slot).ok_or_else(|| invalid("a Bézier spline is cut short"))?;
                    let vec = point.floats("vec").unwrap_or_default();
                    let at = |index: usize| {
                        Vec3::new(
                            vec.get(index * 3).copied().unwrap_or(0.0),
                            vec.get(index * 3 + 1).copied().unwrap_or(0.0),
                            vec.get(index * 3 + 2).copied().unwrap_or(0.0),
                        )
                    };
                    points.push(BezierPoint {
                        co: at(1),
                        left: at(0),
                        right: at(2),
                        left_type: handle(point.int("h1")),
                        right_type: handle(point.int("h2")),
                        radius: float(&point, "radius", 1.0),
                        tilt: float(&point, "alfa", 0.0),
                    });
                }
                Spline::Bezier { points, cyclic, resolution: spline_resolution }
            }
            kind => {
                let block = nurb.target("bp").ok_or_else(|| invalid("a spline has no points"))?;
                let mut points = Vec::with_capacity(count);
                for slot in 0..count {
                    let point = file.view_in("BPoint", block, slot).ok_or_else(|| invalid("a spline is cut short"))?;
                    let vec = point.floats("vec").unwrap_or_default();
                    let value = |index: usize, default: f32| vec.get(index).copied().filter(|value| value.is_finite()).unwrap_or(default);
                    points.push(ControlPoint {
                        co: Vec3::new(value(0, 0.0), value(1, 0.0), value(2, 0.0)),
                        weight: value(3, 1.0),
                        radius: float(&point, "radius", 1.0),
                        tilt: float(&point, "alfa", 0.0),
                    });
                }
                if kind == 4 {
                    let order = u32::try_from(nurb.int("orderu").unwrap_or(4).clamp(2, 6)).unwrap_or(4);
                    let order = order.min(u32::try_from(count.max(2)).unwrap_or(2));
                    if points.iter().any(|point| point.weight <= 0.0) {
                        return Err(invalid("a NURBS point has no weight"));
                    }
                    Spline::Nurbs { points, cyclic, order, endpoint: flags & 2 != 0, resolution: spline_resolution }
                } else {
                    Spline::Poly { points, cyclic }
                }
            }
        };
        out.push(spline);
    }
    Ok(out)
}

/// A curve object's data.
pub fn read_curve(file: &File, curve: &View<'_>) -> Result<ObjectData, SceneError> {
    let flag = curve.int("flag").unwrap_or(FLAG_3D);
    let flat = flag & FLAG_3D == 0;
    let (front, back) = (flag & FLAG_FRONT != 0, flag & FLAG_BACK != 0);
    let fill_mode = match (flat, front, back) {
        (false, false, false) => FillMode::Full,
        // A spatial curve filled on one side is closest to the half profile.
        (false, _, _) => FillMode::Half,
        (true, true, true) => FillMode::Both,
        (true, true, false) => FillMode::Front,
        (true, false, true) => FillMode::Back,
        (true, false, false) => FillMode::None,
    };
    let default_resolution = resolution(curve.int("resolu"));
    let parsed = Curve {
        splines: splines(file, curve, default_resolution)?,
        dimensions: if flat { Dimensions::Flat } else { Dimensions::Spatial },
        fill_mode,
        bevel_depth: float(curve, "ext2", 0.0).max(0.0),
        bevel_resolution: u32::try_from(curve.int("bevresol").unwrap_or(4).clamp(0, 64)).unwrap_or(4),
        extrude: float(curve, "ext1", 0.0).max(0.0),
        // The file keeps one plus the offset.
        offset: float(curve, "width", 1.0) - 1.0,
        fill_caps: flag & FLAG_CAPS != 0,
        resolution: default_resolution,
        // Bevel and taper objects and the taper radius mode are not read from the file yet.
        ..Curve::default()
    };
    parsed.validate()?;
    Ok(ObjectData::Curve(parsed))
}

/// A text object's data.
pub fn read_text(file: &File, curve: &View<'_>) -> Result<ObjectData, SceneError> {
    let body = curve
        .target("str")
        .map(|block| {
            let bytes = file.bytes(block);
            let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned()
        })
        .unwrap_or_default();
    let body: String =
        body.chars().filter(|character| *character == '\n' || !character.is_control()).take(kition_scene_model::text::MAX_LENGTH).collect();
    let font = curve.follow("vfont").and_then(|font| font.text("name")).filter(|name| !name.is_empty() && name != "<builtin>");
    let font = font
        .map(|name| {
            // The file names the font's path; its name is the file's stem.
            let stem = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();
            stem.split('.').next().unwrap_or(&stem).to_string()
        })
        .unwrap_or_else(|| kition_scene_model::text::DEFAULT_FONT.to_string());
    let text = Text {
        body,
        size: float(curve, "fsize", 1.0),
        extrude: float(curve, "ext1", 0.0).max(0.0),
        bevel_depth: float(curve, "ext2", 0.0).max(0.0),
        bevel_resolution: u32::try_from(curve.int("bevresol").unwrap_or(4).clamp(0, 64)).unwrap_or(4),
        align: match curve.int("spacemode").unwrap_or(0) {
            1 => TextAlign::Center,
            2 => TextAlign::Right,
            3 => TextAlign::Justify,
            4 => TextAlign::Flush,
            _ => TextAlign::Left,
        },
        line_spacing: float(curve, "linedist", 1.0),
        character_spacing: float(curve, "spacing", 1.0),
        word_spacing: float(curve, "wordspace", 1.0),
        offset_x: float(curve, "xof", 0.0),
        offset_y: float(curve, "yof", 0.0),
        shear: float(curve, "shear", 0.0),
        resolution: resolution(curve.int("resolu")),
        font,
        // The vertical alignment, text boxes, overflow and follow curve are not read from the file yet.
        ..Text::default()
    };
    text.validate()?;
    Ok(ObjectData::Text(text))
}
