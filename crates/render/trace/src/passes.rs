//! Render passes beyond the picture, as the reference's Render Layers node gives them.
//!
//! The light passes come from the path tracer's samples ([`Sample`], [`Sums`]): diffuse and
//! glossy light split at the first surface by its lobes, direct (light sampled there, or an
//! emitter or the world found right after its first bounce) or indirect (everything later), each
//! divided by its colour (the lobe's colour at the first surface), and the emission seen
//! directly.
//!
//! The geometric passes ([`geometry`]) are traced through each pixel at sixteen points, for
//! fast and path-traced renders alike: the first surface's normal, the vector pass (where each
//! pixel's surface point was the frame before and will be the frame after, in pixels with y
//! up, from the object's and the camera's motion; deformation does not count, and with motion
//! blur on it is zero, as the reference's path tracer leaves it), the world seen directly,
//! ambient occlusion (the share of the hemisphere open within ten metres, the reference's
//! default distance), and each pixel's coverage by object, material, and asset (the topmost
//! parent) names for Cryptomatte.

use std::collections::BTreeMap;

use glam::{Mat4, Vec2, Vec3};
use kition_scene_model::{Id, Scene};
use kition_scene_raster::view::camera_view;

use crate::sampling::{Frame, Rng, cosine_hemisphere};
use crate::{World, compile};

/// One sample's share of the light passes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub diffuse_direct: Vec3,
    pub diffuse_indirect: Vec3,
    pub diffuse_color: Vec3,
    pub glossy_direct: Vec3,
    pub glossy_indirect: Vec3,
    pub glossy_color: Vec3,
    pub emission: Vec3,
}

/// The light passes, by the Render Layers output that shows each, in the sums' order.
pub const LIGHT_PASSES: [&str; 7] =
    ["Diffuse Direct", "Diffuse Indirect", "Diffuse Color", "Glossy Direct", "Glossy Indirect", "Glossy Color", "Emission"];

/// The light passes' sums per pixel.
#[derive(Debug, Clone)]
pub struct Sums {
    pub(crate) buffers: Vec<Vec<Vec3>>,
}

impl Sums {
    pub fn new(count: usize) -> Self {
        Self { buffers: vec![vec![Vec3::ZERO; count]; LIGHT_PASSES.len()] }
    }

    /// Adds a sample at pixel `index`.
    pub fn add(&mut self, index: usize, sample: &Sample) {
        let values = [
            sample.diffuse_direct,
            sample.diffuse_indirect,
            sample.diffuse_color,
            sample.glossy_direct,
            sample.glossy_indirect,
            sample.glossy_color,
            sample.emission,
        ];
        for (buffer, value) in self.buffers.iter_mut().zip(values) {
            if let Some(slot) = buffer.get_mut(index)
                && value.is_finite()
            {
                *slot += value;
            }
        }
    }

    /// The passes: each pixel's mean over its `counts` samples, light over its colour.
    pub fn finish(&self, counts: &[f32]) -> BTreeMap<String, Vec<[f32; 4]>> {
        let mean = |buffer: usize| -> Vec<Vec3> {
            self.buffers.get(buffer).map_or_else(Vec::new, |values| values.iter().zip(counts).map(|(value, count)| *value / count.max(1.0)).collect())
        };
        let means: Vec<Vec<Vec3>> = (0..LIGHT_PASSES.len()).map(mean).collect();
        let over = |light: &Vec3, color: &Vec3| {
            let part = |light: f32, color: f32| if color > 1e-6 { light / color } else { 0.0 };
            Vec3::new(part(light.x, color.x), part(light.y, color.y), part(light.z, color.z))
        };
        let mut out = BTreeMap::new();
        for (index, name) in LIGHT_PASSES.iter().enumerate() {
            let color = match index {
                0 | 1 => means.get(2),
                3 | 4 => means.get(5),
                _ => None,
            };
            let values: Vec<[f32; 4]> = match (means.get(index), color) {
                (Some(light), Some(color)) => light.iter().zip(color).map(|(light, color)| over(light, color).extend(1.0).to_array()).collect(),
                (Some(values), None) => values.iter().map(|value| value.extend(1.0).to_array()).collect(),
                _ => Vec::new(),
            };
            out.insert((*name).to_owned(), values);
        }
        out
    }
}

/// The geometric passes of a render.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    /// By the Render Layers output: `Normal`, `Vector`, `Environment`, `Ambient Occlusion`.
    pub passes: BTreeMap<String, Vec<[f32; 4]>>,
    /// Per pixel, how much each object, material, and asset name covers it.
    pub objects: Vec<BTreeMap<String, f32>>,
    pub materials: Vec<BTreeMap<String, f32>>,
    pub assets: Vec<BTreeMap<String, f32>>,
}

