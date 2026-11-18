//! Shadow maps: what each light sees, as depth, so shading can tell whether a point is lit.
//! A sun gets one orthographic map fitted around the scene, a spot one perspective map along its
//! cone, and point and area lights a cube of six perspective maps. Lookups use a slope-scaled
//! bias, a normal offset, and percentage-closer filtering whose width follows the light's radius.
//!
//! Casters may let light through by their alpha (final renders): clipped alpha leaves holes in
//! the depth picture, and blended alpha is kept apart in a second layer, the nearest such
//! surface's depth and the share of light every such surface lets through ((1 − alpha) each,
//! as the reference's transparent shadows measure), which dims what lies behind it.

use std::sync::Arc;

use glam::{Mat4, Vec2, Vec3, Vec4};
use kition_scene_model::{Bounds, LightKind};

use crate::cache::Prepared;
use crate::texture::Texture;

/// How a caster lets light through by its alpha.
#[derive(Debug, Clone)]
pub struct CasterAlpha {
    /// The material's alpha, times the base colour image's where it has one.
    pub alpha: f32,
    pub texture: Option<Arc<Texture>>,
    /// Clipped: no shadow where alpha is below this. Otherwise blended.
    pub cutoff: Option<f32>,
}

impl CasterAlpha {
    /// How much of the light the caster stops at a point: its alpha, or for clipped casters 1
    /// at and above the cutoff and 0 below.
    fn at(&self, uv: Option<Vec2>) -> f32 {
        let image = match (&self.texture, uv) {
            (Some(texture), Some(uv)) => texture.sample(uv, 0.0).w,
            _ => 1.0,
        };
        let alpha = (self.alpha * image).clamp(0.0, 1.0);
        match self.cutoff {
            Some(cutoff) => f32::from(u8::from(alpha >= cutoff)),
            None => alpha,
        }
    }
}

/// One depth picture from a light: eye depth (distance in front of the light) per texel.
#[derive(Debug, Clone)]
struct Face {
    view: Mat4,
    view_projection: Mat4,
    /// Eye depth as a dot product: `depth_row · (point, 1)`.
    depth_row: Vec4,
    size: u32,
    depth: Vec<f32>,
    /// The nearest surface with blended alpha per texel, and the share of light all of them let
    /// through (empty when no caster blends).
    near: Vec<f32>,
    pass: Vec<f32>,
    orthographic: bool,
    /// World size of a texel at one metre in front of the light (perspective), or everywhere
    /// (orthographic).
    texel: f32,
}

/// The depth pictures of one light.
#[derive(Debug, Clone)]
pub struct ShadowMap {
    faces: Vec<Face>,
    /// Where a cube map is centred (point and area lights).
    centre: Option<Vec3>,
    /// The light's radius in texels at one metre, for the filter width.
    softness: f32,
    /// Filter taps per side (2 in the viewport, 4 in renders).
    taps: u32,
}

/// Geometry that casts shadows: prepared meshes and their world matrices.
pub type Casters = [(Arc<Prepared>, Mat4)];

/// The world box of the casters.
pub fn caster_bounds(casters: &Casters) -> Bounds {
    casters.iter().fold(Bounds::EMPTY, |bounds, (prepared, world)| bounds.union(prepared.mesh.bounds(*world)))
}

/// The most faces a caster may have to be left out of a sun map's box (the check visits each).
const MAX_RECEIVER_FACES: usize = 20_000;

