//! Text objects: a body of text set in a font (Inter, bundled with the engine), made into a mesh
//! the way the reference application does it — glyph outlines flattened with `resolution` points
//! per curved segment (straight segments are not divided), filled, and optionally extruded and
//! bevelled. Sizes follow the reference: a size of 1 maps the font's whole box (its highest
//! ascender to its lowest descender) to one unit, and lines are `size × line spacing` apart.

use std::sync::OnceLock;

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::outline;
use crate::scene::Id;

/// Inter (OFL-1.1, `fonts/inter/OFL.txt`), the font text objects are set in.
static INTER: &[u8] = include_bytes!("../fonts/inter/Inter-Variable.ttf");

/// The name of the bundled font.
pub const DEFAULT_FONT: &str = "Inter";

/// The optical size the reference reads Inter at, whatever the text's size: its letters' sidebearings,
/// stems and advances all match the font's `opsz` axis at 16 (measured on single glyphs; 14, the
/// font's default, is a few units wider), at the default weight.
const OPTICAL_SIZE: f32 = 16.0;

fn face() -> Option<&'static ttf_parser::Face<'static>> {
    static FACE: OnceLock<Option<ttf_parser::Face<'static>>> = OnceLock::new();
    FACE.get_or_init(|| {
        let mut face = ttf_parser::Face::parse(INTER, 0).ok()?;
        face.set_variation(ttf_parser::Tag::from_bytes(b"opsz"), OPTICAL_SIZE)?;
        Some(face)
    })
    .as_ref()
}

/// Horizontal alignment of each line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    /// In a text box: wrapped lines widen their spaces to fill the box.
    Justify,
    /// In a text box: every line spreads its characters to fill the box.
    Flush,
}

/// Where the lines sit against the text box's top and bottom (the origin without a box height).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextVerticalAlign {
    /// The first line's baseline at the origin.
    #[default]
    TopBaseline,
    /// The first line's ascender at the top.
    Top,
    /// Halfway between Top and Bottom.
    Center,
    /// The last line's descender at the bottom.
    Bottom,
    /// The last line's baseline at the bottom.
    BottomBaseline,
}

impl TextVerticalAlign {
    fn is_default(&self) -> bool {
        *self == Self::TopBaseline
    }
}

/// What happens to text taller than its box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextOverflow {
    /// It runs past the box.
    #[default]
    None,
    /// It shrinks until its lines fit.
    Scale,
    /// Lines that would reach below the box are dropped.
    Truncate,
}

impl TextOverflow {
    fn is_default(&self) -> bool {
        *self == Self::None
    }
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

fn one() -> f32 {
    1.0
}

fn twelve() -> u32 {
    12
}

fn four() -> u32 {
    4
}

fn inter() -> String {
    DEFAULT_FONT.to_string()
}

/// A text object's data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    pub body: String,
    #[serde(default = "one")]
    pub size: f32,
    #[serde(default)]
    pub extrude: f32,
    #[serde(default)]
    pub bevel_depth: f32,
    #[serde(default = "four")]
    pub bevel_resolution: u32,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default = "one")]
    pub line_spacing: f32,
    #[serde(default = "one")]
    pub character_spacing: f32,
    #[serde(default = "one")]
    pub word_spacing: f32,
    #[serde(default)]
    pub offset_x: f32,
    #[serde(default)]
    pub offset_y: f32,
    /// Slants the letters: x moves by `shear × y`.
    #[serde(default)]
    pub shear: f32,
    #[serde(default = "twelve")]
    pub resolution: u32,
    /// The font's name; only the bundled one is available, and any other name sets in it.
    #[serde(default = "inter")]
    pub font: String,
    #[serde(default, skip_serializing_if = "TextVerticalAlign::is_default")]
    pub align_y: TextVerticalAlign,
    /// The text box's width: lines wrap at spaces to fit it (0: no wrapping).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub box_width: f32,
    /// The text box's height, for vertical alignment and overflow (0: none).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub box_height: f32,
    /// Where the text box starts, in the object's own space.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub box_x: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub box_y: f32,
    #[serde(default, skip_serializing_if = "TextOverflow::is_default")]
    pub overflow: TextOverflow,
    /// A curve object the letters follow: each sits on the curve's path (in that object's own
    /// space, scaled by its scale) at its distance along the line, turned to the path's
    /// direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_curve: Option<Id>,
}

