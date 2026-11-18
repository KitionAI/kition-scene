//! The Scene kit's path tracer: progressive, physically based final renders through the scene
//! camera, in the reference application's light units (`lights`). Each pass adds one sample to
//! every pixel; a pass can be split into rows across calls, so a page's worker can render a
//! little at a time. Samples are seeded by render, pixel, and pass, so the same scene and
//! options always give the same picture however the work is split.
//!
//! Cameras focus through a thin lens with a round or bladed opening; motion blur traces each
//! sample at a time while the shutter is open (what moves is kept at evenly spaced times across
//! the shutter, straight between them); media fill the world and closed meshes whose shader
//! group has a Volume (`volume`).
//!
//! The work can also be split across workers: each traces a shard (every `count`-th row, from
//! row `index`) and answers its rows' sums ([`Tracer::sums`]), which a collector without a scene
//! of its own merges ([`Tracer::merge`]). A row's sums are made the same way in a shard as in a
//! whole render, so the merged picture is the single tracer's, bit for bit.
#![forbid(unsafe_code)]

pub mod bsdf;
pub mod bvh;
mod denoise;
pub mod glass;
pub mod lights;
pub mod passes;
pub mod sampling;
pub mod volume;

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat3, Vec2, Vec3, Vec4};
use kition_scene_model::node_tree::Tree;
use kition_scene_model::{AlphaMode, Id, ObjectData, Scene};
use kition_scene_raster::cache::Prepared;
use kition_scene_raster::environment::Environment;
use kition_scene_raster::texture::{Texture, TextureCache};
use kition_scene_raster::view::{Projector, camera_view};
use kition_scene_shader::{Flat, Pictures, Point, Program, Space};
use serde::{Deserialize, Serialize};

use bsdf::Surface;
use bvh::{Bvh, MotionBvh, Triangle};
use glass::{Dielectric, Layered};
use lights::Shape;
use passes::{Sample, Sums as PassSums};
use sampling::{Frame, Rng, power_heuristic};
use volume::Medium;

/// How a render is made.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceOptions {
    /// Samples per pixel.
    pub samples: u32,
    /// The longest path, in bounces after the first hit.
    pub bounces: u32,
    /// Smooths the noise away, guided by surface colours and normals.
    pub denoise: bool,
    pub seed: u64,
    /// The brightest a sample of light that bounced may be (the reference clamps indirect light
    /// at 10 by default); 0 leaves it unclamped.
    pub clamp_indirect: f32,
    /// How often light may scatter in a volume before the path ends; 0 (the reference's
    /// default) lights each scattering point directly only.
    pub volume_bounces: u32,
    /// Keep the light passes (diffuse and glossy direct, indirect, and colour, and emission)
    /// as well as the picture.
    pub passes: bool,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self { samples: 64, bounces: 4, denoise: false, seed: 0, clamp_indirect: 10.0, volume_bounces: 0, passes: false }
    }
}

/// The largest sample count.
pub const MAX_SAMPLES: u32 = 4096;
/// The longest path.
pub const MAX_BOUNCES: u32 = 32;
/// The most triangles a render takes (a moving triangle counts once per time it is kept at).
pub const MAX_TRIANGLES: usize = 4_000_000;
/// The times across the shutter that moving objects and cameras are kept at.
pub const MOTION_STEPS: usize = 9;
/// The most surfaces of volumes a path passes through.
const MAX_CROSSINGS: u32 = 128;

/// How far a render has got.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// Complete passes (samples every pixel has).
    pub samples: u32,
    pub target: u32,
    /// From 0 to 1.
    pub fraction: f32,
    pub done: bool,
}

#[derive(Debug, Clone)]
struct MaterialData {
    base: Vec3,
    metallic: f32,
    roughness: f32,
    emission: Vec3,
    base_texture: Option<Arc<Texture>>,
    metallic_roughness_texture: Option<Arc<Texture>>,
    normal_texture: Option<Arc<Texture>>,
    normal_strength: f32,
    alpha: f32,
    alpha_mode: AlphaMode,
    alpha_cutoff: f32,
    transmission: f32,
    ior: f32,
    /// The shader group, when the material has one: it gives the surface in place of the
    /// values above.
    program: Option<Arc<Program>>,
    /// The group's surface may let light through (its values vary or it is not opaque).
    program_alpha: bool,
    /// Whether the material has a surface; one with only a volume lets light past its boundary.
    surface: bool,
    /// The volume inside the material's closed meshes.
    medium: Option<Medium>,
}

impl MaterialData {
    /// The reference's surface for objects without a material: grey 0.8.
    fn fallback() -> Self {
        Self {
            base: Vec3::splat(0.8),
            metallic: 0.0,
            roughness: 0.5,
            emission: Vec3::ZERO,
            base_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_strength: 1.0,
            alpha: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            transmission: 0.0,
            ior: 1.5,
            program: None,
            program_alpha: false,
            surface: true,
            medium: None,
        }
    }

