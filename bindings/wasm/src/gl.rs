//! The GPU viewport's data: what the editor's WebGL2 renderer draws, so the engine owns the scene
//! and the editor only draws (docs/design.md, section 4, "GPU viewport").
//!
//! Two extras make the protocol:
//!
//! - `glScene()`: JSON describing one frame — the view (the orbit, and the engine's own view and
//!   projection matrices for the current size), shading, the world and lights, every drawn
//!   object (world matrix, material, a geometry `key`, selection class), the overlays (edit-mode
//!   selection, selected origins, gizmo lines, peers' outlines), and `supported`: whether the GPU
//!   viewport can show this state at all (otherwise the editor shows the software picture).
//! - `glMesh(id, tangents)`: one object's geometry as a binary buffer (layout below), fetched
//!   only when the object's `key` changed. Sculpt mode's clay and weight paint's colours ride in
//!   it per corner, and hidden faces are left out.
//! - `glWorld()`: the world image as linear floats, fetched when the frame's environment key
//!   changed.
//!
//! The frame also carries what the material view lights with: each light's shadow map as the
//! software viewport plans it (the GPU draws the depth pictures itself), and the world image's
//! irradiance. Particles are drawn as the meshes the software viewport makes of them.
//!
//! The software rasterizer stays the reference: the agent's pictures, the headless kit, and the
//! editor's fallback all use it, and picking stays engine-side through the tools.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use glam::{Mat4, Vec3, Vec4};
use kition_scene_model::{Id, LightKind, Object, ObjectData, Scene};
use kition_scene_raster::shadow::ShadowPlan;
use kition_scene_raster::{MeshCache, Overlays, Prepared, Shading};
use serde_json::{Value, json};

use crate::SceneDocument;

/// The geometry buffer's first four bytes.
pub const MESH_MAGIC: &[u8; 4] = b"KGL1";

/// The world image buffer's first four bytes.
pub const WORLD_MAGIC: &[u8; 4] = b"KGW1";

/// Viewport colours, as the software viewport uses them (sRGB hex).
const SELECTED: u32 = 0xf1_58_00;
const ACTIVE: u32 = 0xff_aa_40;
const WIRE: u32 = 0x1a_1a_1a;
const HANDLE: u32 = 0x7a_7a_82;

fn linear(rgb: u32) -> [f32; 3] {
    kition_scene_raster::draw::hex(rgb)
}

fn matrix(value: Mat4) -> Value {
    json!(value.to_cols_array())
}

fn triple(value: Vec3) -> Value {
    json!(value.to_array())
}

/// The most lights the GPU viewport's material view shades with.
const MAX_LIGHTS: usize = 8;

/// Why the GPU viewport cannot show the document's state, if it cannot.
fn unsupported(doc: &SceneDocument, scene: &Scene) -> Option<&'static str> {
    if doc.session.editor.shading == Shading::Rendered {
        return Some("rendered shading runs in the engine worker");
    }
    if doc.session.editor.shading == Shading::Material
        && scene.objects.iter().filter(|object| !object.hidden && matches!(object.data, ObjectData::Light(_))).count() > MAX_LIGHTS
    {
        return Some("more than eight lights");
    }
    None
}

/// What the GPU draws on a mesh in place of its material: sculpt mode's clay (darker where
/// masked, tinted by face set) or weight paint's colours, without hidden faces.
#[derive(Debug, Clone, Default)]
pub struct Paint {
    /// 1 sculpt, 2 weights.
    pub kind: u32,
    /// One value per vertex of the mesh drawn: the mask, or the weight.
    pub values: Vec<f32>,
    /// One tint per face (white without face sets).
    pub tints: Vec<[f32; 3]>,
    /// Faces left out.
    pub hidden: Vec<bool>,
}