impl Default for Text {
    fn default() -> Self {
        Self {
            body: "Text".into(),
            size: 1.0,
            extrude: 0.0,
            bevel_depth: 0.0,
            bevel_resolution: 4,
            align: TextAlign::Left,
            line_spacing: 1.0,
            character_spacing: 1.0,
            word_spacing: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            shear: 0.0,
            resolution: 12,
            font: inter(),
            align_y: TextVerticalAlign::TopBaseline,
            box_width: 0.0,
            box_height: 0.0,
            box_x: 0.0,
            box_y: 0.0,
            overflow: TextOverflow::None,
            follow_curve: None,
        }
    }
}

/// The longest body a text object takes.
pub const MAX_LENGTH: usize = 10_000;

/// One piece of a glyph's contour, as the font gives it: where it ends, after its control points.
#[derive(Clone, Copy)]
enum Segment {
    Line(Vec2),
    Quad(Vec2, Vec2),
    Cubic(Vec2, Vec2, Vec2),
}

impl Segment {
    fn end(self) -> Vec2 {
        match self {
            Self::Line(end) | Self::Quad(_, end) | Self::Cubic(_, _, end) => end,
        }
    }
}

/// Flattens glyph outlines into contours.
///
/// The points sit where the reference puts them: at the optical size, each point the font stores
/// lands on the font's unit grid (a stem edge at 176.4 units reads as 176), while an on-curve
/// point the font leaves out between two control points stays halfway between them, so it may
/// fall on a half unit (an `o` at 874.5).
struct Flattener {
    contours: Vec<Vec<Vec2>>,
    start: Vec2,
    segments: Vec<Segment>,
    resolution: u32,
}

/// A point the font stores, on the unit grid.
fn grid(point: Vec2) -> Vec2 {
    point.round()
}

impl Flattener {
    fn new(resolution: u32) -> Self {
        Self { contours: Vec::new(), start: Vec2::ZERO, segments: Vec::new(), resolution: resolution.max(1) }
    }

    /// Where segment `index` ends on the grid: halfway between its control point and the next
    /// one when the font left that on-curve point out, else on the grid.
    fn snapped_end(&self, index: usize) -> Vec2 {
        let count = self.segments.len();
        let Some(segment) = self.segments.get(index).copied() else { return grid(self.start) };
        let next = self.segments.get((index + 1) % count.max(1)).copied();
        if let (Segment::Quad(control, end), Some(Segment::Quad(next_control, _))) = (segment, next)
            && end.distance((control + next_control) / 2.0) < 1e-3
        {
            return (grid(control) + grid(next_control)) / 2.0;
        }
        grid(segment.end())
    }

    fn finish(&mut self) {
        if self.segments.is_empty() {
            return;
        }
        let count = self.segments.len();
        let closes = self.segments.last().is_some_and(|last| last.end().distance(self.start) < 1e-3);
        let mut previous = if closes { self.snapped_end(count - 1) } else { grid(self.start) };
        let mut contour = vec![previous];
        let steps = self.resolution;
        for index in 0..count {
            let end = self.snapped_end(index);
            match self.segments.get(index).copied() {
                Some(Segment::Quad(control, _)) => {
                    let control = grid(control);
                    for step in 1..steps {
                        let t = step as f32 / steps as f32;
                        let u = 1.0 - t;
                        contour.push(previous * (u * u) + control * (2.0 * u * t) + end * (t * t));
                    }
                }
                Some(Segment::Cubic(first, second, _)) => {
                    let (first, second) = (grid(first), grid(second));
                    for step in 1..steps {
                        let t = step as f32 / steps as f32;
                        let point = crate::curve::cubic(previous.extend(0.0), first.extend(0.0), second.extend(0.0), end.extend(0.0), t);
                        contour.push(point.truncate());
                    }
                }
                Some(Segment::Line(_)) | None => {}
            }
            contour.push(end);
            previous = end;
        }
        self.segments.clear();
        if contour.len() > 1 && contour.first() == contour.last() {
            contour.pop();
        }
        if contour.len() >= 3 {
            self.contours.push(contour);
        }
    }
}