    /// How much of the surface is there, from the alpha at a point (alpha × the image's).
    fn coverage(&self, alpha: f32) -> f32 {
        match self.alpha_mode {
            AlphaMode::Opaque => 1.0,
            AlphaMode::Clip => {
                if alpha >= self.alpha_cutoff {
                    1.0
                } else {
                    0.0
                }
            }
            AlphaMode::Blend => alpha.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Shading {
    normals: [Vec3; 3],
    uvs: Option<[Vec2; 3]>,
    /// World tangents with the bitangent sign in `w`, when the material has a normal map.
    tangents: Option<[Vec4; 3]>,
    /// The triangle's outward normal (its winding's side).
    geometric: Vec3,
    material: u32,
    /// The object's index in `World::spaces`.
    object: u32,
}

/// A thin lens: depth of field for perspective cameras.
#[derive(Debug, Clone, Copy)]
struct Lens {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// The opening's radius in metres.
    radius: f32,
    /// Metres along `forward` that are sharp.
    focus: f32,
    /// A polygonal opening's blades (0: round), their turn, and the height-over-width ratio.
    blades: u32,
    rotation: f32,
    ratio: f32,
}

impl Lens {
    /// A point on the opening, relative to its centre in the lens plane (right, up), from two
    /// uniform numbers: evenly spread over the disk or the polygon.
    fn point(&self, u: Vec2) -> Vec2 {
        let unit = if self.blades >= 3 {
            // A polygon with corners on the unit circle at π/n + 2πk/n + rotation (measured
            // from the reference's bokeh): pick one of its n triangles, then a point in it.
            let n = self.blades as f32;
            let pick = u.x * n;
            let k = pick.floor().min(n - 1.0);
            let (s, t) = ((pick - k).min(1.0), u.y);
            let corner = |index: f32| {
                let angle = std::f32::consts::PI / n + std::f32::consts::TAU * index / n + self.rotation;
                Vec2::new(angle.cos(), angle.sin())
            };
            // Uniform in the triangle (centre, corner k, corner k + 1).
            let root = s.sqrt();
            corner(k) * (root * (1.0 - t)) + corner(k + 1.0) * (root * t)
        } else {
            sampling::concentric_disk(u)
        };
        let ratio = if self.ratio.is_finite() && self.ratio > 0.0 { self.ratio } else { 1.0 };
        Vec2::new(unit.x / ratio, unit.y) * self.radius
    }
}

/// The camera at one time: its projection and lens.
#[derive(Debug, Clone)]
struct Eye {
    camera: Projector,
    lens: Option<Lens>,
}

/// What the tracer renders: the scene's surfaces, materials, lights, world, and camera.
#[derive(Debug)]
struct World {
    bvh: Bvh,
    shading: Vec<Shading>,
    materials: Vec<MaterialData>,
    lights: Vec<Shape>,
    background: Vec3,
    /// The world image, lighting the scene in place of `background`.
    environment: Option<Arc<Environment>>,
    /// Some material lets light past by its alpha, or has only a volume: shadow rays look at
    /// what they hit.
    alpha: bool,
    /// The camera at each time across the shutter (one when it does not move).
    eyes: Vec<Eye>,
    /// Triangles that move while the shutter is open; their shading follows the still ones'
    /// in `shading` (hit indices from `still` on).
    motion: MotionBvh,
    /// The moving triangles at each step (`moving[i * steps + s]`), and their corner normals.
    moving: Vec<Triangle>,
    moving_normals: Vec<[Vec3; 3]>,
    steps: usize,
    /// How many triangles do not move.
    still: u32,
    /// Each object's space, for shader groups, and the object it is.
    spaces: Vec<Space>,
    objects: Vec<Id>,
    /// The material each material slot is (none for the fallback surface).
    material_ids: Vec<Option<Id>>,
    /// The world's volume.
    medium: Option<Medium>,
    /// Where lights that are not suns are, for sampling points in media near them.
    light_points: Vec<Vec3>,
    /// Some medium scatters light.
    media: bool,
}

/// The first bytes of a shard's sums.
const SUMS_MAGIC: &[u8; 4] = b"KCTS";
/// The layout of the sums.
const SUMS_VERSION: u32 = 2;

/// A render in progress.
#[derive(Debug)]
pub struct Tracer {
    /// What is rendered; `None` for a collector, which only merges shards' sums.
    world: Option<World>,
    options: TraceOptions,
    width: u32,
    height: u32,
    /// Summed radiance per pixel.
    sum: Vec<Vec3>,
    /// The first surface's colour and normal per pixel (summed), to guide the denoiser.
    albedo: Vec<Vec3>,
    normal: Vec<Vec3>,
    /// The light passes' sums, when the options ask for them.
    passes: Option<PassSums>,
    /// The rows this tracer renders (every row, or a shard's), in order.
    rows: Vec<u32>,
    /// Samples each row has.
    counts: Vec<u32>,
    /// Complete passes over `rows`, and the next of them in the pass under way.
    pass: u32,
    cursor: usize,
    /// How the picture's light is shown (the scene's colour management).
    display: kition_scene_model::color::ColorManagement,
}

impl Tracer {
    /// Prepares a render of `scene` through its camera at its render size.
    pub fn new(scene: &Scene, options: TraceOptions) -> Result<Self, String> {
        Self::shard(scene, options, 0, 1)
    }

    /// Prepares shard `index` of `count` of a render: every `count`-th row from row `index`.
    pub fn shard(scene: &Scene, options: TraceOptions, index: u32, count: u32) -> Result<Self, String> {
        if count == 0 || index >= count {
            return Err("invalid-params: the shard is not one of the render's".into());
        }
        let mut tracer = Self::collector(scene, options)?;
        let camera_id = scene.camera.ok_or("not-allowed: the scene has no camera")?;
        let (width, height) = (tracer.width, tracer.height);
        let camera = camera_view(scene, camera_id).ok_or("not-allowed: the scene camera is not a camera")?.fit(width, height, width, height);
        tracer.world = Some(compile(scene, camera, Some(camera_id), None, true)?);
        tracer.rows = (index..height).step_by(count as usize).collect();
        Ok(tracer)
    }

    /// A render that traces nothing itself and is made of its shards' sums ([`Tracer::merge`]).
    pub fn collector(scene: &Scene, options: TraceOptions) -> Result<Self, String> {
        scene.camera.ok_or("not-allowed: the scene has no camera")?;
        let mut tracer = Self::empty(scene.render.width, scene.render.height, options)?;
        tracer.display = scene.render.color_management;
        Ok(tracer)
    }

    fn empty(width: u32, height: u32, options: TraceOptions) -> Result<Self, String> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 8192 * 8192 {
            return Err("invalid-params: the render size is out of range".into());
        }
        let options = TraceOptions {
            samples: options.samples.clamp(1, MAX_SAMPLES),
            bounces: options.bounces.min(MAX_BOUNCES),
            clamp_indirect: if options.clamp_indirect.is_finite() { options.clamp_indirect.max(0.0) } else { 0.0 },
            ..options
        };
        let count = width as usize * height as usize;
        Ok(Self {
            world: None,
            options,
            width,
            height,
            sum: vec![Vec3::ZERO; count],
            albedo: vec![Vec3::ZERO; count],
            normal: vec![Vec3::ZERO; count],
            passes: options.passes.then(|| PassSums::new(count)),
            rows: Vec::new(),
            counts: vec![0; height as usize],
            pass: 0,
            cursor: 0,
            display: kition_scene_model::color::ColorManagement::default(),
        })
    }

    /// A still viewport through an arbitrary projection, using exactly the viewport's
    /// evaluated meshes. Camera views keep depth of field; orbit views have a pinhole.
    /// Motion blur and compositing belong to final renders, not interactive viewports.
    pub fn viewport(
        scene: &Scene,
        camera: Projector,
        camera_id: Option<Id>,
        options: TraceOptions,
        meshes: &HashMap<Id, Arc<Prepared>>,
    ) -> Result<Self, String> {
        if !camera.view.is_finite() || !camera.projection.is_finite() {
            return Err("invalid-params: the viewport projection must be finite".into());
        }
        let mut tracer = Self::empty(camera.width, camera.height, options)?;
        tracer.display = scene.render.color_management;
        tracer.rows = (0..camera.height).collect();
        tracer.world = Some(compile(scene, camera, camera_id, Some(meshes), false)?);
        Ok(tracer)
    }

    /// Changes the sample target without rebuilding geometry, lights, textures, or the BVH.
    /// Lowering the target starts a new accumulation with the same compiled scene.
    pub fn set_samples(&mut self, samples: u32) {
        let samples = samples.clamp(1, MAX_SAMPLES);
        if samples < self.options.samples {
            self.sum.fill(Vec3::ZERO);
            self.albedo.fill(Vec3::ZERO);
            self.normal.fill(Vec3::ZERO);
            self.counts.fill(0);
            self.pass = 0;
            self.cursor = 0;
            self.passes = self.options.passes.then(|| PassSums::new(self.sum.len()));
        }
        self.options.samples = samples;
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn options(&self) -> TraceOptions {
        self.options
    }

    /// How far the render has got: over its own rows, or for a collector, every row.
    pub fn progress(&self) -> Progress {
        let target = self.options.samples;
        let counts: Vec<u32> = if self.world.is_some() {
            self.rows.iter().map(|y| self.counts.get(*y as usize).copied().unwrap_or(0)).collect()
        } else {
            self.counts.clone()
        };
        let samples = counts.iter().copied().min().unwrap_or(target).min(target);
        let rows: f64 = counts.iter().map(|count| f64::from((*count).min(target))).sum();
        let total = f64::from(target) * counts.len() as f64;
        Progress { samples, target, fraction: (rows / total.max(1.0)).min(1.0) as f32, done: samples >= target }
    }

    /// Renders `rows` more rows of the pass under way (starting the next pass as one ends).
    pub fn step_rows(&mut self, rows: u32) -> Progress {
        for _ in 0..rows {
            if self.pass >= self.options.samples || self.rows.is_empty() {
                break;
            }
            if let Some(y) = self.rows.get(self.cursor).copied() {
                self.trace_row(y);
                if let Some(count) = self.counts.get_mut(y as usize) {
                    *count += 1;
                }
            }
            self.cursor += 1;
            if self.cursor >= self.rows.len() {
                self.cursor = 0;
                self.pass += 1;
            }
        }
        self.progress()
    }

    /// This tracer's rows so far, for a collector to merge: a header ("KCTS", the layout, the
    /// width, height, samples, the number of buffers, and the number of rows), then per row its
    /// index, its samples, and per buffer (radiance, albedo, normal, then the light passes when
    /// kept) its sums (RGB, little-endian f32) per pixel.
    pub fn sums(&self) -> Vec<u8> {
        let width = self.width as usize;
        let buffers = self.buffers();
        let mut out = Vec::with_capacity(28 + self.rows.len() * (8 + width * 12 * buffers.len()));
        out.extend(SUMS_MAGIC);
        for value in [SUMS_VERSION, self.width, self.height, self.options.samples, buffers.len() as u32, self.rows.len() as u32] {
            out.extend(value.to_le_bytes());
        }
        for y in &self.rows {
            out.extend(y.to_le_bytes());
            out.extend(self.counts.get(*y as usize).copied().unwrap_or(0).to_le_bytes());
            let start = *y as usize * width;
            for buffer in &buffers {
                for pixel in buffer.get(start..start + width).unwrap_or(&[]) {
                    for channel in pixel.to_array() {
                        out.extend(channel.to_le_bytes());
                    }
                }
            }
        }
        out
    }

    /// The buffers the sums carry.
    fn buffers(&self) -> Vec<&Vec<Vec3>> {
        let mut out = vec![&self.sum, &self.albedo, &self.normal];
        if let Some(passes) = &self.passes {
            out.extend(passes.buffers.iter());
        }
        out
    }

    /// Takes a shard's sums ([`Tracer::sums`]) in place of what this tracer had for its rows.
    pub fn merge(&mut self, sums: &[u8]) -> Result<Progress, String> {
        let bad = || "invalid-params: these are not this render's sums".to_string();
        let mut at = 0usize;
        let word = |at: &mut usize| -> Result<u32, String> {
            let bytes: [u8; 4] = sums.get(*at..*at + 4).and_then(|bytes| bytes.try_into().ok()).ok_or_else(bad)?;
            *at += 4;
            Ok(u32::from_le_bytes(bytes))
        };
        if sums.get(..4) != Some(SUMS_MAGIC.as_slice()) {
            return Err(bad());
        }
        at += 4;
        let header = [word(&mut at)?, word(&mut at)?, word(&mut at)?, word(&mut at)?, word(&mut at)?];
        let count = self.buffers().len();
        if header != [SUMS_VERSION, self.width, self.height, self.options.samples, count as u32] {
            return Err(bad());
        }
        let rows = word(&mut at)? as usize;
        let width = self.width as usize;
        if rows > self.height as usize || sums.len() != 28 + rows * (8 + width * 12 * count) {
            return Err(bad());
        }
        for _ in 0..rows {
            let y = word(&mut at)? as usize;
            let samples = word(&mut at)?;
            if y >= self.height as usize || samples > self.options.samples {
                return Err(bad());
            }
            if let Some(slot) = self.counts.get_mut(y) {
                *slot = samples;
            }
            let mut buffers = vec![&mut self.sum, &mut self.albedo, &mut self.normal];
            if let Some(passes) = &mut self.passes {
                buffers.extend(passes.buffers.iter_mut());
            }
            for buffer in buffers {
                for x in 0..width {
                    let mut channels = [0.0f32; 3];
                    for channel in &mut channels {
                        *channel = f32::from_bits(word(&mut at)?);
                    }
                    if let Some(pixel) = buffer.get_mut(y * width + x) {
                        *pixel = Vec3::from_array(channels);
                    }
                }
            }
        }
        Ok(self.progress())
    }

    /// The light passes so far, by the Render Layers output that shows each (`Diffuse Direct`,
    /// ...), as linear RGBA per pixel; `None` unless the options asked for them. Direct and
    /// indirect light are divided by their colour, as the reference keeps them.
    pub fn passes(&self) -> Option<std::collections::BTreeMap<String, Vec<[f32; 4]>>> {
        let passes = self.passes.as_ref()?;
        let counts: Vec<f32> = (0..self.width as usize * self.height as usize)
            .map(|index| self.samples_at((index / self.width.max(1) as usize) as u32).max(1) as f32)
            .collect();
        Some(passes.finish(&counts))
    }

    /// Renders for about `budget_ms` milliseconds by the clock `now` (milliseconds), at least
    /// one row.
    pub fn step(&mut self, budget_ms: f64, now: &dyn Fn() -> f64) -> Progress {
        let start = now();
        loop {
            let progress = self.step_rows(1);
            if progress.done || now() - start >= budget_ms {
                return progress;
            }
        }
    }

    /// Advances a viewport until its next complete sample or the cooperative time budget.
    /// Stopping at pass boundaries lets the editor publish whole passes without doing a full
    /// raster composition for every partial row batch.
    pub fn step_preview(&mut self, budget_ms: f64, now: &dyn Fn() -> f64) -> Progress {
        let started = now();
        let samples = self.progress().samples;
        // A row cap also yields when a host clock stalls or moves backwards.
        for _ in 0..32 {
            let progress = self.step_rows(1);
            if progress.done || progress.samples != samples || now() - started >= budget_ms {
                return progress;
            }
        }
        self.progress()
    }

    /// Renders to the end: the pass under way row by row, then each remaining pass's rows on
    /// every core (outside the web page, which has one), each pixel summed in the same order, so
    /// the picture is the one stepping row by row makes.
    pub fn finish(&mut self) -> Progress {
        let threads = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get) };
        if threads > 1 && self.passes.is_none() && self.world.is_some() {
            while self.cursor != 0 && self.pass < self.options.samples {
                self.step_rows(1);
            }
            while self.pass < self.options.samples && !self.rows.is_empty() {
                self.parallel_pass(threads);
            }
            return self.progress();
        }
        let rows = (self.options.samples.saturating_sub(self.pass)).saturating_mul(self.rows.len() as u32);
        self.step_rows(rows.saturating_add(1))
    }

    /// One whole pass over this tracer's rows, shared out between `threads` threads a row at a
    /// time (rows through the scene take longer than rows of sky).
    fn parallel_pass(&mut self, threads: usize) {
        let Some(world) = &self.world else { return };
        let (options, width, rows, counts) = (&self.options, self.width, &self.rows, &self.counts);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let traced: Vec<(u32, Vec<[Vec3; 3]>)> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads.min(rows.len()))
                .map(|_| {
                    scope.spawn(|| {
                        let mut done = Vec::new();
                        while let Some(&y) = rows.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
                            done.push((y, row_samples(world, options, width, y, counts.get(y as usize).copied().unwrap_or(0))));
                        }
                        done
                    })
                })
                .collect();
            workers.into_iter().flat_map(|worker| worker.join().unwrap_or_default()).collect()
        });
        for (y, samples) in traced {
            let row = y as usize * width as usize;
            for (x, [radiance, albedo, normal]) in samples.into_iter().enumerate() {
                if let Some(sum) = self.sum.get_mut(row + x) {
                    *sum += radiance;
                }
                if let Some(stored) = self.albedo.get_mut(row + x) {
                    *stored += albedo;
                }
                if let Some(stored) = self.normal.get_mut(row + x) {
                    *stored += normal;
                }
            }
            if let Some(count) = self.counts.get_mut(y as usize) {
                *count += 1;
            }
        }
        self.pass += 1;
    }

    fn samples_at(&self, y: u32) -> u32 {
        self.counts.get(y as usize).copied().unwrap_or(0)
    }

    /// The picture so far as linear RGB per pixel (denoised when the options ask and the render
    /// is done).
    pub fn linear(&self) -> Vec<Vec3> {
        let mut colour = Vec::with_capacity(self.sum.len());
        let denoise = self.options.denoise && self.progress().done;
        let mut albedo = Vec::with_capacity(if denoise { self.sum.len() } else { 0 });
        let mut normal = Vec::with_capacity(if denoise { self.sum.len() } else { 0 });
        for y in 0..self.height {
            let count = self.samples_at(y).max(1) as f32;
            let row = y as usize * self.width as usize;
            for x in 0..self.width as usize {
                let slot = row + x;
                colour.push(self.sum.get(slot).copied().unwrap_or(Vec3::ZERO) / count);
                if denoise {
                    albedo.push(self.albedo.get(slot).copied().unwrap_or(Vec3::ZERO) / count);
                    normal.push(self.normal.get(slot).copied().unwrap_or(Vec3::ZERO).normalize_or_zero());
                }
            }
        }
        if denoise { denoise::atrous(&colour, &albedo, &normal, self.width, self.height) } else { colour }
    }

    /// The picture so far, as straight-alpha sRGB RGBA8, through the scene's colour management.
    pub fn image(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.sum.len() * 4);
        for pixel in self.linear() {
            let [r, g, b] = self.display.apply(pixel.to_array()).map(kition_scene_raster::draw::encode);
            out.extend([r, g, b, 255]);
        }
        out
    }

    fn trace_row(&mut self, y: u32) {
        let width = self.width;
        let Some(world) = &self.world else { return };
        let pass = self.counts.get(y as usize).copied().unwrap_or(0);
        if self.passes.is_none() {
            let row = y as usize * width as usize;
            for (x, [radiance, albedo, normal]) in row_samples(world, &self.options, width, y, pass).into_iter().enumerate() {
                if let Some(sum) = self.sum.get_mut(row + x) {
                    *sum += radiance;
                }
                if let Some(stored) = self.albedo.get_mut(row + x) {
                    *stored += albedo;
                }
                if let Some(stored) = self.normal.get_mut(row + x) {
                    *stored += normal;
                }
            }
            return;
        }
        for x in 0..width {
            let pixel = u64::from(y) * u64::from(width) + u64::from(x);
            let mut rng = Rng::for_sample(self.options.seed, pixel, u64::from(pass));
            let jitter = rng.next2();
            let lens_sample = world.eyes.iter().any(|eye| eye.lens.is_some()).then(|| rng.next2());
            let time = if world.steps > 1 { rng.uniform() } else { 0.5 };
            let (origin, direction) = world.camera_ray(x as f32 + jitter.x, y as f32 + jitter.y, lens_sample, time);
            let mut sample = self.passes.as_ref().map(|_| Sample::default());
            let (radiance, albedo, normal) = radiance(world, &self.options, origin, direction, time, &mut rng, sample.as_mut());
            if let (Some(passes), Some(sample)) = (self.passes.as_mut(), sample) {
                passes.add(pixel as usize, &sample);
            }
            let slot = pixel as usize;
            if let Some(sum) = self.sum.get_mut(slot) {
                *sum += if radiance.is_finite() { radiance } else { Vec3::ZERO };
            }
            if let Some(stored) = self.albedo.get_mut(slot) {
                *stored += albedo;
            }
            if let Some(stored) = self.normal.get_mut(slot) {
                *stored += normal;
            }
        }
    }
}