/// The box a sun's map covers: the casters' box, leaving out large casters that cannot shadow
/// any other caster, such as a floor or a backdrop under and behind everything, lit from in
/// front. The shadows the others cast on them lie inside the others' box as the sun sees it, so
/// the map's texels go to what casts: a floor 400 m wide would otherwise spread a 2048-texel map
/// over 400 m, and every shadow would turn to blocks. Points of the left-out casters outside the
/// map are lit, which they are (nothing else can shadow them there).
pub fn sun_bounds(casters: &Casters, direction: Vec3) -> Bounds {
    let all = caster_bounds(casters);
    let to_sun = -direction.normalize_or_zero();
    if to_sun == Vec3::ZERO || casters.len() < 2 || all.is_empty() {
        return all;
    }
    let boxes: Vec<Bounds> = casters.iter().map(|(prepared, world)| prepared.mesh.bounds(*world)).collect();
    let diagonal = all.size().length();
    let mut kept = Bounds::EMPTY;
    let mut left_out = false;
    for (index, ((prepared, world), own)) in casters.iter().zip(&boxes).enumerate() {
        let large = own.size().length() > diagonal * 0.5;
        let receiver = large
            && !prepared.mesh.faces.is_empty()
            && prepared.mesh.faces.len() <= MAX_RECEIVER_FACES
            && shadows_nothing(
                prepared,
                *world,
                to_sun,
                boxes.iter().enumerate().filter(|(other, _)| *other != index).map(|(_, bounds)| bounds),
                diagonal,
            );
        if receiver {
            left_out = true;
        } else {
            kept = kept.union(*own);
        }
    }
    if left_out && !kept.is_empty() { kept } else { all }
}

/// Whether no ray from a point of `others` towards the sun can meet the caster: for each of its
/// faces, every corner of every other box is on the sun's side of the face's plane (or on it).
fn shadows_nothing<'a>(prepared: &Prepared, world: Mat4, to_sun: Vec3, others: impl Iterator<Item = &'a Bounds>, diagonal: f32) -> bool {
    let corners: Vec<Vec3> = others
        .filter(|bounds| !bounds.is_empty())
        .flat_map(|bounds| {
            let (a, b) = (bounds.min, bounds.max);
            [
                Vec3::new(a.x, a.y, a.z),
                Vec3::new(b.x, a.y, a.z),
                Vec3::new(a.x, b.y, a.z),
                Vec3::new(b.x, b.y, a.z),
                Vec3::new(a.x, a.y, b.z),
                Vec3::new(b.x, a.y, b.z),
                Vec3::new(a.x, b.y, b.z),
                Vec3::new(b.x, b.y, b.z),
            ]
        })
        .collect();
    let positions: Vec<Vec3> = prepared.mesh.positions.iter().map(|point| world.transform_point3(*point)).collect();
    let tolerance = diagonal.max(1e-3) * 1e-4;
    for face in &prepared.mesh.faces {
        let points: Vec<Vec3> = face.iter().filter_map(|index| positions.get(*index as usize).copied()).collect();
        let normal = kition_scene_model::mesh::newell_normal(&points);
        let Some(origin) = points.first().copied() else { continue };
        let toward = normal.dot(to_sun);
        if normal == Vec3::ZERO || toward.abs() < 1e-6 {
            continue;
        }
        // A ray from a corner towards the sun crosses the plane when the corner is on its other
        // side.
        if corners.iter().any(|corner| {
            let side = normal.dot(*corner - origin);
            side.abs() > tolerance && (side < 0.0) == (toward > 0.0)
        }) {
            return false;
        }
    }
    true
}

impl Face {
    fn new(view: Mat4, projection: Mat4, size: u32, orthographic: bool, texel: f32) -> Self {
        let count = (size as usize).saturating_mul(size as usize);
        let depth_row = -view.row(2);
        Self {
            view,
            view_projection: projection * view,
            depth_row,
            size,
            depth: vec![f32::INFINITY; count],
            near: Vec::new(),
            pass: Vec::new(),
            orthographic,
            texel,
        }
    }