impl Paint {
    /// What the software viewport paints on `id`'s mesh `prepared`, if anything.
    fn of(overlays: &Overlays, id: Id, prepared: &Prepared) -> Option<Self> {
        let vertices = prepared.mesh.positions.len();
        if let Some(sculpted) = overlays.sculpt.as_ref().filter(|sculpted| sculpted.object == id) {
            let values = if sculpted.mask.len() == vertices { sculpted.mask.clone() } else { vec![0.0; vertices] };
            let tints = sculpted.face_sets.iter().map(|set| kition_scene_model::sculpt::face_sets::color(*set).unwrap_or([1.0; 3])).collect();
            return Some(Self { kind: 1, values, tints, hidden: sculpted.hidden.clone() });
        }
        let painted = overlays.weights.as_ref().filter(|painted| painted.object == id && painted.weights.len() == vertices)?;
        Some(Self { kind: 2, values: painted.weights.clone(), tints: Vec::new(), hidden: Vec::new() })
    }

    fn hash(&self, hasher: &mut impl Hasher) {
        self.kind.hash(hasher);
        for value in &self.values {
            value.to_bits().hash(hasher);
        }
        for tint in &self.tints {
            tint.map(f32::to_bits).hash(hasher);
        }
        self.hidden.hash(hasher);
    }

    fn hides(&self, face: u32) -> bool {
        self.hidden.get(face as usize).copied().unwrap_or(false)
    }
}

/// The geometry key of an object as the GPU viewport draws it: the prepared mesh (its cache key
/// and identity), the texture coordinates (which the evaluation key leaves out), and the paint.
fn mesh_key(cache: &MeshCache, object: &Object, prepared: &Arc<Prepared>, tangents: bool, paint: Option<&Paint>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cache.key(object.id).hash(&mut hasher);
    (Arc::as_ptr(prepared) as usize).hash(&mut hasher);
    prepared.mesh.positions.len().hash(&mut hasher);
    tangents.hash(&mut hasher);
    if let Some(mesh) = object.mesh() {
        for corner in mesh.uvs.iter().flatten() {
            corner.to_array().map(f32::to_bits).hash(&mut hasher);
        }
    }
    if let Some(paint) = paint {
        paint.hash(&mut hasher);
    }
    hasher.finish()
}

/// A projection in WebGL's depth convention (−1 at the near plane) from one in the engine's (0).
fn gl_projection(projection: Mat4) -> Mat4 {
    Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::new(0.0, 0.0, 2.0, 0.0), Vec4::new(0.0, 0.0, -1.0, 1.0)) * projection
}

/// Each light's shadow map as JSON (null for a light without one): its depth pictures (the
/// WebGL view-projection to draw each, the row giving eye depth, the size, and a texel's size),
/// where a cube map is centred, the softness, and the filter taps.
fn shadows_json(plans: &[Option<ShadowPlan>]) -> Vec<Value> {
    plans
        .iter()
        .map(|plan| {
            let Some(plan) = plan else { return Value::Null };
            let faces: Vec<Value> = plan
                .faces
                .iter()
                .map(|face| {
                    json!({
                        "viewProj": matrix(gl_projection(face.projection) * face.view),
                        "depthRow": (-face.view.row(2)).to_array(),
                        "size": face.size,
                        "orthographic": face.orthographic,
                        "texel": face.texel,
                    })
                })
                .collect();
            json!({
                "faces": faces,
                "centre": plan.centre.map(|centre| centre.to_array()),
                "softness": plan.softness,
                "taps": plan.taps,
            })
        })
        .collect()
}

fn material_json(scene: &Scene, object: &Object) -> Value {
    let Some(material) = object.material.and_then(|id| scene.material(id).ok()) else { return Value::Null };
    let alpha = match material.alpha_mode {
        kition_scene_model::AlphaMode::Opaque => "opaque",
        kition_scene_model::AlphaMode::Clip => "clip",
        kition_scene_model::AlphaMode::Blend => "blend",
    };
    json!({
        "baseColor": material.base_color,
        "metallic": material.metallic,
        "roughness": material.roughness,
        "emission": triple(material.emission * material.emission_strength),
        "alphaMode": alpha,
        "alphaCutoff": material.alpha_cutoff,
        "transmission": material.transmission,
        "ior": material.ior,
        "baseColorTexture": material.base_color_texture,
        "metallicRoughnessTexture": material.metallic_roughness_texture,
        "normalTexture": material.normal_texture,
        "normalStrength": material.normal_strength,
    })
}

/// Lines in world space, with a colour per end.
#[derive(Debug, Default)]
struct Lines {
    positions: Vec<f32>,
    colors: Vec<f32>,
}