/// Sample `pass` of each pixel of row `y`: its radiance (zero when not finite), first-surface
/// colour and normal, as `Tracer::trace_row` adds them up.
fn row_samples(world: &World, options: &TraceOptions, width: u32, y: u32, pass: u32) -> Vec<[Vec3; 3]> {
    (0..width)
        .map(|x| {
            let pixel = u64::from(y) * u64::from(width) + u64::from(x);
            let mut rng = Rng::for_sample(options.seed, pixel, u64::from(pass));
            let jitter = rng.next2();
            let lens_sample = world.eyes.iter().any(|eye| eye.lens.is_some()).then(|| rng.next2());
            let time = if world.steps > 1 { rng.uniform() } else { 0.5 };
            let (origin, direction) = world.camera_ray(x as f32 + jitter.x, y as f32 + jitter.y, lens_sample, time);
            let (radiance, albedo, normal) = radiance(world, options, origin, direction, time, &mut rng, None);
            [if radiance.is_finite() { radiance } else { Vec3::ZERO }, albedo, normal]
        })
        .collect()
}

impl Eye {
    /// The ray through picture point (`x`, `y`), through the lens point `lens` (two uniform
    /// numbers) when the camera has depth of field.
    fn ray(&self, x: f32, y: f32, lens_sample: Option<Vec2>) -> (Vec3, Vec3) {
        let (mut origin, mut direction) = self.camera.ray(x, y);
        if let (Some(lens), Some(u)) = (&self.lens, lens_sample) {
            // Through a point on the lens towards where the pinhole ray meets the focal plane.
            let along = direction.dot(lens.forward);
            if along > 1e-6 {
                let focal = lens.eye + direction * (lens.focus / along);
                let at = lens.point(u);
                origin = lens.eye + lens.right * at.x + lens.up * at.y;
                direction = (focal - origin).normalize_or_zero();
            }
        }
        (origin, direction)
    }
}