    /// Draws the casters' depth. A closed mesh draws only its faces turned away from the light:
    /// its lit side can then never shadow itself (no acne along edges and grazing faces), and
    /// the far side still blocks the light. Open meshes draw both sides, so nothing leaks.
    fn draw(&mut self, casters: &Casters, alphas: &[Option<CasterAlpha>]) {
        let size = self.size as f32;
        if alphas.iter().any(|alpha| alpha.as_ref().is_some_and(|alpha| alpha.cutoff.is_none())) {
            self.near = vec![f32::INFINITY; self.depth.len()];
            self.pass = vec![1.0; self.depth.len()];
        }
        for (index, (prepared, world)) in casters.iter().enumerate() {
            let alpha = alphas.get(index).and_then(Option::as_ref);
            let blended = alpha.is_some_and(|alpha| alpha.cutoff.is_none());
            let matrix = self.view_projection * *world;
            let eye = self.view * *world;
            let clips: Vec<Vec4> = prepared.mesh.positions.iter().map(|point| matrix * point.extend(1.0)).collect();
            let depths: Vec<f32> = prepared.mesh.positions.iter().map(|point| -eye.transform_point3(*point).z).collect();
            let eyes: Vec<Vec3> =
                if prepared.closed { prepared.mesh.positions.iter().map(|point| eye.transform_point3(*point)).collect() } else { Vec::new() };
            // A mirroring transform turns the windings over.
            let mirrored = world.determinant() < 0.0;
            for (number, (triangle, _)) in prepared.triangles.iter().enumerate() {
                // Blended surfaces dim the light on both sides: every face counts.
                if prepared.closed && !blended {
                    let (Some(pa), Some(pb), Some(pc)) =
                        (eyes.get(triangle[0] as usize), eyes.get(triangle[1] as usize), eyes.get(triangle[2] as usize))
                    else {
                        continue;
                    };
                    let mut normal = (*pb - *pa).cross(*pc - *pa);
                    if mirrored {
                        normal = -normal;
                    }
                    // Towards the light: the eye is at the origin looking down -Z.
                    let toward = if self.orthographic { Vec3::Z } else { -*pa };
                    if normal.dot(toward) > 0.0 {
                        continue;
                    }
                }
                let corner = |index: u32| -> Option<(Vec4, f32)> { Some((*clips.get(index as usize)?, *depths.get(index as usize)?)) };
                let (Some(a), Some(b), Some(c)) = (corner(triangle[0]), corner(triangle[1]), corner(triangle[2])) else { continue };
                // Triangles reaching behind the light are left out (a cube face covers 90°, so
                // only geometry beside the light is lost, and it is in another face).
                if a.0.w <= 1e-4 || b.0.w <= 1e-4 || c.0.w <= 1e-4 {
                    continue;
                }
                let screen = |(clip, depth): (Vec4, f32)| {
                    let ndc = clip.truncate() / clip.w;
                    (Vec3::new((ndc.x + 1.0) * 0.5 * size, (1.0 - ndc.y) * 0.5 * size, depth), 1.0 / clip.w)
                };
                let uvs = prepared.triangle_uvs.get(number).copied();
                match alpha {
                    None => self.raster([screen(a), screen(b), screen(c)], None, &|_| 1.0),
                    Some(alpha) => self.raster([screen(a), screen(b), screen(c)], Some((uvs, blended)), &|uv| alpha.at(uv)),
                }
            }
        }
    }