impl ttf_parser::OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        self.start = Vec2::new(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.segments.push(Segment::Line(Vec2::new(x, y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.segments.push(Segment::Quad(Vec2::new(x1, y1), Vec2::new(x, y)));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.segments.push(Segment::Cubic(Vec2::new(x1, y1), Vec2::new(x2, y2), Vec2::new(x, y)));
    }

    fn close(&mut self) {
        self.finish();
    }
}

/// One glyph set on the page: its contours in object space.
struct Placed {
    contours: Vec<Vec<Vec2>>,
}

/// One character on a line: where its pen starts and how far it moves, with its glyph's
/// outline relative to the pen (unscaled by spacing).
#[derive(Debug, Clone)]
struct Slot {
    /// Its index in the body.
    index: usize,
    character: char,
    pen: f32,
    advance: f32,
    contours: Vec<Vec<Vec2>>,
}

/// A laid out line.
#[derive(Debug, Clone)]
struct Line {
    slots: Vec<Slot>,
    /// The index the line ends at (where the caret goes after its last character).
    end: usize,
    baseline: f32,
    /// Ended by wrapping, not by a line break or the end.
    wrapped: bool,
}

impl Line {
    /// From its first pen to its last letter's advance, wherever alignment has moved it.
    fn width(&self) -> f32 {
        let left = self.slots.first().map_or(0.0, |slot| slot.pen);
        self.slots.last().map_or(0.0, |slot| slot.pen + slot.advance - left)
    }
}

/// A path text follows: its evaluated points (3D, for distances along it; a closed path ends on
/// its first point again) and their running distances.
///
/// As measured in the reference: each letter's pen lands on the path at its distance along it,
/// the path read as straight pieces between its evaluated points; the letter turns to the path's
/// direction at its middle (pen plus half its advance), and that direction is smooth across the
/// points. It follows the derivative of a uniform cubic B-spline over the evaluated points: within
/// the piece from point `i` to `i + 1`, at fraction `f` of its length, the pieces before, at and
/// after it (as vectors, so longer pieces weigh more) weigh `(1 - f)² / 2`, `(1 + 2f - 2f²) / 2`
/// and `f² / 2`. An open path has no piece before its first or after its last; a closed one
/// wraps round. A closed Bézier path starts at its last control point, a closed poly at its
/// first.
pub struct Path {
    points: Vec<Vec3>,
    distances: Vec<f32>,
    cyclic: bool,
}

impl Path {
    /// The path along a curve's first spline, scaled by `scale`; `None` without two points.
    pub fn of(curve: &crate::curve::Curve, scale: Vec3) -> Option<Self> {
        let (positions, _) = curve.path()?;
        let spline = curve.splines.first()?;
        let cyclic = spline.cyclic() && spline.len() > 2;
        let mut points: Vec<Vec3> = positions.into_iter().map(|point| point * scale).collect();
        if let (true, crate::curve::Spline::Bezier { points: controls, .. }) = (cyclic, spline) {
            // Start at the sample on the last control point; the closing point follows again.
            points.pop();
            let last = controls.last().map_or(Vec3::ZERO, |point| point.co * scale);
            let start = points
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.distance_squared(last).total_cmp(&b.distance_squared(last)))
                .map_or(0, |(index, _)| index);
            points.rotate_left(start);
            if let Some(first) = points.first().copied() {
                points.push(first);
            }
        }
        let mut distances = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                total += points.get(index - 1).map_or(0.0, |previous| previous.distance(*point));
            }
            distances.push(total);
        }
        (total > 1e-9).then_some(Self { points, distances, cyclic })
    }

    fn length(&self) -> f32 {
        self.distances.last().copied().unwrap_or(0.0)
    }

    /// The piece `distance` along lies on, and how far along that piece (0 to 1).
    fn piece(&self, distance: f32) -> (usize, f32) {
        let distance = distance.clamp(0.0, self.length());
        let index = self.distances.windows(2).position(|pair| distance <= pair.get(1).copied().unwrap_or(0.0)).unwrap_or(0);
        let (start, end) = (self.distances.get(index).copied().unwrap_or(0.0), self.distances.get(index + 1).copied().unwrap_or(0.0));
        (index, if end > start { (distance - start) / (end - start) } else { 0.0 })
    }

    /// The piece `index` as a vector: wrapping round a closed path, nothing beyond an open one's
    /// ends.
    fn vector(&self, index: isize) -> Vec3 {
        let count = self.points.len().saturating_sub(1);
        let Ok(signed) = isize::try_from(count) else { return Vec3::ZERO };
        if count == 0 {
            return Vec3::ZERO;
        }
        let index = if self.cyclic { index.rem_euclid(signed) } else { index };
        let Ok(index) = usize::try_from(index) else { return Vec3::ZERO };
        match (self.points.get(index), self.points.get(index + 1)) {
            (Some(a), Some(b)) => *b - *a,
            _ => Vec3::ZERO,
        }
    }

    /// The point (in the XY plane) `distance` along.
    fn point(&self, distance: f32) -> Vec2 {
        let (index, t) = self.piece(distance);
        match (self.points.get(index), self.points.get(index + 1)) {
            (Some(a), Some(b)) => a.lerp(*b, t).truncate(),
            (Some(a), None) => a.truncate(),
            _ => Vec2::ZERO,
        }
    }

    /// The path's direction in the XY plane `distance` along.
    fn direction(&self, distance: f32) -> Vec2 {
        let (index, f) = self.piece(distance);
        let Ok(here) = isize::try_from(index) else { return Vec2::X };
        let smooth = self.vector(here - 1) * ((1.0 - f) * (1.0 - f) / 2.0)
            + self.vector(here) * ((1.0 + 2.0 * f - 2.0 * f * f) / 2.0)
            + self.vector(here + 1) * (f * f / 2.0);
        let flat = smooth.truncate().normalize_or_zero();
        if flat != Vec2::ZERO {
            return flat;
        }
        // Seen end on: the nearest piece that is not.
        let count = self.points.len().saturating_sub(1);
        (index..count)
            .chain((0..index).rev())
            .filter_map(|other| isize::try_from(other).ok())
            .map(|other| self.vector(other).truncate().normalize_or_zero())
            .find(|direction| *direction != Vec2::ZERO)
            .unwrap_or(Vec2::X)
    }
}