impl World {
    /// The camera's ray at `time` (0 to 1 across the shutter): between the eyes on either side.
    fn camera_ray(&self, x: f32, y: f32, lens: Option<Vec2>, time: f32) -> (Vec3, Vec3) {
        let first = self.eyes.first().map_or((Vec3::ZERO, Vec3::NEG_Z), |eye| eye.ray(x, y, lens));
        if self.eyes.len() < 2 {
            return first;
        }
        let (index, share) = MotionBvh::bracket(self.eyes.len(), time);
        let (Some(a), Some(b)) = (self.eyes.get(index), self.eyes.get(index + 1)) else { return first };
        let ((oa, da), (ob, db)) = (a.ray(x, y, lens), b.ray(x, y, lens));
        (oa.lerp(ob, share), da.lerp(db, share).normalize_or_zero())
    }

    /// The nearest triangle along a ray at `time`, still or moving.
    fn intersect(&self, origin: Vec3, direction: Vec3, far: f32, any: bool, time: f32) -> Option<bvh::Hit> {
        let still = self.bvh.intersect(origin, direction, 0.0, far, any);
        if self.motion.is_empty() || (any && still.is_some()) {
            return still;
        }
        let far = still.map_or(far, |hit| hit.distance);
        let moving = self.motion.intersect(origin, direction, 0.0, far, any, time).map(|hit| bvh::Hit { triangle: hit.triangle + self.still, ..hit });
        moving.or(still)
    }

    /// A moving triangle's corners and normals at `time`.
    fn moving_at(&self, index: usize, time: f32) -> Option<(Triangle, [Vec3; 3])> {
        let steps = self.steps.max(1);
        let base = index * steps;
        if steps == 1 {
            return Some((*self.moving.get(base)?, *self.moving_normals.get(base)?));
        }
        let (first, share) = MotionBvh::bracket(steps, time);
        let (a, b) = (self.moving.get(base + first)?, self.moving.get(base + first + 1)?);
        let (na, nb) = (self.moving_normals.get(base + first)?, self.moving_normals.get(base + first + 1)?);
        let triangle = Triangle { a: a.a.lerp(b.a, share), edge1: a.edge1.lerp(b.edge1, share), edge2: a.edge2.lerp(b.edge2, share) };
        let [a0, a1, a2] = *na;
        let [b0, b1, b2] = *nb;
        Some((triangle, [a0.lerp(b0, share), a1.lerp(b1, share), a2.lerp(b2, share)].map(Vec3::normalize_or_zero)))
    }

    /// The outward normal and corner normals of triangle `index` at `time`.
    fn normals_at(&self, index: u32, data: &Shading, time: f32) -> (Vec3, [Vec3; 3]) {
        let Some(moving) = index.checked_sub(self.still) else { return (data.geometric, data.normals) };
        let Some((triangle, normals)) = self.moving_at(moving as usize, time) else { return (data.geometric, data.normals) };
        let mut geometric = triangle.edge1.cross(triangle.edge2).normalize_or_zero();
        // The still copy's outward normal says which side the winding faces.
        if geometric.dot(data.geometric) < 0.0 {
            geometric = -geometric;
        }
        (if geometric == Vec3::ZERO { data.geometric } else { geometric }, normals)
    }

    /// The media a path starts in from the camera: the world's.
    fn medium_of(&self, inside: &[u32]) -> Option<Medium> {
        let mut out = self.medium.clone();
        for slot in inside {
            if let Some(medium) = self.materials.get(*slot as usize).and_then(|material| material.medium.as_ref()) {
                match &mut out {
                    Some(sum) => sum.add(medium),
                    None => out = Some(medium.clone()),
                }
            }
        }
        out
    }
}

/// One object's triangles in world space at one time: corners, corner normals, and outward
/// normals (empty when it has no surface).
struct Posed {
    triangles: Vec<Triangle>,
    normals: Vec<[Vec3; 3]>,
    geometric: Vec<Vec3>,
}

/// An object's surface as a renderer takes it, posed by its world matrix.
fn pose(prepared: &Prepared, world: glam::Mat4) -> Posed {
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    let mirrored = world.determinant() < 0.0;
    let positions: Vec<Vec3> = prepared.mesh.positions.iter().map(|point| world.transform_point3(*point)).collect();
    let vertex_normals: Vec<Vec3> = prepared.vertex_normals.iter().map(|normal| (normal_matrix * *normal).normalize_or_zero()).collect();
    let mut posed = Posed { triangles: Vec::new(), normals: Vec::new(), geometric: Vec::new() };
    for (number, (corners, face)) in prepared.triangles.iter().enumerate() {
        let point = |corner: u32| positions.get(corner as usize).copied();
        let (Some(a), Some(b), Some(c)) = (point(corners[0]), point(corners[1]), point(corners[2])) else {
            // Kept in step with the triangle list, so poses at other times line up.
            posed.triangles.push(Triangle::new(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO));
            posed.normals.push([Vec3::ZERO; 3]);
            posed.geometric.push(Vec3::ZERO);
            continue;
        };
        let mut geometric = (b - a).cross(c - a).normalize_or_zero();
        let face_normal = prepared.face_normals.get(*face as usize).map_or(geometric, |normal| (normal_matrix * *normal).normalize_or_zero());
        if mirrored {
            geometric = -geometric;
        }
        let normals = if let Some(split) = prepared.split_normals.get(number) {
            split.map(|normal| (normal_matrix * normal).normalize_or_zero())
        } else if vertex_normals.is_empty() {
            [face_normal; 3]
        } else {
            corners.map(|corner| vertex_normals.get(corner as usize).copied().unwrap_or(face_normal))
        };
        posed.triangles.push(Triangle::new(a, b, c));
        posed.normals.push(normals);
        posed.geometric.push(geometric);
    }
    posed
}

/// The frames the shutter is open between, when motion blur is on: the scene at each step.
fn shutter_scenes(scene: &Scene) -> Option<Vec<Scene>> {
    let frame = scene.animation.timeline.frame as f32;
    let (open, close) = scene.render.motion_blur.interval(frame)?;
    let span = (MOTION_STEPS - 1) as f32;
    Some((0..MOTION_STEPS).map(|step| scene.at_frame(open + (close - open) * step as f32 / span)).collect())
}

/// The volume a shader group's Material Output gives, evaluated at `point` in `space` (media are
/// homogeneous: one value for the whole volume).
fn medium_of(program: &Program, point: Vec3, space: &Space) -> Option<Medium> {
    if !program.has_volume() {
        return None;
    }
    Medium::of(&program.eval_volume(&Point::at(point, Vec3::Z), space))
}