impl Lines {
    fn push(&mut self, a: Vec3, b: Vec3, color: [f32; 3]) {
        self.positions.extend(a.to_array());
        self.positions.extend(b.to_array());
        self.colors.extend(color);
        self.colors.extend(color);
    }

    fn json(&self) -> Value {
        json!({ "positions": self.positions, "colors": self.colors })
    }
}

/// Flat triangles in world space, depth-tested (custom bone shapes' faces), with a colour each.
#[derive(Debug, Default)]
struct Fills {
    positions: Vec<f32>,
    colors: Vec<f32>,
}

impl Fills {
    fn push(&mut self, corners: [Vec3; 3], color: [f32; 3]) {
        for corner in corners {
            self.positions.extend(corner.to_array());
            self.colors.extend(color);
        }
    }

    fn json(&self) -> Value {
        json!({ "positions": self.positions, "colors": self.colors })
    }
}

/// Square dots in world space, drawn over everything, with a colour and a size in pixels.
#[derive(Debug, Default)]
struct Dots {
    positions: Vec<f32>,
    colors: Vec<f32>,
    sizes: Vec<f32>,
}

impl Dots {
    fn push(&mut self, at: Vec3, color: [f32; 3], size: f32) {
        self.positions.extend(at.to_array());
        self.colors.extend(color);
        self.sizes.push(size);
    }

    fn json(&self) -> Value {
        json!({ "positions": self.positions, "colors": self.colors, "sizes": self.sizes })
    }
}