impl Text {
    /// Checks the text's numbers and length.
    pub fn validate(&self) -> Result<()> {
        let invalid = |message: &str| Err(SceneError::InvalidFile(message.into()));
        if self.body.chars().count() > MAX_LENGTH {
            return invalid("a text body is too long");
        }
        for value in [
            self.size,
            self.extrude,
            self.bevel_depth,
            self.line_spacing,
            self.character_spacing,
            self.word_spacing,
            self.offset_x,
            self.offset_y,
            self.shear,
            self.box_width,
            self.box_height,
            self.box_x,
            self.box_y,
        ] {
            if !value.is_finite() || value.abs() > 1e6 {
                return invalid("a text setting is not a usable number");
            }
        }
        if self.size <= 0.0 || self.extrude < 0.0 || self.bevel_depth < 0.0 || self.box_width < 0.0 || self.box_height < 0.0 {
            return invalid("a text's size must be positive and its extrusion and bevel not negative");
        }
        if !(1..=64).contains(&self.resolution) || self.bevel_resolution > 32 {
            return invalid("a text's resolution is out of range");
        }
        Ok(())
    }

    /// The font's units to object units, its ascender, and its descender (both positive, in
    /// object units) at `size`.
    fn metrics(&self, size: f32) -> Option<(f32, f32, f32)> {
        let face = face()?;
        let bounds = face.global_bounding_box();
        let height = f32::from(bounds.y_max) - f32::from(bounds.y_min);
        if height <= 0.0 {
            return None;
        }
        let scale = size / height;
        Some((scale, f32::from(face.ascender()) * scale, -f32::from(face.descender()) * scale))
    }

    /// The characters of the body, line by line, wrapped to the text box's width, at `size`.
    fn lines(&self, size: f32) -> Vec<Line> {
        let Some(face) = face() else { return Vec::new() };
        let Some((scale, _, _)) = self.metrics(size) else { return Vec::new() };
        let slot_of = |index: usize, character: char| -> Slot {
            let glyph = face.glyph_index(character).or_else(|| face.glyph_index('?'));
            let advance = glyph.and_then(|glyph| face.glyph_hor_advance(glyph)).map_or(0.0, f32::from) * scale;
            let spacing = if character == ' ' { self.word_spacing } else { self.character_spacing };
            let mut contours = Vec::new();
            if let Some(glyph) = glyph {
                let mut flattener = Flattener::new(self.resolution);
                if face.outline_glyph(glyph, &mut flattener).is_some() {
                    flattener.finish();
                    contours = flattener.contours.into_iter().map(|contour| contour.into_iter().map(|point| point * scale).collect()).collect();
                }
            }
            Slot { index, character, pen: 0.0, advance: advance * spacing, contours }
        };
        let mut lines = Vec::new();
        let mut index = 0usize;
        for paragraph in self.body.split('\n') {
            let characters: Vec<(usize, char)> = paragraph.chars().enumerate().map(|(offset, character)| (index + offset, character)).collect();
            let paragraph_end = index + characters.len();
            index = paragraph_end + 1;
            let slots: Vec<Slot> = characters.iter().map(|(index, character)| slot_of(*index, *character)).collect();
            if self.box_width <= 0.0 {
                lines.push(Line { slots, end: paragraph_end, baseline: 0.0, wrapped: false });
                continue;
            }
            // Words, greedily: a word that would pass the box's edge starts a new line, and the
            // space it wrapped at is dropped.
            let width_of = |slots: &[Slot]| slots.iter().map(|slot| slot.advance).sum::<f32>();
            let mut words: Vec<Vec<Slot>> = vec![Vec::new()];
            let mut spaces: Vec<Option<Slot>> = Vec::new();
            for slot in slots {
                if slot.character == ' ' {
                    spaces.push(Some(slot));
                    words.push(Vec::new());
                } else if let Some(word) = words.last_mut() {
                    word.push(slot);
                }
            }
            let mut line: Vec<Slot> = words.first().cloned().unwrap_or_default();
            for (word, space) in words.into_iter().skip(1).zip(spaces) {
                let space_width = space.as_ref().map_or(0.0, |space| space.advance);
                let fits = width_of(&line) + space_width + width_of(&word) <= self.box_width + 1e-6;
                if fits || line.is_empty() {
                    line.extend(space);
                    line.extend(word);
                } else {
                    let end = space.as_ref().map_or(paragraph_end, |space| space.index);
                    lines.push(Line { slots: std::mem::take(&mut line), end, baseline: 0.0, wrapped: true });
                    line = word;
                }
            }
            lines.push(Line { slots: line, end: paragraph_end, baseline: 0.0, wrapped: false });
        }
        for line in &mut lines {
            let mut pen = 0.0;
            for slot in &mut line.slots {
                slot.pen = pen;
                pen += slot.advance;
            }
        }
        lines
    }