/// Builds the render's world: every visible mesh after its modifiers in world space (at each
/// time across the shutter for what moves), materials with their decoded images and shader
/// groups, lights, the world image and volume, and the camera's lens.
fn compile(
    scene: &Scene,
    camera: Projector,
    camera_id: Option<Id>,
    meshes: Option<&HashMap<Id, Arc<Prepared>>>,
    motion_blur: bool,
) -> Result<World, String> {
    let mut textures = TextureCache::default();
    let mut pictures = Pictures::default();
    let mut material_slots: HashMap<Option<Id>, u32> = HashMap::new();
    let mut materials: Vec<MaterialData> = Vec::new();
    let mut triangles = Vec::new();
    let mut shading = Vec::new();
    let mut spaces = Vec::new();
    let mut objects = Vec::new();
    let mut material_ids = Vec::new();
    // Moving triangles, kept apart until the still ones are all in.
    let mut moving_shading: Vec<Shading> = Vec::new();
    let mut moving: Vec<Triangle> = Vec::new();
    let mut moving_normals: Vec<[Vec3; 3]> = Vec::new();
    let shutter = if motion_blur { shutter_scenes(scene) } else { None };
    let steps = shutter.as_ref().map_or(1, Vec::len);
    for (object_index, object) in scene.objects.iter().enumerate().filter(|(_, object)| !object.hidden) {
        if !object.has_geometry() {
            continue;
        }
        let prepared = match meshes {
            Some(meshes) => match meshes.get(&object.id) {
                Some(prepared) => Arc::clone(prepared),
                None => continue,
            },
            None => match object.evaluated_mesh(scene) {
                Some(mesh) => Arc::new(Prepared::new(mesh)),
                None => continue,
            },
        };
        let Ok(world) = scene.world_matrix(object.id) else { continue };
        let space_index = u32::try_from(spaces.len()).unwrap_or(0);
        let local = prepared.mesh.bounds(glam::Mat4::IDENTITY);
        let space = Space::new(world, local.min, local.max);
        spaces.push(space);
        objects.push(object.id);
        let slot = *material_slots.entry(object.material).or_insert_with(|| {
            material_ids.push(object.material);
            let mut data = object.material.and_then(|id| scene.material(id).ok()).map_or_else(MaterialData::fallback, |material| MaterialData {
                base: Vec3::new(material.base_color[0], material.base_color[1], material.base_color[2]),
                metallic: material.metallic,
                roughness: material.roughness,
                emission: material.emission * material.emission_strength,
                base_texture: material.base_color_texture.as_ref().and_then(|hash| textures.get(scene, hash, true)),
                metallic_roughness_texture: material.metallic_roughness_texture.as_ref().and_then(|hash| textures.get(scene, hash, false)),
                normal_texture: material.normal_texture.as_ref().and_then(|hash| textures.get(scene, hash, false)),
                normal_strength: material.normal_strength,
                alpha: material.base_color[3],
                alpha_mode: material.alpha_mode,
                alpha_cutoff: material.alpha_cutoff,
                transmission: material.transmission.clamp(0.0, 1.0),
                ior: if material.ior.is_finite() && material.ior > 0.0 { material.ior } else { 1.5 },
                program: material
                    .shader
                    .and_then(|id| scene.node_group(id))
                    .filter(|group| group.tree == Tree::Shader)
                    .map(|group| Arc::new(Program::new(scene, group, &mut pictures))),
                program_alpha: false,
                surface: true,
                medium: None,
            });
            if let Some(program) = data.program.clone() {
                data.surface = program.has_surface() || !program.has_volume();
                data.medium = medium_of(&program, world.transform_point3((local.min + local.max) * 0.5), &space);
                data.program_alpha =
                    data.surface && (program.varying() || program.eval(&Point::at(Vec3::ZERO, Vec3::Z), &space).flatten().alpha < 1.0);
            }
            materials.push(data);
            u32::try_from(materials.len() - 1).unwrap_or(0)
        });
        let wants_tangents = materials.get(slot as usize).is_some_and(|material| material.normal_texture.is_some());
        let turn = Mat3::from_mat4(world);
        let mirrored = world.determinant() < 0.0;
        let posed = pose(&prepared, world);
        // The object at each time across the shutter, when it moves and keeps its triangles.
        let poses: Option<Vec<Posed>> = shutter.as_ref().and_then(|scenes| {
            let poses: Vec<Posed> = scenes
                .iter()
                .filter_map(|at| {
                    let other = at.objects.get(object_index).filter(|other| other.id == object.id)?;
                    let mesh = other.evaluated_mesh(at)?;
                    Some(pose(&Prepared::new(mesh), at.world_matrix(object.id).ok()?))
                })
                .collect();
            let fits = poses.len() == scenes.len() && poses.iter().all(|other| other.triangles.len() == posed.triangles.len());
            let moves = poses
                .iter()
                .any(|other| other.triangles.iter().zip(&posed.triangles).any(|(a, b)| a.a != b.a || a.edge1 != b.edge1 || a.edge2 != b.edge2));
            (fits && moves).then_some(poses)
        });
        let object_tangents = if wants_tangents { prepared.triangle_tangents() } else { &[] };
        for (index, ((triangle, normals), geometric)) in posed.triangles.iter().zip(&posed.normals).zip(&posed.geometric).enumerate() {
            if *geometric == Vec3::ZERO && poses.is_none() {
                continue;
            }
            // World tangents as the fast renderer makes them (a mirror flips the bitangent).
            let tangents = object_tangents.get(index).map(|corner_tangents| {
                corner_tangents.map(|tangent| (turn * tangent.truncate()).normalize_or_zero().extend(tangent.w * if mirrored { -1.0 } else { 1.0 }))
            });
            let data = Shading {
                normals: *normals,
                uvs: prepared.triangle_uvs.get(index).copied(),
                tangents,
                geometric: *geometric,
                material: slot,
                object: space_index,
            };
            match &poses {
                Some(poses) => {
                    moving_shading.push(data);
                    for other in poses {
                        moving.push(other.triangles.get(index).copied().unwrap_or(*triangle));
                        moving_normals.push(other.normals.get(index).copied().unwrap_or(*normals));
                    }
                }
                None => {
                    triangles.push(*triangle);
                    shading.push(data);
                }
            }
            if triangles.len() + moving.len() > MAX_TRIANGLES {
                return Err("invalid-params: the scene has too many triangles to path trace".into());
            }
        }
    }
    let still = u32::try_from(triangles.len()).map_err(|_| "invalid-params: the scene has too many triangles to path trace")?;
    shading.extend(moving_shading);
    let background = scene.world.color.max(Vec3::ZERO) * scene.world.strength.max(0.0);
    let environment = Environment::of(scene).map(Arc::new);
    let medium = scene
        .world
        .volume
        .and_then(|id| scene.node_group(id))
        .filter(|group| group.tree == Tree::Shader)
        .and_then(|group| medium_of(&Program::new(scene, group, &mut pictures), Vec3::ZERO, &Space::identity()));
    let alpha = materials.iter().any(|material| material.alpha_mode != AlphaMode::Opaque || material.program_alpha || !material.surface);
    let media = medium.is_some() || materials.iter().any(|material| material.medium.is_some());
    // The camera at each time, when it moves.
    let lens = camera_id.and_then(|id| lens_of(scene, id, &camera));
    let (width, height) = (camera.width, camera.height);
    let centre = Eye { camera, lens };
    let mut eyes = Vec::new();
    if let Some(scenes) = &shutter {
        let others: Vec<Eye> = scenes
            .iter()
            .filter_map(|at| {
                let id = camera_id?;
                let camera = camera_view(at, id)?.fit(width, height, width, height);
                let lens = lens_of(at, id, &camera);
                Some(Eye { camera, lens })
            })
            .collect();
        let moves = others.iter().any(|eye| eye.camera.view != centre.camera.view || eye.camera.projection != centre.camera.projection);
        if others.len() == scenes.len() && moves {
            eyes = others;
        }
    }
    if eyes.is_empty() {
        eyes.push(centre);
    }
    let (moving_is_empty, eyes_still) = (moving.is_empty(), eyes.len() < 2);
    let lights = lights::collect(scene);
    let light_points = lights
        .iter()
        .filter_map(|light| match light {
            Shape::Sphere { centre, .. } | Shape::Rect { centre, .. } => Some(*centre),
            Shape::Sun { .. } => None,
        })
        .collect();
    Ok(World {
        bvh: Bvh::build(triangles),
        shading,
        materials,
        lights,
        background,
        environment,
        alpha,
        eyes,
        motion: MotionBvh::build(&moving, steps),
        moving,
        moving_normals,
        steps: if moving_is_empty && eyes_still { 1 } else { steps },
        still,
        spaces,
        objects,
        material_ids,
        medium,
        media,
        light_points,
    })
}

/// The camera's thin lens, when depth of field is on (perspective cameras only).
fn lens_of(scene: &Scene, camera_id: Id, projector: &Projector) -> Option<Lens> {
    let ObjectData::Camera(camera) = &scene.object(camera_id).ok()?.data else { return None };
    if !camera.dof.enabled || camera.orthographic || camera.dof.f_stop.is_nan() || camera.dof.f_stop <= 0.0 {
        return None;
    }
    let to_world = projector.view.inverse();
    let eye = to_world.transform_point3(Vec3::ZERO);
    let (right, up, forward) = (
        to_world.transform_vector3(Vec3::X).normalize_or_zero(),
        to_world.transform_vector3(Vec3::Y).normalize_or_zero(),
        to_world.transform_vector3(Vec3::NEG_Z).normalize_or_zero(),
    );
    let focus = match camera.dof.focus_object.and_then(|id| scene.world_matrix(id).ok()) {
        Some(world) => (world.transform_point3(Vec3::ZERO) - eye).dot(forward),
        None => camera.dof.focus_distance,
    };
    // The reference's aperture: the focal length (millimetres) over twice the f-number.
    let radius = camera.lens * 1e-3 / (2.0 * camera.dof.f_stop);
    let blades = if camera.dof.blades >= 3 { camera.dof.blades.min(kition_scene_model::scene::MAX_BLADES) } else { 0 };
    (focus > 1e-4 && radius.is_finite()).then_some(Lens {
        eye,
        right,
        up,
        forward,
        radius,
        focus,
        blades,
        rotation: if camera.dof.rotation.is_finite() { camera.dof.rotation } else { 0.0 },
        ratio: camera.dof.ratio,
    })
}

struct SurfaceHit {
    position: Vec3,
    geometric: Vec3,
    shading: Vec3,
    layered: Layered,
    emission: Vec3,
    albedo: Vec3,
}

/// The texture coordinates at a point of a triangle.
fn uv_at(data: &Shading, u: f32, v: f32) -> Option<Vec2> {
    let [t0, t1, t2] = data.uvs?;
    Some(t0 * (1.0 - u - v) + t1 * u + t2 * v)
}

/// A shader group's surface at a point of a triangle, seen along `direction`.
fn program_at(
    world: &World,
    program: &Program,
    data: &Shading,
    position: Vec3,
    shading: Vec3,
    geometric: Vec3,
    direction: Vec3,
    u: f32,
    v: f32,
) -> Flat {
    let space = world.spaces.get(data.object as usize).copied().unwrap_or_else(Space::identity);
    let mut point = Point::at(position, shading);
    point.true_normal = geometric;
    point.incoming = -direction;
    point.uv = uv_at(data, u, v).unwrap_or(Vec2::ZERO);
    point.backfacing = data.geometric.dot(direction) > 0.0;
    if let Some([t0, t1, t2]) = data.tangents {
        point.tangent = t0 * (1.0 - u - v) + t1 * u + t2 * v;
    }
    program.eval(&point, &space).flatten()
}