    /// Draws a triangle's depth. With `alpha` (the corners' texture coordinates, and whether it
    /// blends), `opacity` gives the alpha at a point: clipped casters leave texels below their
    /// cutoff (`opacity` answers 0 there), blended ones go into the second layer.
    fn raster(&mut self, corners: [(Vec3, f32); 3], alpha: Option<(Option<[Vec2; 3]>, bool)>, opacity: &dyn Fn(Option<Vec2>) -> f32) {
        let [(a, wa), (b, wb), (c, wc)] = corners;
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if !area.is_finite() || area.abs() < 1e-12 {
            return;
        }
        let size = self.size as i64;
        let min_x = (a.x.min(b.x).min(c.x).floor() as i64).max(0);
        let max_x = (a.x.max(b.x).max(c.x).ceil() as i64).min(size);
        let min_y = (a.y.min(b.y).min(c.y).floor() as i64).max(0);
        let max_y = (a.y.max(b.y).max(c.y).ceil() as i64).min(size);
        if min_x >= max_x || min_y >= max_y {
            return;
        }
        let inverse_area = 1.0 / area;
        let (step0, step1) = ((b.y - c.y) * inverse_area, (c.y - a.y) * inverse_area);
        // Depth over w and 1 / w are linear across the picture.
        let (da, db, dc) = (a.z * wa, b.z * wb, c.z * wc);
        for y in min_y..max_y {
            let py = y as f32 + 0.5;
            let px = min_x as f32 + 0.5;
            let mut w0 = ((b.x - px) * (c.y - py) - (b.y - py) * (c.x - px)) * inverse_area;
            let mut w1 = ((c.x - px) * (a.y - py) - (c.y - py) * (a.x - px)) * inverse_area;
            let row = y as usize * self.size as usize;
            for x in min_x..max_x {
                let (u0, u1) = (w0, w1);
                w0 += step0;
                w1 += step1;
                let u2 = 1.0 - u0 - u1;
                if u0 < 0.0 || u1 < 0.0 || u2 < 0.0 {
                    continue;
                }
                let inverse = u0 * wa + u1 * wb + u2 * wc;
                if inverse <= 0.0 {
                    continue;
                }
                let depth = (u0 * da + u1 * db + u2 * dc) / inverse;
                let slot = row + x as usize;
                if let Some((uvs, blended)) = alpha {
                    // Texture coordinates over w are linear across the picture too.
                    let uv = uvs.map(|[ta, tb, tc]| (ta * (u0 * wa) + tb * (u1 * wb) + tc * (u2 * wc)) / inverse);
                    let opacity = opacity(uv);
                    if blended {
                        if let Some(near) = self.near.get_mut(slot) {
                            *near = near.min(depth);
                        }
                        if let Some(pass) = self.pass.get_mut(slot) {
                            *pass *= 1.0 - opacity;
                        }
                        continue;
                    }
                    if opacity <= 0.0 {
                        continue;
                    }
                }
                if let Some(stored) = self.depth.get_mut(slot)
                    && depth < *stored
                {
                    *stored = depth;
                }
            }
        }
    }

    /// The share of a point's filter footprint that the light reaches, or `None` when the point
    /// is outside this face.
    fn visibility(&self, point: Vec3, normal: Vec3, to_light: Vec3, softness: f32, taps: u32) -> Option<f32> {
        let depth = self.depth_row.dot(point.extend(1.0));
        if depth <= 0.0 {
            return None;
        }
        // A texel's world size where the point is.
        let texel = if self.orthographic { self.texel } else { self.texel * depth };
        let cosine = normal.dot(to_light).clamp(0.05, 1.0);
        let slope = (1.0 - cosine * cosine).sqrt() / cosine;
        // Move the point off its surface, then allow for the texel's slope across it.
        let shift = normal * texel * 1.5;
        let clip = self.view_projection * (point + shift).extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
            return None;
        }
        let size = self.size as f32;
        let (sx, sy) = ((ndc.x + 1.0) * 0.5 * size, (1.0 - ndc.y) * 0.5 * size);
        let reference = depth + self.depth_row.truncate().dot(shift) - texel * (1.0 + 2.0 * slope.min(8.0));
        // Percentage-closer filtering on a `taps` × `taps` grid, spaced by the softness.
        let spread = if self.orthographic { softness } else { softness / depth.max(0.1) }.clamp(0.75, 6.0);
        let middle = (taps as f32 - 1.0) / 2.0;
        let (size, stride) = (self.size as i64, self.size as usize);
        let mut lit = 0.0f32;
        for j in 0..taps {
            let y = (sy + (j as f32 - middle) * spread) as i64;
            if y < 0 || y >= size {
                lit += taps as f32;
                continue;
            }
            let start = y as usize * stride;
            let row = self.depth.get(start..start + stride).unwrap_or(&[]);
            for i in 0..taps {
                let x = (sx + (i as f32 - middle) * spread) as i64;
                if x < 0 || x >= size {
                    lit += 1.0;
                } else if reference <= row.get(x as usize).copied().unwrap_or(f32::INFINITY) {
                    // Behind a blended surface, only what they all let through.
                    let slot = start + x as usize;
                    let behind = self.near.get(slot).is_some_and(|near| reference > *near);
                    lit += if behind { self.pass.get(slot).copied().unwrap_or(1.0) } else { 1.0 };
                }
            }
        }
        Some(lit / (taps * taps) as f32)
    }
}