/// Points per pixel side.
const SIDE: u32 = 4;
/// How far ambient occlusion looks (metres).
const AO_DISTANCE: f32 = 10.0;

/// The geometric passes of `scene` through its camera at its render size.
pub fn geometry(scene: &Scene) -> Result<Geometry, String> {
    let camera_id = scene.camera.ok_or("not-allowed: the scene has no camera")?;
    let (width, height) = (scene.render.width, scene.render.height);
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 8192 * 8192 {
        return Err("invalid-params: the render size is out of range".into());
    }
    let camera = camera_view(scene, camera_id).ok_or("not-allowed: the scene camera is not a camera")?.fit(width, height, width, height);
    // Still, at the frame itself: the shutter only matters for the vector pass.
    let mut still = scene.clone();
    still.render.motion_blur.enabled = false;
    let world = compile(&still, camera.clone(), Some(camera_id), None, true)?;
    let frame = scene.animation.timeline.frame as f32;
    let motion = !scene.render.motion_blur.enabled;
    let neighbours = motion.then(|| [scene.at_frame(frame - 1.0), scene.at_frame(frame + 1.0)]);
    // Each object's world matrix now and the frames either side, and the camera then.
    let matrices = |at: &Scene| -> Vec<Mat4> { world.objects.iter().map(|id| at.world_matrix(*id).unwrap_or(Mat4::IDENTITY)).collect() };
    let now = matrices(scene);
    let others: Option<[(Vec<Mat4>, Mat4); 2]> = neighbours.as_ref().map(|[before, after]| {
        let view =
            |at: &Scene| camera_view(at, camera_id).map_or(camera.view_projection(), |view| view.fit(width, height, width, height).view_projection());
        [(matrices(before), view(before)), (matrices(after), view(after))]
    });
    let names = Names::new(scene, &world);
    let count = width as usize * height as usize;
    let mut normal = vec![[0.0f32; 4]; count];
    let mut vector = vec![[0.0f32; 4]; count];
    let mut environment = vec![[0.0f32; 4]; count];
    let mut occlusion = vec![[0.0f32; 4]; count];
    let (mut objects, mut materials, mut assets) = (vec![BTreeMap::new(); count], vec![BTreeMap::new(); count], vec![BTreeMap::new(); count]);
    let share = 1.0 / (SIDE * SIDE) as f32;
    let to_screen = |projection: Mat4, point: Vec3| {
        let clip = projection * point.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        Vec2::new((ndc.x + 1.0) * 0.5 * width as f32, (ndc.y + 1.0) * 0.5 * height as f32)
    };
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let mut rng = Rng::for_sample(0x9a55, index as u64, 0);
            for j in 0..SIDE {
                for i in 0..SIDE {
                    let (px, py) = (x as f32 + (i as f32 + 0.5) / SIDE as f32, y as f32 + (j as f32 + 0.5) / SIDE as f32);
                    let (origin, direction) = camera.ray(px, py);
                    let Some(hit) = world.intersect(origin, direction, f32::INFINITY, false, 0.5) else {
                        let light = crate::world_light(&world, direction);
                        if let Some(slot) = environment.get_mut(index) {
                            for (value, channel) in slot.iter_mut().zip(light.to_array()) {
                                *value += channel * share;
                            }
                            slot[3] = 1.0;
                        }
                        continue;
                    };
                    let Some(data) = world.shading.get(hit.triangle as usize) else { continue };
                    let (outward, [n0, n1, n2]) = world.normals_at(hit.triangle, data, 0.5);
                    let w = 1.0 - hit.u - hit.v;
                    let mut shading = (n0 * w + n1 * hit.u + n2 * hit.v).normalize_or_zero();
                    if shading == Vec3::ZERO {
                        shading = outward;
                    }
                    let point = origin + direction * hit.distance;
                    if let Some(slot) = normal.get_mut(index) {
                        for (value, channel) in slot.iter_mut().zip(shading.to_array()) {
                            *value += channel * share;
                        }
                    }
                    // Ambient occlusion: one direction per point.
                    let facing = if outward.dot(direction) < 0.0 { outward } else { -outward };
                    let up = if shading.dot(facing) < 0.0 { -shading } else { shading };
                    let towards = Frame::new(up).to_world(cosine_hemisphere(rng.next2())).normalize_or_zero();
                    let start = crate::offset(point, facing, towards);
                    if towards.dot(facing) > 0.0 && world.intersect(start, towards, AO_DISTANCE, true, 0.5).is_none() {
                        for value in occlusion.get_mut(index).map_or(&mut [][..], |slot| &mut slot[..3]) {
                            *value += share;
                        }
                    }
                    let object = data.object as usize;
                    if let (Some(others), Some(matrix)) = (&others, now.get(object)) {
                        let local = matrix.inverse().transform_point3(point);
                        let here = to_screen(camera.view_projection(), point);
                        let [(before, before_view), (after, after_view)] = others;
                        let at = |matrices: &Vec<Mat4>, view: Mat4| {
                            to_screen(view, matrices.get(object).copied().unwrap_or(*matrix).transform_point3(local))
                        };
                        let (previous, next) = (at(before, *before_view), at(after, *after_view));
                        let motion = [previous.x - here.x, previous.y - here.y, here.x - next.x, here.y - next.y];
                        if let Some(slot) = vector.get_mut(index) {
                            for (value, channel) in slot.iter_mut().zip(motion) {
                                *value += if channel.is_finite() { channel * share } else { 0.0 };
                            }
                        }
                    }
                    let credit = |into: &mut Vec<BTreeMap<String, f32>>, name: Option<&String>| {
                        if let (Some(name), Some(pixel)) = (name, into.get_mut(index)) {
                            *pixel.entry(name.clone()).or_insert(0.0) += share;
                        }
                    };
                    credit(&mut objects, names.objects.get(object));
                    credit(&mut materials, names.materials.get(data.material as usize).and_then(Option::as_ref));
                    credit(&mut assets, names.assets.get(object));
                }
            }
        }
    }
    let passes = BTreeMap::from([
        ("Normal".to_owned(), normal),
        ("Vector".to_owned(), vector),
        ("Environment".to_owned(), environment.into_iter().map(|[r, g, b, _]| [r, g, b, 1.0]).collect()),
        ("Ambient Occlusion".to_owned(), occlusion.into_iter().map(|[r, g, b, _]| [r, g, b, 1.0]).collect()),
    ]);
    Ok(Geometry { width, height, passes, objects, materials, assets })
}