/// What the software viewport draws as lines and dots: objects without a surface (camera
/// pyramids, light rings, empties' axes, lattices, bones), force fields and constraints (depth
/// tested), and over everything the armature being edited or posed, curve and lattice points
/// with their handles, and the text caret.
fn line_overlays(doc: &SceneDocument, scene: &Scene, overlays: &Overlays, gizmos: &mut Lines, front: &mut Lines, dots: &mut Dots, fills: &mut Fills) {
    if overlays.hide_extras {
        return;
    }
    let editor = &doc.session.editor;
    let class_color = |id: Id| {
        if editor.active == Some(id) {
            linear(ACTIVE)
        } else if editor.selected.contains(&id) {
            linear(SELECTED)
        } else {
            linear(WIRE)
        }
    };
    for object in scene.objects.iter().filter(|object| !object.hidden && !object.has_geometry()) {
        let Ok(world) = scene.world_matrix(object.id) else { continue };
        // The camera looked through is not drawn.
        if editor.view.camera == Some(object.id) {
            continue;
        }
        let color = class_color(object.id);
        let mut line = |a: Vec3, b: Vec3, to_world: Mat4| gizmos.push(to_world.transform_point3(a), to_world.transform_point3(b), color);
        match &object.data {
            ObjectData::Camera(camera) => {
                let aspect = scene.render.width.max(1) as f32 / scene.render.height.max(1) as f32;
                let (w, h) = camera.frame_half_size(aspect);
                let corners = [Vec3::new(-w, -h, -1.0), Vec3::new(w, -h, -1.0), Vec3::new(w, h, -1.0), Vec3::new(-w, h, -1.0)];
                for (index, corner) in corners.iter().enumerate() {
                    let next = corners.get((index + 1) % 4).copied().unwrap_or(*corner);
                    line(*corner, next, world);
                    let apex = if camera.orthographic { Vec3::new(corner.x, corner.y, 0.0) } else { Vec3::ZERO };
                    line(apex, *corner, world);
                }
                line(Vec3::new(-w * 0.6, h * 1.1, -1.0), Vec3::new(0.0, h * 1.6, -1.0), world);
                line(Vec3::new(0.0, h * 1.6, -1.0), Vec3::new(w * 0.6, h * 1.1, -1.0), world);
                line(Vec3::new(w * 0.6, h * 1.1, -1.0), Vec3::new(-w * 0.6, h * 1.1, -1.0), world);
            }
            ObjectData::Light(light) => {
                let segments = 16;
                let radius = 0.2;
                for (u, v) in [(Vec3::X, Vec3::Y), (Vec3::X, Vec3::Z), (Vec3::Y, Vec3::Z)] {
                    for step in 0..segments {
                        let a = std::f32::consts::TAU * step as f32 / segments as f32;
                        let b = std::f32::consts::TAU * (step + 1) as f32 / segments as f32;
                        line((u * a.cos() + v * a.sin()) * radius, (u * b.cos() + v * b.sin()) * radius, world);
                    }
                }
                if light.kind != LightKind::Point {
                    line(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.5), world);
                }
            }
            ObjectData::Empty { size } => {
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    line(-axis * *size, axis * *size, world);
                }
            }
            ObjectData::Lattice(lattice) => {
                for (a, b) in lattice.edges() {
                    if let (Some(a), Some(b)) = (lattice.points.get(a), lattice.points.get(b)) {
                        line(*a, *b, world);
                    }
                }
            }
            ObjectData::Armature(armature) => {
                // The armature being edited or posed is drawn over everything, its selected bones
                // in orange, with dots at heads and tails; bones at rest while edited; custom
                // shapes' faces depth-tested, except in wireframe.
                let edited = overlays.bones.as_ref().filter(|bones| bones.object == object.id);
                let posed = kition_scene_model::constraints::bone_matrices(scene, object.id, 0);
                let marks = kition_scene_raster::armature_marks(scene, armature, &posed, world, color, edited);
                let target = if marks.front { &mut *front } else { &mut *gizmos };
                for ([a, b], shade) in &marks.lines {
                    target.push(*a, *b, *shade);
                }
                for (point, shade) in &marks.dots {
                    dots.push(*point, *shade, 4.0);
                }
                if editor.shading != Shading::Wireframe {
                    for (corners, shade) in &marks.fills {
                        fills.push(*corners, *shade);
                    }
                }
            }
            ObjectData::Mesh(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_) => {}
        }
    }
    // Force fields and rigid body constraints, on any object.
    for object in scene.objects.iter().filter(|object| !object.hidden && object.physics.is_some()) {
        let color = class_color(object.id);
        for [a, b] in scene.physics_lines(object.id) {
            gizmos.push(a, b, color);
        }
    }
    // Curve and lattice edit mode's points: handles as lines to small dots, points as larger
    // dots, selected ones orange.
    if let Some(curve) = &overlays.curve
        && let Ok(world) = scene.world_matrix(curve.object)
    {
        let (selected, wire, handle) = (linear(SELECTED), linear(WIRE), linear(HANDLE));
        for point in &curve.points {
            let co = world.transform_point3(point.co);
            if let Some((left, right)) = point.handles {
                for (end, chosen) in [(left, point.selected[1]), (right, point.selected[2])] {
                    let end = world.transform_point3(end);
                    let shade = if chosen { selected } else { handle };
                    front.push(co, end, shade);
                    dots.push(end, shade, 4.0);
                }
            }
            dots.push(co, if point.selected[0] { selected } else { wire }, 6.0);
        }
    }
    if let Some(caret) = &overlays.caret
        && let Ok(world) = scene.world_matrix(caret.object)
    {
        let bottom = world.transform_point3(caret.at - Vec3::new(0.0, caret.height * 0.25, 0.0));
        let top = world.transform_point3(caret.at + Vec3::new(0.0, caret.height * 0.85, 0.0));
        front.push(bottom, top, linear(ACTIVE));
    }
}

impl SceneDocument {
    /// The scene the viewport shows: the document's, with particles made into meshes for the view.
    fn shown_scene(&self) -> Option<Scene> {
        crate::commands_particles::viewport_scene(self)
    }