/// An up vector not parallel to `direction`.
fn up_for(direction: Vec3) -> Vec3 {
    if direction.cross(Vec3::Z).length_squared() < 1e-6 { Vec3::Y } else { Vec3::Z }
}

/// Where one depth picture of a light looks: its view and projection, its size in texels, and a
/// texel's world size (at one metre in front of the light, or everywhere when orthographic).
#[derive(Debug, Clone, Copy)]
pub struct FacePlan {
    pub view: Mat4,
    pub projection: Mat4,
    pub size: u32,
    pub orthographic: bool,
    pub texel: f32,
}

/// The depth pictures a light's shadow map is made of, before anything is drawn into them (the
/// GPU viewport draws them itself, the same way).
#[derive(Debug, Clone)]
pub struct ShadowPlan {
    pub faces: Vec<FacePlan>,
    /// Where a cube map is centred (point and area lights): lookups choose the face by the
    /// largest axis of the direction from it.
    pub centre: Option<Vec3>,
    /// The light's radius in texels at one metre, for the filter width.
    pub softness: f32,
    /// Filter taps per side.
    pub taps: u32,
}

impl ShadowPlan {
    /// The pictures of one light at `position` shining along `direction` (unit), over `bounds`,
    /// with `size` texels per side (cube faces get half), filtered with `taps` × `taps` lookups.
    pub fn new(kind: LightKind, position: Vec3, direction: Vec3, spot_size: f32, radius: f32, bounds: Bounds, size: u32, taps: u32) -> Option<Self> {
        let taps = taps.clamp(1, 6);
        if bounds.is_empty() {
            return None;
        }
        let size = size.clamp(64, 4096);
        let centre = bounds.center();
        let reach = (bounds.size().length() / 2.0).max(0.01);
        match kind {
            LightKind::Sun => {
                let eye = centre - direction * reach * 2.0;
                let view = Mat4::look_at_rh(eye, centre, up_for(direction));
                let projection = Mat4::orthographic_rh(-reach, reach, -reach, reach, 0.0, reach * 4.0);
                let face = FacePlan { view, projection, size, orthographic: true, texel: 2.0 * reach / size as f32 };
                // A sun's softness is its angle (radius stands for it): a little blur only.
                Some(Self { faces: vec![face], centre: None, softness: 1.0 + radius * 4.0, taps })
            }
            LightKind::Spot => {
                let far = (position.distance(centre) + reach).max(0.1);
                let fov = (spot_size + 0.1).clamp(0.05, 3.0);
                let view = Mat4::look_at_rh(position, position + direction, up_for(direction));
                let projection = Mat4::perspective_rh(fov, 1.0, far * 1e-4, far * 1.01);
                let texel = 2.0 * (fov / 2.0).tan() / size as f32;
                let face = FacePlan { view, projection, size, orthographic: false, texel };
                Some(Self { faces: vec![face], centre: None, softness: radius / texel.max(1e-6) * 0.5, taps })
            }
            LightKind::Point | LightKind::Area => {
                let size = (size / 2).max(64);
                let far = (position.distance(centre) + reach).max(0.1);
                let projection = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, far * 1e-4, far * 1.01);
                let texel = 2.0 / size as f32;
                let axes = [
                    (Vec3::X, Vec3::NEG_Z),
                    (Vec3::NEG_X, Vec3::NEG_Z),
                    (Vec3::Y, Vec3::NEG_Z),
                    (Vec3::NEG_Y, Vec3::NEG_Z),
                    (Vec3::Z, Vec3::Y),
                    (Vec3::NEG_Z, Vec3::Y),
                ];
                let faces = axes
                    .iter()
                    .map(|(forward, up)| FacePlan {
                        view: Mat4::look_at_rh(position, position + *forward, *up),
                        projection,
                        size,
                        orthographic: false,
                        texel,
                    })
                    .collect();
                Some(Self { faces, centre: Some(position), softness: radius / texel * 0.5, taps })
            }
        }
    }
}