    /// The lines at a size, placed: aligned in the box, vertically aligned, and truncated.
    fn placed_lines(&self) -> (Vec<Line>, f32) {
        let mut size = self.size;
        let line_height = |size: f32| size * self.line_spacing;
        let mut lines = self.lines(size);
        if self.overflow == TextOverflow::Scale && self.box_height > 0.0 && lines.len() as f32 * line_height(size) > self.box_height {
            // The largest size whose lines fit: for each line count, the size that fits that
            // many, if the text then wraps into no more.
            let mut best = None;
            for count in 1..=lines.len().max(1) {
                let candidate = (self.box_height / (count as f32 * self.line_spacing.max(1e-6))).min(self.size);
                let wrapped = self.lines(candidate);
                if wrapped.len() <= count {
                    best = Some((candidate, wrapped));
                    break;
                }
            }
            if let Some((fitted, wrapped)) = best {
                size = fitted;
                lines = wrapped;
            }
        }
        let Some((_, ascender, descender)) = self.metrics(size) else { return (Vec::new(), size) };
        let count = lines.len();
        let height = line_height(size);
        let top = if self.box_height > 0.0 { self.size } else { 0.0 };
        let bottom = if self.box_height > 0.0 { top - self.box_height } else { 0.0 };
        let last = count.saturating_sub(1) as f32 * height;
        let to_top = top - ascender;
        let to_bottom = bottom + descender + last;
        let shift = match self.align_y {
            TextVerticalAlign::TopBaseline => 0.0,
            TextVerticalAlign::Top => to_top,
            TextVerticalAlign::Center => (to_top + to_bottom) / 2.0,
            TextVerticalAlign::Bottom => to_bottom,
            TextVerticalAlign::BottomBaseline => bottom + last,
        };
        let boxed = self.box_width > 0.0;
        for (line_index, line) in lines.iter_mut().enumerate() {
            line.baseline = shift - line_index as f32 * height;
            let width = line.width();
            let room = self.box_width - width;
            match self.align {
                TextAlign::Left => {}
                TextAlign::Center => line.slots.iter_mut().for_each(|slot| slot.pen += if boxed { room / 2.0 } else { -width / 2.0 }),
                TextAlign::Right => line.slots.iter_mut().for_each(|slot| slot.pen += if boxed { room } else { -width }),
                TextAlign::Justify if boxed && line.wrapped => {
                    let spaces = line.slots.iter().filter(|slot| slot.character == ' ').count();
                    if spaces > 0 {
                        let extra = room / spaces as f32;
                        let mut moved = 0.0;
                        for slot in &mut line.slots {
                            slot.pen += moved;
                            if slot.character == ' ' {
                                moved += extra;
                            }
                        }
                    }
                }
                TextAlign::Flush if boxed && line.slots.len() > 1 => {
                    let extra = room / (line.slots.len() - 1) as f32;
                    for (index, slot) in line.slots.iter_mut().enumerate() {
                        slot.pen += extra * index as f32;
                    }
                }
                TextAlign::Justify | TextAlign::Flush => {}
            }
        }
        if self.overflow == TextOverflow::Truncate && self.box_height > 0.0 {
            lines.retain(|line| line.baseline - descender >= bottom - 1e-4);
        }
        (lines, size)
    }