    /// One frame of the GPU viewport, as JSON (see the module's notes).
    pub fn gl_scene(&self) -> Value {
        let shown = self.shown_scene();
        let scene = shown.as_ref().unwrap_or(&self.scene);
        let editor = &self.session.editor;
        let scheme = self.session.scheme.colors();
        let (width, height) = (self.session.width, self.session.height);
        let view = &editor.view;
        let projector = view.projector(scene, width, height);
        let overlays = self.overlays();
        let material = editor.shading == Shading::Material;
        let shading = match editor.shading {
            Shading::Wireframe => "wireframe",
            Shading::Solid => "solid",
            Shading::Material => "material",
            Shading::Rendered => "rendered",
        };
        let mut cache = self.meshes.borrow_mut();
        let drawn = kition_scene_raster::viewport_meshes(scene, &overlays, &mut cache);
        let mut objects = Vec::new();
        for (id, prepared, world) in &drawn {
            let Ok(object) = scene.object(*id) else { continue };
            let paint = Paint::of(&overlays, *id, prepared);
            let tangents = material
                && paint.is_none()
                && object.material.and_then(|id| scene.material(id).ok()).is_some_and(|material| material.normal_texture.is_some());
            let class = if editor.active == Some(*id) {
                2
            } else if overlays.selected.contains(id) {
                1
            } else {
                0
            };
            let peer = overlays.peers.iter().position(|peer| peer.objects.contains(id) && !overlays.selected.contains(id));
            objects.push(json!({
                "id": id,
                "key": format!("{:016x}", mesh_key(&cache, object, prepared, tangents, paint.as_ref())),
                "tangents": tangents,
                "world": matrix(*world),
                "material": material_json(scene, object),
                "paint": paint.as_ref().map(|paint| paint.kind),
                "class": class,
                "peer": peer,
            }));
        }
        let lights: Vec<Value> = scene
            .objects
            .iter()
            .filter(|object| !object.hidden)
            .filter_map(|object| {
                let ObjectData::Light(light) = &object.data else { return None };
                let world = scene.world_matrix(object.id).ok()?;
                let kind = match light.kind {
                    LightKind::Point => "point",
                    LightKind::Sun => "sun",
                    LightKind::Spot => "spot",
                    LightKind::Area => "area",
                };
                Some(json!({
                    "kind": kind,
                    "position": triple(world.transform_point3(Vec3::ZERO)),
                    "direction": triple(world.transform_vector3(Vec3::NEG_Z).normalize_or_zero()),
                    "power": triple(light.color * light.power),
                    "spotSize": light.spot_size,
                }))
            })
            .collect();
        // The material view's shadows and world image, as the software viewport makes them.
        let (shadows, shadow_key) = if material {
            let plans = kition_scene_raster::viewport_shadow_plans(scene, &drawn);
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for (id, prepared, world) in &drawn {
                id.hash(&mut hasher);
                cache.key(*id).hash(&mut hasher);
                (Arc::as_ptr(prepared) as usize).hash(&mut hasher);
                world.to_cols_array().map(f32::to_bits).hash(&mut hasher);
            }
            let shadows = shadows_json(&plans);
            shadows.iter().map(Value::to_string).collect::<Vec<_>>().hash(&mut hasher);
            (shadows, Some(format!("{:016x}", hasher.finish())))
        } else {
            (Vec::new(), None)
        };
        let environment = if material { cache.environment(scene) } else { None };
        let environment = environment.map(|environment| {
            let (w, h) = environment.size();
            json!({
                "key": format!("{}:{}:{w}x{h}", scene.world.image.as_deref().unwrap_or(""), scene.world.strength.to_bits()),
                "coefficients": environment.coefficients().iter().flat_map(|c| c.to_array()).collect::<Vec<f32>>(),
                "turn": environment.turn().to_cols_array(),
            })
        });
        drop(cache);
        let (mut gizmos, mut front, mut dots, mut fills) = (Lines::default(), Lines::default(), Dots::default(), Fills::default());
        line_overlays(self, scene, &overlays, &mut gizmos, &mut front, &mut dots, &mut fills);
        let origins: Vec<Value> = editor
            .selected
            .iter()
            .filter_map(|id| {
                let world = scene.world_matrix(*id).ok()?;
                Some(json!({ "at": triple(world.transform_point3(Vec3::ZERO)), "active": editor.active == Some(*id) }))
            })
            .collect();
        let edit = overlays.edit.as_ref().map(|edit| {
            json!({
                "object": edit.object,
                "vertices": edit.vertices.iter().copied().collect::<Vec<u32>>(),
                "faces": edit.faces.iter().copied().collect::<Vec<u32>>(),
            })
        });
        // Looking through a camera: the engine's view and projection, and the render frame.
        let camera = projector.frame.map(|frame| {
            let clip = view.camera.and_then(|id| scene.object(id).ok()).and_then(|object| match &object.data {
                ObjectData::Camera(camera) => Some(camera.clip_start.max(1e-4)),
                _ => None,
            });
            json!({
                "viewMatrix": matrix(projector.view),
                "projection": matrix(gl_projection(projector.projection)),
                "eye": triple(projector.eye()),
                "perspective": projector.perspective,
                "near": clip.unwrap_or(1e-4),
                "frame": frame,
            })
        });
        let peers: Vec<Value> = overlays.peers.iter().map(|peer| json!({ "color": peer.color })).collect();
        let reason = unsupported(self, scene);
        json!({
            "supported": reason.is_none(),
            "reason": reason,
            "revision": self.revision,
            "width": width,
            "height": height,
            "view": {
                "target": triple(view.target),
                "distance": view.distance,
                "tilt": view.tilt,
                "turn": view.turn,
                "perspective": view.perspective,
                "lens": view.lens,
                "camera": view.camera,
            },
            "viewMatrix": matrix(projector.view),
            "projection": matrix(projector.projection),
            "eye": triple(projector.eye()),
            "orthographic": !projector.perspective,
            "camera": camera,
            "shading": shading,
            // X-ray: the surfaces see-through, edit mode's marks over them.
            "xray": editor.xray && editor.shading != Shading::Wireframe,
            "grid": editor.grid && editor.overlays,
            "world": { "color": triple(scene.world.color), "strength": scene.world.strength },
            "environment": environment,
            "lights": lights,
            "shadows": shadows,
            "shadowKey": shadow_key,
            "objects": objects,
            "fills": fills.json(),
            "gizmos": gizmos.json(),
            "front": front.json(),
            "dots": dots.json(),
            "origins": origins,
            "edit": edit,
            "peers": peers,
            // The editor's own colours follow the host's scheme (`view.scheme`), as the software
            // viewport's do.
            "background": { "top": scheme.top, "bottom": scheme.bottom },
            "colors": { "selected": linear(SELECTED), "active": linear(ACTIVE), "wire": linear(WIRE), "outside": scheme.outside, "grid": scheme.grid },
        })
    }