/// How much of a triangle's surface is there at a point (alpha); 0 for a material with only a
/// volume.
fn coverage_at(world: &World, triangle: u32, u: f32, v: f32, origin: Vec3, direction: Vec3, distance: f32) -> f32 {
    let Some(data) = world.shading.get(triangle as usize) else { return 1.0 };
    let Some(material) = world.materials.get(data.material as usize) else { return 1.0 };
    if !material.surface {
        return 0.0;
    }
    if let Some(program) = &material.program {
        if !material.program_alpha {
            return 1.0;
        }
        let position = origin + direction * distance;
        return program_at(world, program, data, position, data.geometric, data.geometric, direction, u, v).alpha;
    }
    if material.alpha_mode == AlphaMode::Opaque {
        return 1.0;
    }
    let mut alpha = material.alpha;
    if let (Some(texture), Some(uv)) = (&material.base_texture, uv_at(data, u, v)) {
        alpha *= texture.sample(uv, 0.0).w;
    }
    material.coverage(alpha)
}

/// Where a path is: the volumes (material slots) it is inside, beyond the world's.
type Inside = Vec<u32>;

/// Crossing triangle `triangle` along `direction`: entering or leaving its material's volume.
fn cross(world: &World, inside: &mut Inside, triangle: u32, direction: Vec3, time: f32) {
    let Some(data) = world.shading.get(triangle as usize) else { return };
    if !bounds_medium(world, triangle) {
        return;
    }
    let (outward, _) = world.normals_at(triangle, data, time);
    if outward.dot(direction) < 0.0 {
        if inside.len() < 16 {
            inside.push(data.material);
        }
    } else if let Some(at) = inside.iter().rposition(|slot| *slot == data.material) {
        inside.remove(at);
    }
}

/// The nearest surface along a ray that is there: surfaces with alpha are passed through as
/// often as they are not there. A volume's boundary that is not there stops the search too
/// (the medium changes there): the answer says to pass it. Answers the hit, where its ray
/// started, and whether to pass it.
fn nearest(world: &World, mut origin: Vec3, direction: Vec3, time: f32, rng: &mut Rng) -> Option<(bvh::Hit, Vec3, bool)> {
    for _ in 0..MAX_CROSSINGS {
        let hit = world.intersect(origin, direction, f32::INFINITY, false, time)?;
        if !world.alpha || rng.uniform() < coverage_at(world, hit.triangle, hit.u, hit.v, origin, direction, hit.distance) {
            return Some((hit, origin, false));
        }
        if world.media && bounds_medium(world, hit.triangle) {
            return Some((hit, origin, true));
        }
        let geometric = world.shading.get(hit.triangle as usize).map_or(direction, |data| world.normals_at(hit.triangle, data, time).0);
        origin = offset(origin + direction * hit.distance, geometric, direction);
    }
    None
}

/// Whether a triangle's material has a volume.
fn bounds_medium(world: &World, triangle: u32) -> bool {
    world
        .shading
        .get(triangle as usize)
        .and_then(|data| world.materials.get(data.material as usize))
        .is_some_and(|material| material.medium.is_some())
}

/// How much light gets from `origin` along `direction` for `reach` metres, through the volumes
/// it starts `inside` and those it passes.
fn transmittance(world: &World, mut origin: Vec3, direction: Vec3, reach: f32, time: f32, inside: &Inside, rng: &mut Rng) -> Vec3 {
    if !world.alpha && !world.media {
        return if world.intersect(origin, direction, reach, true, time).is_none() { Vec3::ONE } else { Vec3::ZERO };
    }
    let mut inside = inside.clone();
    let mut through = Vec3::ONE;
    let mut left = reach;
    for _ in 0..MAX_CROSSINGS {
        let found = world.intersect(origin, direction, left, false, time);
        let segment = found.map_or(left, |hit| hit.distance);
        if world.media
            && let Some(medium) = world.medium_of(&inside)
        {
            through *= medium.transmittance(segment);
            if through.max_element() <= 0.0 {
                return Vec3::ZERO;
            }
        }
        let Some(hit) = found else { return through };
        if rng.uniform() < coverage_at(world, hit.triangle, hit.u, hit.v, origin, direction, hit.distance) {
            return Vec3::ZERO;
        }
        let geometric = world.shading.get(hit.triangle as usize).map_or(direction, |data| world.normals_at(hit.triangle, data, time).0);
        cross(world, &mut inside, hit.triangle, direction, time);
        let next = offset(origin + direction * hit.distance, geometric, direction);
        left -= next.distance(origin);
        origin = next;
        if left <= 0.0 {
            return through;
        }
    }
    Vec3::ZERO
}

#[allow(clippy::too_many_arguments)]
fn surface_at(world: &World, triangle: u32, u: f32, v: f32, origin: Vec3, direction: Vec3, distance: f32, time: f32) -> Option<SurfaceHit> {
    let data = world.shading.get(triangle as usize)?;
    let material = world.materials.get(data.material as usize)?;
    let w = 1.0 - u - v;
    let (outward, [n0, n1, n2]) = world.normals_at(triangle, data, time);
    let mut shading = (n0 * w + n1 * u + n2 * v).normalize_or_zero();
    if shading == Vec3::ZERO {
        shading = outward;
    }
    let position = origin + direction * distance;
    let uv = uv_at(data, u, v);
    let mut base = material.base;
    let (mut metallic, mut roughness) = (material.metallic, material.roughness);
    let (mut emission, mut transmission, mut ior) = (material.emission, material.transmission, material.ior);
    if let Some(program) = &material.program {
        // The shader group's surface, as one principled surface standing for its lobes.
        let entering = outward.dot(direction) < 0.0;
        let facing = if entering { outward } else { -outward };
        let seen = if shading.dot(facing) < 0.0 { -shading } else { shading };
        let flat = program_at(world, program, data, position, seen, facing, direction, u, v);
        base = flat.base;
        metallic = flat.metallic;
        roughness = flat.roughness;
        emission = flat.emission;
        transmission = flat.transmission.clamp(0.0, 1.0);
        ior = if flat.ior.is_finite() && flat.ior > 0.0 { flat.ior } else { 1.5 };
        if flat.normal != Vec3::ZERO {
            shading = flat.normal;
        }
    } else if let Some(uv) = uv {
        if let Some(texture) = &material.base_texture {
            base *= texture.sample(uv, 0.0).truncate();
        }
        if let Some(texture) = &material.metallic_roughness_texture {
            let sample = texture.sample(uv, 0.0);
            roughness *= sample.y;
            metallic *= sample.z;
        }
        // The normal map bends the shading normal in its tangent frame, as the fast renderer does.
        if let (Some(texture), Some([t0, t1, t2])) = (&material.normal_texture, data.tangents) {
            let tangent = t0 * w + t1 * u + t2 * v;
            let along = tangent.truncate();
            if along.length_squared() > 1e-12 {
                let tangent_axis = (along - shading * shading.dot(along)).normalize_or_zero();
                let bitangent = shading.cross(tangent_axis) * if tangent.w < 0.0 { -1.0 } else { 1.0 };
                let sample = texture.sample(uv, 0.0);
                let x = (sample.x * 2.0 - 1.0) * material.normal_strength;
                let y = (sample.y * 2.0 - 1.0) * material.normal_strength;
                let z = sample.z * 2.0 - 1.0;
                let bent = (tangent_axis * x + bitangent * y + shading * z).normalize_or_zero();
                if bent != Vec3::ZERO {
                    shading = bent;
                }
            }
        }
    }
    // Two-sided: the side the ray arrives on is the front.
    let entering = outward.dot(direction) < 0.0;
    let geometric = if entering { outward } else { -outward };
    if shading.dot(geometric) < 0.0 {
        shading = -shading;
    }
    let metallic = metallic.clamp(0.0, 1.0);
    let surface = Surface { base: base.clamp(Vec3::ZERO, Vec3::ONE), metallic, roughness: roughness.clamp(0.0, 1.0) };
    // Glass is the dielectric part's transmission, tinted by the base colour.
    let glass = Dielectric { eta: if entering { ior } else { 1.0 / ior }, roughness: surface.roughness, tint: surface.base };
    let share = (transmission * (1.0 - metallic)).clamp(0.0, 1.0);
    Some(SurfaceHit { position, geometric, shading, layered: Layered { surface, glass, share }, emission, albedo: surface.base })
}

/// A point just off a surface, on the side `towards` points.
fn offset(point: Vec3, normal: Vec3, towards: Vec3) -> Vec3 {
    let scale = 1e-4 * point.abs().max_element().max(1.0);
    point + normal * if normal.dot(towards) >= 0.0 { scale } else { -scale }
}

fn clamp_sample(value: Vec3, limit: f32) -> Vec3 {
    let peak = value.max_element();
    if limit > 0.0 && peak > limit { value * (limit / peak) } else { value }
}