    /// The glyphs laid out, line by line; letters following a path when one is given.
    fn layout(&self, path: Option<&Path>) -> Vec<Placed> {
        let (lines, _) = self.placed_lines();
        let mut placed = Vec::new();
        let (x0, y0) = (self.box_x + self.offset_x, self.box_y + self.offset_y);
        for line in &lines {
            let width = line.width();
            // On a path, a line longer than the path is pressed together to fit it.
            let along = path.map(|path| {
                let length = path.length();
                let squeeze = if width > length && width > 0.0 { length / width } else { 1.0 };
                let start = match self.align {
                    TextAlign::Center => (length - width * squeeze) / 2.0,
                    TextAlign::Right => length - width * squeeze,
                    _ => 0.0,
                };
                (squeeze, start)
            });
            for slot in &line.slots {
                let sheared = |point: Vec2| Vec2::new(point.x + point.y * self.shear, point.y);
                let contours: Vec<Vec<Vec2>> = match (path, along) {
                    (Some(path), Some((squeeze, start))) => {
                        // Rebuild the pen from the left-aligned line, so the alignment along the
                        // path is the path's. A squeezed line moves its letters closer; the
                        // letters keep their size.
                        let left = line.slots.first().map_or(0.0, |first| first.pen);
                        let pen = slot.pen - left;
                        let point = path.point(start + pen * squeeze + self.offset_x);
                        let direction = path.direction(start + (pen + slot.advance / 2.0) * squeeze + self.offset_x);
                        let normal = Vec2::new(-direction.y, direction.x);
                        slot.contours
                            .iter()
                            .map(|contour| {
                                contour
                                    .iter()
                                    .map(|glyph_point| {
                                        let local = sheared(Vec2::new(glyph_point.x, glyph_point.y + line.baseline + self.offset_y));
                                        point + direction * local.x + normal * local.y
                                    })
                                    .collect()
                            })
                            .collect()
                    }
                    _ => slot
                        .contours
                        .iter()
                        .map(|contour| {
                            contour
                                .iter()
                                .map(|glyph_point| {
                                    let point = Vec2::new(slot.pen + glyph_point.x, line.baseline + glyph_point.y);
                                    Vec2::new(point.x + point.y * self.shear + x0, point.y + y0)
                                })
                                .collect()
                        })
                        .collect(),
                };
                if !contours.is_empty() {
                    placed.push(Placed { contours });
                }
            }
        }
        placed
    }

    /// How many points the flattened glyph outlines have (before overlapping strokes are merged):
    /// what the reference's text shows as a wire with its fill off.
    pub fn outline_count(&self) -> usize {
        self.layout(None).iter().map(|glyph| glyph.contours.iter().map(Vec::len).sum::<usize>()).sum()
    }

    /// The flattened glyph outlines' points, laid out (on `path` when one is given): what the
    /// reference's text shows as a wire with its fill off.
    pub fn outline_points(&self, path: Option<&Path>) -> Vec<Vec3> {
        self.layout(path).into_iter().flat_map(|glyph| glyph.contours.into_iter().flatten()).map(|point| point.extend(0.0)).collect()
    }

    /// The flattened glyph outlines laid out without a path, contour by contour (closed, the
    /// last point joining the first), in the object's own space with y up.
    pub fn contours(&self) -> Vec<Vec<Vec2>> {
        self.layout(None).into_iter().flat_map(|glyph| glyph.contours).collect()
    }

    /// The text as a mesh, in the object's own space: letters face +Z.
    pub fn to_mesh(&self) -> Mesh {
        self.to_mesh_on(None)
    }

    /// The text as a mesh, its letters following `path` when one is given.
    pub fn to_mesh_on(&self, path: Option<&Path>) -> Mesh {
        let mut mesh = Mesh::default();
        for glyph in self.layout(path) {
            add_glyph(&mut mesh, glyph.contours, self.extrude, self.bevel_depth, self.bevel_resolution);
        }
        mesh
    }
}