    /// One object's geometry for the GPU viewport (the module's layout), or an error when the
    /// object draws nothing.
    pub fn gl_mesh(&self, id: Id, tangents: bool) -> Result<Vec<u8>, String> {
        // Particles are objects of the scene shown, not of the document.
        let shown = if self.scene.object(id).is_ok() { None } else { self.shown_scene() };
        let scene = shown.as_ref().unwrap_or(&self.scene);
        let object = scene.object(id).map_err(|error| error.to_string())?;
        let overlays = self.overlays();
        let prepared = kition_scene_raster::viewport_mesh(scene, object, &overlays, &mut self.meshes.borrow_mut())
            .ok_or_else(|| format!("not-found: {} draws no geometry", object.name))?;
        let paint = Paint::of(&overlays, id, &prepared);
        Ok(encode_mesh(&prepared, tangents && paint.is_none(), paint.as_ref()))
    }

    /// The world image for the GPU viewport: magic `KGW1`, width, and height as little-endian
    /// u32s, then linear RGBA as f32s per texel, top row first (empty without a world image).
    pub fn gl_world(&self) -> Vec<u8> {
        let Some(environment) = self.meshes.borrow_mut().environment(&self.scene) else { return Vec::new() };
        let (width, height) = environment.size();
        let mut out = Vec::with_capacity(12 + width * height * 16);
        out.extend(WORLD_MAGIC);
        out.extend(u32::try_from(width).unwrap_or(0).to_le_bytes());
        out.extend(u32::try_from(height).unwrap_or(0).to_le_bytes());
        for texel in environment.texels() {
            for value in [texel.x, texel.y, texel.z, 1.0] {
                out.extend(value.to_le_bytes());
            }
        }
        out
    }
}