impl ShadowMap {
    /// The map of one light at `position` shining along `direction` (unit), over `bounds`, with
    /// `size` texels per side (cube faces get half), filtered with `taps` × `taps` lookups.
    /// `alphas` says how each caster lets light through (none: opaque; may be empty).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        kind: LightKind,
        position: Vec3,
        direction: Vec3,
        spot_size: f32,
        radius: f32,
        bounds: Bounds,
        casters: &Casters,
        alphas: &[Option<CasterAlpha>],
        size: u32,
        taps: u32,
    ) -> Option<Self> {
        if casters.is_empty() {
            return None;
        }
        let plan = ShadowPlan::new(kind, position, direction, spot_size, radius, bounds, size, taps)?;
        let faces = plan
            .faces
            .iter()
            .map(|face| {
                let mut drawn = Face::new(face.view, face.projection, face.size, face.orthographic, face.texel);
                drawn.draw(casters, alphas);
                drawn
            })
            .collect();
        Some(Self { faces, centre: plan.centre, softness: plan.softness, taps: plan.taps })
    }

    /// How much of the light reaches `point` (0 in full shadow, 1 fully lit).
    pub fn visibility(&self, point: Vec3, normal: Vec3, to_light: Vec3) -> f32 {
        let normal = if normal.dot(to_light) < 0.0 { -normal } else { normal };
        match self.centre {
            Some(centre) => {
                // The cube face the point is on, by the largest axis of its direction.
                let offset = point - centre;
                let abs = offset.abs();
                let index = if abs.x >= abs.y && abs.x >= abs.z {
                    if offset.x >= 0.0 { 0 } else { 1 }
                } else if abs.y >= abs.z {
                    if offset.y >= 0.0 { 2 } else { 3 }
                } else if offset.z >= 0.0 {
                    4
                } else {
                    5
                };
                self.faces.get(index).and_then(|face| face.visibility(point, normal, to_light, self.softness, self.taps)).unwrap_or(1.0)
            }
            None => self.faces.first().and_then(|face| face.visibility(point, normal, to_light, self.softness, self.taps)).unwrap_or(1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::Primitive;

    #[test]
    fn a_sun_map_shadows_the_floor_behind_a_cube() {
        let cube = Arc::new(Prepared::new(Primitive::Cube { size: 2.0 }.build().unwrap()));
        let floor = Arc::new(Prepared::new(Primitive::Plane { size: 12.0 }.build().unwrap()));
        let casters = vec![(cube, Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0))), (floor, Mat4::IDENTITY)];
        let direction = Vec3::new(0.0, -1.0, -1.0).normalize();
        let map = ShadowMap::build(LightKind::Sun, Vec3::ZERO, direction, 0.7, 0.1, caster_bounds(&casters), &casters, &[], 512, 4).unwrap();
        let to_light = -direction;
        assert_eq!(map.visibility(Vec3::new(0.0, -2.5, 0.0), Vec3::Z, to_light), 0.0, "in the cube's shadow");
        assert_eq!(map.visibility(Vec3::new(3.5, 0.0, 0.0), Vec3::Z, to_light), 1.0, "open floor");
        assert_eq!(map.visibility(Vec3::new(0.0, 0.0, 2.0), Vec3::Z, to_light), 1.0, "the cube's lit top does not shadow itself");
    }

    #[test]
    fn a_point_light_shadows_through_its_cube_faces() {
        let cube = Arc::new(Prepared::new(Primitive::Cube { size: 2.0 }.build().unwrap()));
        let floor = Arc::new(Prepared::new(Primitive::Plane { size: 20.0 }.build().unwrap()));
        let casters = vec![(cube, Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0))), (floor, Mat4::IDENTITY)];
        let light = Vec3::new(0.0, 4.0, 3.0);
        let map = ShadowMap::build(LightKind::Point, light, Vec3::NEG_Z, 0.7, 0.05, caster_bounds(&casters), &casters, &[], 1024, 4).unwrap();
        let behind = Vec3::new(0.0, -2.0, 0.0);
        let beside = Vec3::new(4.0, 4.0, 0.0);
        assert_eq!(map.visibility(behind, Vec3::Z, (light - behind).normalize()), 0.0);
        assert_eq!(map.visibility(beside, Vec3::Z, (light - beside).normalize()), 1.0);
    }
}