/// Outward 2D normals of a contour whose solid side is on its left (counter-clockwise outer
/// contours, clockwise holes), mitred so offsets keep their distance from the edges.
fn outward_offsets(contour: &[Vec2]) -> Vec<Vec2> {
    let count = contour.len();
    (0..count)
        .map(|index| {
            let (Some(previous), Some(here), Some(next)) =
                (contour.get((index + count - 1) % count), contour.get(index), contour.get((index + 1) % count))
            else {
                return Vec2::ZERO;
            };
            let edge_in = (*here - *previous).normalize_or_zero();
            let edge_out = (*next - *here).normalize_or_zero();
            let normal_in = Vec2::new(edge_in.y, -edge_in.x);
            let normal_out = Vec2::new(edge_out.y, -edge_out.x);
            let average = (normal_in + normal_out).normalize_or_zero();
            let cosine = average.dot(normal_out).max(0.25);
            if average == Vec2::ZERO { normal_out } else { average / cosine }
        })
        .collect()
}

/// The contour without points closer than `near` to the point before them.
fn merge_close(contour: &[Vec2], near: f32) -> Vec<Vec2> {
    let mut points: Vec<Vec2> = Vec::with_capacity(contour.len());
    for point in contour {
        if points.last().is_none_or(|last| last.distance(*point) > near) {
            points.push(*point);
        }
    }
    while points.len() > 1 && points.first().zip(points.last()).is_some_and(|(first, last)| first.distance(*last) <= near) {
        points.pop();
    }
    points
}

/// Adds one glyph: its fill, and when it has depth, a back fill and the sides between.
fn add_glyph(mesh: &mut Mesh, contours: Vec<Vec<Vec2>>, extrude: f32, bevel: f32, bevel_resolution: u32) {
    // Variable fonts draw letters from overlapping strokes: their covered region's outline.
    let contours = if outline::crosses(&contours) { outline::resolve(&contours) } else { contours };
    // Where strokes cross just beside a point of the outline (closer than a font's unit grid
    // could tell apart), the crossing and the point are one.
    let (low, high) = contours.iter().flatten().fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(low, high), point| (low.min(*point), high.max(*point)));
    let near = (high - low).max_element().max(0.0) * 1e-4;
    let contours: Vec<Vec<Vec2>> =
        contours.iter().map(|contour| outline::drop_straight(&merge_close(contour, near))).filter(|contour| contour.len() >= 3).collect();
    // Holes run clockwise, outlines counter-clockwise, so the solid side is always on the left.
    let oriented: Vec<Vec<Vec2>> = contours
        .iter()
        .map(|contour| {
            let probe = contour.first().copied().unwrap_or(Vec2::ZERO);
            let depth = contours.iter().filter(|other| !std::ptr::eq(*other, contour) && outline::contains(other, probe)).count();
            let counter_clockwise = outline::signed_area(contour) > 0.0;
            let mut contour = contour.clone();
            if counter_clockwise != (depth % 2 == 0) {
                contour.reverse();
            }
            contour
        })
        .collect();
    let triangles = outline::fill(&oriented);
    let points: Vec<Vec2> = oriented.iter().flatten().copied().collect();
    let front = extrude + bevel;
    let add_fill = |mesh: &mut Mesh, z: f32, reverse: bool| -> u32 {
        let first = mesh.vertex_count();
        mesh.positions.extend(points.iter().map(|point| point.extend(z)));
        for [a, b, c] in &triangles {
            mesh.faces.push(if reverse { vec![first + a, first + c, first + b] } else { vec![first + a, first + b, first + c] });
        }
        first
    };
    if front <= 0.0 {
        add_fill(mesh, 0.0, false);
        return;
    }
    let front_first = add_fill(mesh, front, false);
    let back_first = add_fill(mesh, -front, true);
    // The rings from the back fill to the front fill: out from the outline by the bevel, at a
    // height; the first and last are the fills' own outlines.
    let mut levels: Vec<(f32, f32)> = Vec::new();
    if bevel > 0.0 {
        let steps = bevel_resolution as usize + 1;
        for step in 1..=steps {
            let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_2 * step as f32 / steps as f32;
            levels.push((bevel * angle.cos(), -extrude + bevel * angle.sin()));
        }
        for step in 0..steps {
            let angle = std::f32::consts::FRAC_PI_2 * step as f32 / steps as f32;
            levels.push((bevel * angle.cos(), extrude + bevel * angle.sin()));
        }
        // The last level of each arc meets the fill: drop duplicates at the ends.
        levels.retain(|(out, _)| *out > 1e-7);
    }
    // The sides follow the fill's own boundary (its edges used by one triangle), chained into
    // loops with the solid on their left, so the solid is closed whatever the fill did where
    // contours touch.
    for ring in boundary_loops(&triangles) {
        let contour: Vec<Vec2> = ring.iter().filter_map(|index| points.get(*index as usize).copied()).collect();
        let count = u32::try_from(contour.len()).unwrap_or(0);
        if count < 2 {
            continue;
        }
        let offsets = outward_offsets(&contour);
        let mut rings: Vec<Vec<u32>> = vec![ring.iter().map(|index| back_first + index).collect()];
        for (out, z) in &levels {
            let first = mesh.vertex_count();
            mesh.positions.extend(contour.iter().zip(&offsets).map(|(point, normal)| (*point + *normal * *out).extend(*z)));
            rings.push((0..count).map(|slot| first + slot).collect());
        }
        rings.push(ring.iter().map(|index| front_first + index).collect());
        for pair in rings.windows(2) {
            let [lower, upper] = pair else { continue };
            for slot in 0..contour.len() {
                let next = (slot + 1) % contour.len();
                let (Some(a), Some(b), Some(c), Some(d)) = (lower.get(slot), lower.get(next), upper.get(next), upper.get(slot)) else { continue };
                // Solid on the left: going along the loop, outward is to the right.
                mesh.faces.push(vec![*a, *b, *c, *d]);
            }
        }
    }
}