/// The geometry buffer: a little-endian header of u32s — magic `KGL1`, corner count, vertex
/// count, edge index count, flags (bit 0: the mesh is closed, its back faces can be culled;
/// bit 1: tangents follow; bit 2: paint follows) — then, per triangle corner, f32 positions (3),
/// normals (3), texture coordinates (2), and the face index (1), then tangents (4 per corner)
/// when flagged, then paint (the vertex's value and its face's tint, 4 per corner) when
/// flagged, then the mesh's own vertex positions (3 per vertex), then its edges as u32 vertex
/// index pairs. Faces the paint hides have no triangles.
pub fn encode_mesh(prepared: &Prepared, tangents: bool, paint: Option<&Paint>) -> Vec<u8> {
    let mesh = &prepared.mesh;
    let kept: Vec<usize> = (0..prepared.triangles.len())
        .filter(|slot| paint.is_none_or(|paint| !prepared.triangles.get(*slot).is_some_and(|(_, face)| paint.hides(*face))))
        .collect();
    let corners = kept.len() * 3;
    let edges = mesh.edges();
    let per_corner = 9 + if tangents { 4 } else { 0 } + if paint.is_some() { 4 } else { 0 };
    let mut out = Vec::with_capacity(24 + corners * 4 * per_corner + mesh.positions.len() * 12 + edges.len() * 8);
    let count = |value: usize| u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes();
    out.extend(MESH_MAGIC);
    out.extend(count(corners));
    out.extend(count(mesh.positions.len()));
    out.extend(count(edges.len() * 2));
    let flags = u32::from(prepared.closed) | (u32::from(tangents) << 1) | (u32::from(paint.is_some()) << 2);
    out.extend(flags.to_le_bytes());
    let floats = |values: &[f32], out: &mut Vec<u8>| {
        for value in values {
            out.extend(value.to_le_bytes());
        }
    };
    let smooth = mesh.smooth && prepared.vertex_normals.len() == mesh.positions.len();
    for slot in &kept {
        let Some((triangle, face)) = prepared.triangles.get(*slot) else { continue };
        let flat = prepared.face_normals.get(*face as usize).copied().unwrap_or(Vec3::Z);
        let uvs = prepared.triangle_uvs.get(*slot);
        let split = prepared.split_normals.get(*slot);
        for (corner, index) in triangle.iter().enumerate() {
            let position = mesh.positions.get(*index as usize).copied().unwrap_or(Vec3::ZERO);
            let normal = match split {
                Some(normals) => normals.get(corner).copied().unwrap_or(flat),
                None if smooth => prepared.vertex_normals.get(*index as usize).copied().unwrap_or(flat),
                None => flat,
            };
            let uv = uvs.and_then(|uvs| uvs.get(corner)).copied().unwrap_or_default();
            floats(&position.to_array(), &mut out);
            floats(&normal.to_array(), &mut out);
            floats(&uv.to_array(), &mut out);
            floats(&[*face as f32], &mut out);
        }
    }
    if tangents {
        let all = prepared.triangle_tangents();
        for slot in &kept {
            for corner in 0..3 {
                let tangent = all.get(*slot).and_then(|corners| corners.get(corner)).copied().unwrap_or(glam::Vec4::new(1.0, 0.0, 0.0, 1.0));
                floats(&tangent.to_array(), &mut out);
            }
        }
    }
    if let Some(paint) = paint {
        for slot in &kept {
            let Some((triangle, face)) = prepared.triangles.get(*slot) else { continue };
            let tint = paint.tints.get(*face as usize).copied().unwrap_or([1.0; 3]);
            for index in triangle {
                let value = paint.values.get(*index as usize).copied().unwrap_or(0.0);
                floats(&[value, tint[0], tint[1], tint[2]], &mut out);
            }
        }
    }
    for point in &mesh.positions {
        floats(&point.to_array(), &mut out);
    }
    for [a, b] in edges {
        out.extend(a.to_le_bytes());
        out.extend(b.to_le_bytes());
    }
    out
}

/// Reads a geometry buffer's header: corners, vertices, edge indices, and flags.
pub fn mesh_header(bytes: &[u8]) -> Option<(u32, u32, u32, u32)> {
    if bytes.get(0..4)? != MESH_MAGIC {
        return None;
    }
    let word = |at: usize| -> Option<u32> { Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)) };
    Some((word(4)?, word(8)?, word(12)?, word(16)?))
}

/// The objects the GPU viewport would ask geometry for, by id (for tests).
pub fn drawn_ids(frame: &Value) -> BTreeSet<u64> {
    frame["objects"].as_array().map(|objects| objects.iter().filter_map(|object| object["id"].as_u64()).collect()).unwrap_or_default()
}