/// The names the Cryptomatte layers use, by the world's object and material slots.
struct Names {
    objects: Vec<String>,
    materials: Vec<Option<String>>,
    assets: Vec<String>,
}

impl Names {
    fn new(scene: &Scene, world: &World) -> Self {
        let name = |id: Id| scene.object(id).map(|object| object.name.clone()).unwrap_or_default();
        let top = |id: Id| {
            let mut current = id;
            for _ in 0..1024 {
                match scene.object(current).ok().and_then(|object| object.parent) {
                    Some(parent) => current = parent,
                    None => break,
                }
            }
            name(current)
        };
        Self {
            objects: world.objects.iter().map(|id| name(*id)).collect(),
            materials: world.material_ids.iter().map(|id| id.and_then(|id| scene.material(id).ok()).map(|material| material.name.clone())).collect(),
            assets: world.objects.iter().map(|id| top(*id)).collect(),
        }
    }
}

/// The render's world and the triangle the scene camera sees at picture point (`x`, `y`)
/// (fractions of the render from its top left), if it sees one there.
fn seen_at(scene: &Scene, x: f32, y: f32) -> Result<Option<(World, usize)>, String> {
    let camera_id = scene.camera.ok_or("not-allowed: the scene has no camera")?;
    let (width, height) = (scene.render.width.max(1), scene.render.height.max(1));
    if !(x.is_finite() && y.is_finite()) {
        return Err("invalid-params: the point is not finite".into());
    }
    let camera = camera_view(scene, camera_id).ok_or("not-allowed: the scene camera is not a camera")?.fit(width, height, width, height);
    let mut still = scene.clone();
    still.render.motion_blur.enabled = false;
    let world = compile(&still, camera.clone(), Some(camera_id), None, true)?;
    let (origin, direction) = camera.ray(x.clamp(0.0, 1.0) * width as f32, y.clamp(0.0, 1.0) * height as f32);
    let Some(hit) = world.intersect(origin, direction, f32::INFINITY, false, 0.5) else { return Ok(None) };
    Ok(Some((world, hit.triangle as usize)))
}

/// The object, material, and asset names Cryptomatte gives the surface at picture point
/// (`x`, `y`) (fractions of the render from its top left), if the camera sees one there.
pub fn names_at(scene: &Scene, x: f32, y: f32) -> Result<Option<[Option<String>; 3]>, String> {
    let Some((world, triangle)) = seen_at(scene, x, y)? else { return Ok(None) };
    let Some(data) = world.shading.get(triangle) else { return Ok(None) };
    let names = Names::new(scene, &world);
    let object = data.object as usize;
    Ok(Some([names.objects.get(object).cloned(), names.materials.get(data.material as usize).cloned().flatten(), names.assets.get(object).cloned()]))
}

/// The object whose surface the camera sees at picture point (`x`, `y`), as [`names_at`] names it.
pub fn object_at(scene: &Scene, x: f32, y: f32) -> Result<Option<Id>, String> {
    let Some((world, triangle)) = seen_at(scene, x, y)? else { return Ok(None) };
    Ok(world.shading.get(triangle).and_then(|data| world.objects.get(data.object as usize)).copied())
}