/// The nearest light along a ray closer than `far`.
fn light_hit(world: &World, origin: Vec3, direction: Vec3, far: f32) -> Option<(f32, Vec3, f32)> {
    world.lights.iter().filter_map(|light| light.hit(origin, direction)).filter(|(distance, _, _)| *distance < far).min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The world's light along a direction that escapes the scene.
fn world_light(world: &World, direction: Vec3) -> Vec3 {
    world.environment.as_ref().map_or(world.background, |environment| environment.radiance(direction))
}

/// What happened along a stretch of ray through a medium.
enum Passage {
    /// The ray got through, its throughput weighted for that.
    Through,
    /// The ray scattered this far along, its throughput weighted for that.
    Scattered(f32),
    /// The ray got through, its throughput weighted for that, and the light scattered towards it
    /// at a point this far along is to be gathered with this weight.
    Lit(f32, Vec3),
}

/// Equiangular sampling (Kulla and Fajardo, "Importance Sampling Techniques for Path Tracing
/// in Participating Media", 2012): distances along a ray spread evenly in the angle they are seen
/// at from a light, over [0, `distance`].
#[derive(Debug, Clone, Copy)]
struct Equiangular {
    /// Where along the ray the light is nearest, and how far it is from the ray there.
    along: f32,
    gap: f32,
    /// The angles the stretch's ends are seen at from the light, as the first and the span.
    start: f32,
    span: f32,
    distance: f32,
}

impl Equiangular {
    fn new(centre: Vec3, origin: Vec3, direction: Vec3, distance: f32) -> Option<Self> {
        let along = (centre - origin).dot(direction);
        let gap = (centre - (origin + direction * along)).length();
        if !(gap > 1e-6 && along.is_finite()) {
            return None;
        }
        let start = (-along / gap).atan();
        let end = if distance.is_finite() { ((distance - along) / gap).atan() } else { std::f32::consts::FRAC_PI_2 };
        (end > start).then_some(Self { along, gap, start, span: end - start, distance })
    }

    fn sample(&self, u: f32) -> f32 {
        self.along + self.gap * (self.start + u * self.span).tan()
    }

    fn pdf(&self, t: f32) -> f32 {
        if !(0.0..=self.distance).contains(&t) {
            return 0.0;
        }
        let off = t - self.along;
        self.gap / (self.span * (self.gap * self.gap + off * off))
    }
}

/// A distance along a stretch through a medium where light scatters, and its density: half the
/// time following transmittance (on one colour channel picked at random; the channels' mean is
/// the density), half the time equiangular towards a light picked at random, the two mixed by
/// their summed densities. With `within`, transmittance sampling stays within the stretch;
/// otherwise it may pass its end (then the distance is past `distance`).
fn scatter_distance(world: &World, sigma: Vec3, origin: Vec3, direction: Vec3, distance: f32, within: bool, rng: &mut Rng) -> (f32, f32) {
    let targets: Vec<Equiangular> = world.light_points.iter().filter_map(|centre| Equiangular::new(*centre, origin, direction, distance)).collect();
    let share = if targets.is_empty() { 1.0 } else { 0.5 };
    let by_distance = targets.is_empty() || rng.uniform() < share;
    // The share of each channel's transmittance sampling that lands within the stretch.
    let reach = |sigma: f32| if within && distance.is_finite() { -(-sigma * distance).exp_m1() } else { 1.0 };
    let t = if by_distance {
        let pick = ((rng.uniform() * 3.0) as usize).min(2);
        let channel = sigma[pick];
        let u = rng.uniform();
        if channel > 0.0 { -(-u * reach(channel)).ln_1p() / channel } else { f32::INFINITY }
    } else {
        let pick = ((rng.uniform() * targets.len() as f32) as usize).min(targets.len() - 1);
        let u = rng.uniform();
        targets.get(pick).map_or(f32::INFINITY, |target| target.sample(u))
    };
    if !(t.is_finite() && t >= 0.0 && t < distance) {
        return (t, 0.0);
    }
    let by_transmittance = [sigma.x, sigma.y, sigma.z]
        .iter()
        .map(|sigma| if *sigma > 0.0 { sigma * (-sigma * t).exp() / reach(*sigma).max(1e-30) } else { 0.0 })
        .sum::<f32>()
        / 3.0;
    let towards: f32 = if targets.is_empty() { 0.0 } else { targets.iter().map(|target| target.pdf(t)).sum::<f32>() / targets.len() as f32 };
    (t, share * by_transmittance + (1.0 - share) * towards)
}

/// Light along a `distance` metre stretch from `origin` through `medium`: the medium's own light
/// (answered), and how the ray goes on. With `bounce` the ray may scatter there and go on from
/// that point (or get through), its throughput weighted for which; otherwise it gets through
/// weighted by the transmittance, and a point within the stretch is picked to gather the light
/// scattered towards the ray (single scattering, the reference's default of no volume bounces).
#[allow(clippy::too_many_arguments)]
fn pass(
    world: &World,
    medium: &Medium,
    origin: Vec3,
    direction: Vec3,
    distance: f32,
    bounce: bool,
    throughput: &mut Vec3,
    rng: &mut Rng,
) -> (Passage, Vec3) {
    let emitted = *throughput * medium.emitted(distance);
    let sigma = medium.extinction();
    if medium.scattering.max_element() <= 0.0 {
        *throughput *= medium.transmittance(distance);
        return (Passage::Through, emitted);
    }
    if !bounce {
        let (t, pdf) = scatter_distance(world, sigma, origin, direction, distance, true, rng);
        let before = *throughput;
        *throughput *= medium.transmittance(distance);
        if pdf <= 0.0 || !pdf.is_finite() {
            return (Passage::Through, emitted);
        }
        return (Passage::Lit(t, before * medium.scattering * medium.transmittance(t) / pdf), emitted);
    }
    let (t, pdf) = scatter_distance(world, sigma, origin, direction, distance, false, rng);
    if t < distance {
        if pdf <= 0.0 || !pdf.is_finite() {
            *throughput = Vec3::ZERO;
            return (Passage::Through, emitted);
        }
        *throughput *= medium.scattering * medium.transmittance(t) / pdf;
        (Passage::Scattered(t), emitted)
    } else {
        // Only transmittance sampling gets through, half the time when equiangular shares.
        let transmittance = medium.transmittance(distance);
        let share = if world.light_points.is_empty() { 1.0 } else { 0.5 };
        let survive = share * transmittance.element_sum() / 3.0;
        *throughput = if survive > 0.0 { *throughput * transmittance / survive } else { Vec3::ZERO };
        (Passage::Through, emitted)
    }
}

/// The light from every light (and a world image) scattered by `medium` at `point` towards a ray
/// going `direction`, through what lies between, weighted by `weight`; with `mis`, shared with
/// phase sampling.
#[allow(clippy::too_many_arguments)]
fn medium_light(
    world: &World,
    medium: &Medium,
    point: Vec3,
    direction: Vec3,
    weight: Vec3,
    mis: bool,
    time: f32,
    inside: &Inside,
    rng: &mut Rng,
) -> Vec3 {
    let mut total = Vec3::ZERO;
    for light in &world.lights {
        let Some(sample) = light.sample(point, rng.next2()) else { continue };
        let phase = medium.phase(direction, sample.direction);
        let reach = if sample.distance.is_finite() { sample.distance * (1.0 - 1e-4) } else { f32::INFINITY };
        let through = transmittance(world, point, sample.direction, reach, time, inside, rng);
        if through.max_element() <= 0.0 {
            continue;
        }
        let share = if mis && sample.pdf > 0.0 { power_heuristic(sample.pdf, phase) } else { 1.0 };
        total += weight * through * sample.contribution * phase * share;
    }
    if let Some(environment) = &world.environment
        && let Some(sample) = environment.sample(rng.next2())
    {
        let phase = medium.phase(direction, sample.direction);
        let through = transmittance(world, point, sample.direction, f32::INFINITY, time, inside, rng);
        if through.max_element() > 0.0 {
            let share = if mis { power_heuristic(sample.pdf, phase) } else { 1.0 };
            total += weight * through * sample.radiance * phase / sample.pdf * share;
        }
    }
    total
}

/// The radiance arriving along a camera ray at `time`, with the first surface's albedo and
/// normal; with `passes`, the sample's share of each light pass too.
#[allow(clippy::too_many_arguments)]
fn radiance(
    world: &World,
    options: &TraceOptions,
    mut origin: Vec3,
    mut direction: Vec3,
    time: f32,
    rng: &mut Rng,
    mut passes: Option<&mut Sample>,
) -> (Vec3, Vec3, Vec3) {
    // The share of the path's light the first bounce's diffuse lobe carries (per channel).
    let mut diffuse_share = Vec3::ONE;
    let mut total = Vec3::ZERO;
    let mut throughput = Vec3::ONE;
    let mut first = (Vec3::ZERO, Vec3::ZERO);
    let mut last_pdf = 0.0f32;
    let mut inside: Inside = Vec::new();
    // Bounces off surfaces and scatterings in volumes so far.
    let (mut depth, mut scatterings) = (0u32, 0u32);
    let mut crossings = 0;
    for _ in 0..(options.bounces + options.volume_bounces + 3 + MAX_CROSSINGS) {
        let indirect = depth > 0 || scatterings > 0;
        let add = |total: &mut Vec3, value: Vec3| {
            *total += if indirect { clamp_sample(value, options.clamp_indirect) } else { value };
        };
        // Light reaching the camera from a bounce off the first surface: direct when an emitter
        // is found right after the first bounce, indirect otherwise.
        let credit = |passes: &mut Option<&mut Sample>, value: Vec3, emitter: bool| {
            let Some(sample) = passes.as_deref_mut() else { return };
            if depth == 0 || scatterings > 0 {
                return;
            }
            let value = clamp_sample(value, options.clamp_indirect);
            let (diffuse, glossy) = (value * diffuse_share, value * (Vec3::ONE - diffuse_share));
            if emitter && depth == 1 {
                sample.diffuse_direct += diffuse;
                sample.glossy_direct += glossy;
            } else {
                sample.diffuse_indirect += diffuse;
                sample.glossy_indirect += glossy;
            }
        };
        let found = nearest(world, origin, direction, time, rng);
        let mut surface_distance = found.as_ref().map_or(f32::INFINITY, |(hit, start, _)| hit.distance + start.distance(origin));
        let light = if indirect { light_hit(world, origin, direction, surface_distance) } else { None };
        if let Some((distance, _, _)) = light {
            surface_distance = distance;
        }
        // The volume the stretch to the surface (or light) passes through.
        if world.media
            && let Some(medium) = world.medium_of(&inside)
        {
            let bounce = scatterings < options.volume_bounces;
            let (passage, emitted) = pass(world, &medium, origin, direction, surface_distance, bounce, &mut throughput, rng);
            add(&mut total, emitted);
            match passage {
                Passage::Through => {}
                Passage::Lit(t, weight) => {
                    let gathered = medium_light(world, &medium, origin + direction * t, direction, weight, false, time, &inside, rng);
                    add(&mut total, gathered);
                }
                Passage::Scattered(t) => {
                    let point = origin + direction * t;
                    scatterings += 1;
                    let gathered = medium_light(world, &medium, point, direction, throughput, true, time, &inside, rng);
                    add(&mut total, gathered);
                    let (next, pdf) = medium.sample_phase(direction, rng.uniform(), rng.next2());
                    if next == Vec3::ZERO || pdf <= 0.0 {
                        break;
                    }
                    last_pdf = pdf;
                    origin = point;
                    direction = next;
                    if depth + scatterings >= 3 {
                        let survive = throughput.max_element().min(0.95);
                        if survive <= 0.0 || rng.uniform() > survive {
                            break;
                        }
                        throughput /= survive;
                    }
                    continue;
                }
            }
            if throughput.max_element() <= 0.0 {
                break;
            }
        }
        // Lights are not seen by the camera; later bounces find them and share the work with
        // light sampling.
        if let Some((_, light_radiance, light_pdf)) = light {
            let value = throughput * light_radiance * power_heuristic(last_pdf, light_pdf);
            add(&mut total, value);
            credit(&mut passes, value, true);
            break;
        }
        let Some((hit, start, pass_through)) = found else {
            let light = world_light(world, direction);
            // A world image is also sampled directly: share bounces' finds with that.
            let weight = match &world.environment {
                Some(environment) if indirect => power_heuristic(last_pdf, environment.pdf(direction)),
                _ => 1.0,
            };
            add(&mut total, throughput * light * weight);
            credit(&mut passes, throughput * light * weight, true);
            if !indirect {
                first = (light, Vec3::ZERO);
            }
            break;
        };
        if pass_through {
            // A volume's boundary: into or out of its medium, and on along the same ray.
            crossings += 1;
            if crossings > MAX_CROSSINGS {
                break;
            }
            cross(world, &mut inside, hit.triangle, direction, time);
            let geometric = world.shading.get(hit.triangle as usize).map_or(direction, |data| world.normals_at(hit.triangle, data, time).0);
            origin = offset(start + direction * hit.distance, geometric, direction);
            continue;
        }
        let Some(point) = surface_at(world, hit.triangle, hit.u, hit.v, start, direction, hit.distance, time) else { break };
        if !indirect {
            first = (point.albedo, point.shading);
            if let Some(sample) = passes.as_deref_mut() {
                sample.emission += point.emission;
                let cos = point.shading.dot(-direction).clamp(0.0, 1.0);
                sample.diffuse_color += point.layered.diffuse_color(cos);
                sample.glossy_color += point.layered.glossy_color(cos);
            }
        }
        add(&mut total, throughput * point.emission);
        credit(&mut passes, throughput * point.emission, true);
        if depth > options.bounces {
            break;
        }
        let frame = Frame::new(point.shading);
        let outgoing = frame.to_local(-direction);
        if outgoing.z <= 0.0 {
            break;
        }
        let glassy = point.layered.share > 0.0;
        // Light sampling: one sample of every light.
        for light in &world.lights {
            let Some(sample) = light.sample(point.position, rng.next2()) else { continue };
            if sample.direction.dot(point.geometric) <= 0.0 && !glassy {
                continue;
            }
            let incoming = frame.to_local(sample.direction);
            let response = point.layered.eval(outgoing, incoming);
            if response.max_element() <= 0.0 {
                continue;
            }
            let start = offset(point.position, point.geometric, sample.direction);
            let reach = if sample.distance.is_finite() { sample.distance * (1.0 - 1e-4) } else { f32::INFINITY };
            let through = transmittance(world, start, sample.direction, reach, time, &inside, rng);
            if through.max_element() <= 0.0 {
                continue;
            }
            let weight = if sample.pdf > 0.0 { power_heuristic(sample.pdf, point.layered.pdf(outgoing, incoming)) } else { 1.0 };
            let value = throughput * through * response * sample.contribution * weight;
            add(&mut total, value);
            if depth == 0 {
                if let Some(passes) = passes.as_deref_mut() {
                    let diffuse = throughput * through * point.layered.diffuse(outgoing, incoming) * sample.contribution * weight;
                    passes.diffuse_direct += diffuse;
                    passes.glossy_direct += value - diffuse;
                }
            } else {
                credit(&mut passes, value, false);
            }
        }
        // World image sampling.
        if let Some(environment) = &world.environment
            && let Some(sample) = environment.sample(rng.next2())
            && (sample.direction.dot(point.geometric) > 0.0 || glassy)
        {
            let incoming = frame.to_local(sample.direction);
            let response = point.layered.eval(outgoing, incoming);
            if response.max_element() > 0.0 {
                let start = offset(point.position, point.geometric, sample.direction);
                let through = transmittance(world, start, sample.direction, f32::INFINITY, time, &inside, rng);
                if through.max_element() > 0.0 {
                    let weight = power_heuristic(sample.pdf, point.layered.pdf(outgoing, incoming));
                    let value = throughput * through * response * sample.radiance / sample.pdf * weight;
                    add(&mut total, value);
                    if depth == 0 {
                        if let Some(passes) = passes.as_deref_mut() {
                            let diffuse = throughput * through * point.layered.diffuse(outgoing, incoming) * sample.radiance / sample.pdf * weight;
                            passes.diffuse_direct += diffuse;
                            passes.glossy_direct += value - diffuse;
                        }
                    } else {
                        credit(&mut passes, value, false);
                    }
                }
            }
        }
        // The next bounce.
        let Some(next) = point.layered.sample(outgoing, rng.uniform(), rng.next2(), rng.uniform()) else { break };
        let new_direction = frame.to_world(next.direction).normalize_or_zero();
        // A reflection stays on the arriving side, a transmission crosses to the other.
        let crossing = next.direction.z < 0.0;
        if (new_direction.dot(point.geometric) <= 0.0) != crossing {
            break;
        }
        if crossing {
            cross(world, &mut inside, hit.triangle, new_direction, time);
        }
        if depth == 0 {
            let whole = point.layered.eval(outgoing, next.direction);
            let part = point.layered.diffuse(outgoing, next.direction);
            let share = |part: f32, whole: f32| if whole > 1e-12 { (part / whole).clamp(0.0, 1.0) } else { 0.0 };
            diffuse_share = Vec3::new(share(part.x, whole.x), share(part.y, whole.y), share(part.z, whole.z));
        }
        throughput *= next.weight;
        last_pdf = next.pdf;
        depth += 1;
        if depth + scatterings >= 3 {
            let survive = throughput.max_element().min(0.95);
            if survive <= 0.0 || rng.uniform() > survive {
                break;
            }
            throughput /= survive;
        }
        origin = offset(point.position, point.geometric, new_direction);
        direction = new_direction;
    }
    (total, first.0, first.1)
}

/// Renders a scene to the end: RGBA8, its width and height.
pub fn render(scene: &Scene, options: TraceOptions) -> Result<(Vec<u8>, u32, u32), String> {
    let mut tracer = Tracer::new(scene, options)?;
    tracer.finish();
    Ok((tracer.image(), tracer.width, tracer.height))
}

#[cfg(test)]
mod lens_tests {
    use super::*;

    /// A bladed opening is covered evenly: each of its triangles gets its share.
    #[test]
    fn a_bladed_opening_is_covered_evenly() {
        let lens = Lens {
            eye: Vec3::ZERO,
            right: Vec3::X,
            up: Vec3::Y,
            forward: Vec3::NEG_Z,
            radius: 1.0,
            focus: 1.0,
            blades: 6,
            rotation: 0.0,
            ratio: 1.0,
        };
        let mut bins = [0u32; 12];
        let mut rng = Rng::for_sample(0, 5, 7);
        for _ in 0..120_000 {
            let p = lens.point(rng.next2());
            assert!(p.length() <= 1.0 + 1e-5);
            let angle = p.y.atan2(p.x).rem_euclid(std::f32::consts::TAU);
            if let Some(bin) = bins.get_mut((angle / std::f32::consts::TAU * 12.0) as usize % 12) {
                *bin += 1;
            }
        }
        assert!(bins.iter().all(|count| (9_000..11_000).contains(count)), "{bins:?}");
        // The corners sit at π/n + 2πk/n, so the opening reaches its radius at 30° and only
        // cos 30° of it at 0°.
        let corner = lens.point(Vec2::new(0.0, 0.0) + Vec2::new(1e-6, 0.0));
        assert!(corner.length() < 1e-2);
        let edge_point = lens.point(Vec2::new(0.999_999, 1.0));
        assert!((edge_point.y.atan2(edge_point.x).to_degrees() - 30.0).abs() < 0.1, "{edge_point}");
    }
}