/// The fill's boundary as loops of point indices: its directed edges used by one triangle, in
/// the triangles' (counter-clockwise) direction, so the solid is on each loop's left.
fn boundary_loops(triangles: &[[u32; 3]]) -> Vec<Vec<u32>> {
    use std::collections::{HashMap, HashSet};
    let mut directed: HashSet<(u32, u32)> = HashSet::new();
    for [a, b, c] in triangles {
        for edge in [(*a, *b), (*b, *c), (*c, *a)] {
            directed.insert(edge);
        }
    }
    let mut next_of: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut boundary: Vec<(u32, u32)> = directed.iter().filter(|(a, b)| !directed.contains(&(*b, *a))).copied().collect();
    boundary.sort_unstable();
    for (a, b) in &boundary {
        next_of.entry(*a).or_default().push(*b);
    }
    let mut used: HashSet<(u32, u32)> = HashSet::new();
    let mut loops = Vec::new();
    for (start, first) in &boundary {
        if used.contains(&(*start, *first)) {
            continue;
        }
        let mut ring = vec![*start];
        let (mut from, mut to) = (*start, *first);
        for _ in 0..boundary.len() {
            used.insert((from, to));
            if to == *start {
                break;
            }
            ring.push(to);
            let Some(next) = next_of.get(&to).and_then(|list| list.iter().find(|candidate| !used.contains(&(to, **candidate))).copied()) else {
                break;
            };
            from = to;
            to = next;
        }
        if ring.len() >= 3 {
            loops.push(ring);
        }
    }
    loops
}

/// Where the edit cursor sits before character `index` (on its line's baseline), in the object's
/// own space, laid out as without a path.
pub fn caret_position(text: &Text, index: usize) -> Vec3 {
    let (lines, _) = text.placed_lines();
    let z = text.extrude + text.bevel_depth;
    let (x0, y0) = (text.box_x + text.offset_x, text.box_y + text.offset_y);
    let at = |x: f32, y: f32| Vec3::new(x + y * text.shear + x0, y + y0, z);
    for line in &lines {
        if let Some(slot) = line.slots.iter().find(|slot| slot.index == index) {
            return at(slot.pen, line.baseline);
        }
        if index == line.end || (line.slots.first().is_some_and(|first| first.index <= index) && index <= line.end) {
            let x = line.slots.last().map_or_else(|| line.slots.first().map_or(0.0, |slot| slot.pen), |slot| slot.pen + slot.advance);
            let x = if line.slots.is_empty() { empty_line_x(text) } else { x };
            return at(x, line.baseline);
        }
    }
    let last = lines.last();
    at(last.map_or(0.0, |line| line.slots.last().map_or(empty_line_x(text), |slot| slot.pen + slot.advance)), last.map_or(0.0, |line| line.baseline))
}

/// Where an empty line's caret sits: where its alignment would start a line.
fn empty_line_x(text: &Text) -> f32 {
    match (text.align, text.box_width > 0.0) {
        (TextAlign::Center, true) => text.box_width / 2.0,
        (TextAlign::Right, true) => text.box_width,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_font_loads() {
        assert!(face().is_some());
    }

    #[test]
    fn a_capital_i_is_one_box() {
        let mesh = Text { body: "I".into(), ..Text::default() }.to_mesh();
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.faces.len(), 2);
        let bounds = mesh.bounds(glam::Mat4::IDENTITY);
        // The reference: cap height 1490 of a 2929-unit box.
        assert!((bounds.max.y - 0.508_706).abs() < 1e-4, "{bounds:?}");
    }
}
