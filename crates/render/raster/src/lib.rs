//! The Scene kit's renderer: a software rasterizer that draws the viewport (solid, wireframe,
//! or material shading with the overlays a modeller works with) and final renders through the
//! scene camera, and answers what is under a pixel. It runs anywhere the engine runs: in the
//! page's worker as WebAssembly and in the headless kit.
#![forbid(unsafe_code)]

pub mod ao;
pub mod cache;
pub mod draw;
pub mod environment;
pub mod label;
pub mod peers;
pub mod shadow;
pub mod texture;
pub mod view;

use std::collections::BTreeSet;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use kition_scene_model::{Id, LightKind, Mesh, ObjectData, Scene};
use serde::{Deserialize, Serialize};

pub use cache::{MeshCache, Prepared};
pub use draw::{Canvas, NONE};
pub use view::{Axis, Projector, ViewState};

use draw::{Corner, Fragment, hex};

/// How surfaces are drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shading {
    /// Edges only, seen through.
    Wireframe,
    /// Grey surfaces lit from the viewer's side, so shape reads from any angle.
    #[default]
    Solid,
    /// Materials lit by the scene's lights.
    Material,
    /// Progressive path tracing, composed with the same geometry and overlays.
    Rendered,
}

/// What edit mode shows on the object being edited.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOverlay {
    pub object: Id,
    pub vertices: BTreeSet<u32>,
    pub faces: BTreeSet<u32>,
}

/// The editor's colour scheme, which the viewport's own colours follow (the host's light or dark
/// scheme): its background, grid floor and the shade outside a camera's frame. They are the
/// editor's, not the scene's, so nothing of the scheme is saved with the file, and object colours,
/// materials and the world are drawn the same in both. Renders, exports and headless pictures draw
/// in the light scheme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scheme {
    #[default]
    Light,
    Dark,
}

impl Scheme {
    /// `light` or `dark`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// The viewport's own colours in this scheme, linear RGB.
    pub fn colors(self) -> ViewportColors {
        let [top, bottom, grid, outside] = match self {
            Self::Light => [palette::BACKGROUND_TOP, palette::BACKGROUND_BOTTOM, palette::GRID, palette::OUTSIDE_FRAME],
            Self::Dark => [palette::DARK_BACKGROUND_TOP, palette::DARK_BACKGROUND_BOTTOM, palette::DARK_GRID, palette::DARK_OUTSIDE_FRAME],
        };
        ViewportColors { top: hex(top), bottom: hex(bottom), grid: hex(grid), outside: hex(outside) }
    }
}

/// The viewport's own colours (linear RGB): the background from its top to its bottom, the grid
/// floor's lines, and the shade blended over what lies outside a camera's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportColors {
    pub top: [f32; 3],
    pub bottom: [f32; 3],
    pub grid: [f32; 3],
    pub outside: [f32; 3],
}

/// The overlays drawn over the viewport.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlays {
    /// The editor's colour scheme: the background, grid and frame shade follow it.
    pub scheme: Scheme,
    pub grid: bool,
    pub selected: BTreeSet<Id>,
    pub active: Option<Id>,
    pub edit: Option<EditOverlay>,
    /// The armature being edited or posed, and its selected bones.
    pub bones: Option<BoneOverlay>,
    /// Weight paint: the mesh object painted, and one group's weight per vertex.
    pub weights: Option<WeightOverlay>,
    /// Other people's selections (co-editing), outlined in their colours.
    pub peers: Vec<peers::PeerOutline>,
    /// A curve in edit mode: its control points and handles.
    pub curve: Option<CurveOverlay>,
    /// A text in edit mode: where typing goes.
    pub caret: Option<CaretOverlay>,
    /// Sculpt mode: the sculpted mesh as it is, darkened where masked.
    pub sculpt: Option<SculptOverlay>,
    /// The overlays are off: lights, cameras, empties, force fields and origins are not drawn.
    pub hide_extras: bool,
}

/// What sculpt mode shows: the object's own mesh (no modifiers, so dabs land where they are
/// drawn) or the level sculpted, grey, darker where the mask protects it, tinted by face set,
/// and without its hidden faces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SculptOverlay {
    pub object: Id,
    /// The mask per vertex of the mesh drawn; empty when nothing is masked.
    pub mask: Vec<f32>,
    /// The face set of each face drawn (0 for none), coloured; empty when there are none.
    pub face_sets: Vec<u32>,
    /// Whether each face drawn is hidden (not drawn); empty when none is.
    pub hidden: Vec<bool>,
    /// The mesh to draw instead of the object's own (a multiresolution level).
    pub mesh: Option<std::sync::Arc<kition_scene_model::Mesh>>,
}

/// What curve edit mode shows: each control point (and a Bézier point's handles) in the curve
/// object's own space, with what is selected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CurveOverlay {
    pub object: Id,
    pub points: Vec<CurvePointOverlay>,
}

/// One control point as curve edit mode draws it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CurvePointOverlay {
    pub co: Vec3,
    /// A Bézier point's left and right handles.
    pub handles: Option<(Vec3, Vec3)>,
    /// The point, its left handle, its right handle.
    pub selected: [bool; 3],
}

/// The text edit caret: a vertical bar in the text object's own space.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CaretOverlay {
    pub object: Id,
    pub at: Vec3,
    pub height: f32,
}

/// What weight paint mode shows: an object's surface coloured by one vertex group's weights.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeightOverlay {
    pub object: Id,
    /// One weight per vertex of the object's own mesh.
    pub weights: Vec<f32>,
}

/// What armature edit and pose mode show on their armature.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoneOverlay {
    pub object: Id,
    pub selected: BTreeSet<String>,
    pub active: Option<String>,
    /// Pose mode (bones as posed); otherwise edit mode (bones at rest).
    pub pose: bool,
}

/// How a final render is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    /// Lights cast shadows.
    pub shadows: bool,
    /// Creases and covered places get less of the world's light.
    pub ambient_occlusion: bool,
    /// Samples per pixel side, 1 to 4.
    pub samples: u32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { shadows: true, ambient_occlusion: true, samples: 2 }
    }
}

/// Shadow map sizes: final renders, and the material viewport.
const RENDER_SHADOW_SIZE: u32 = 2048;
const VIEWPORT_SHADOW_SIZE: u32 = 1024;
/// Shadow filter taps per side in the material viewport.
const VIEWPORT_SHADOW_TAPS: u32 = 2;

/// Viewport colours (sRGB hex).
mod palette {
    pub const BACKGROUND_TOP: u32 = 0xf4_f4_f6;
    pub const BACKGROUND_BOTTOM: u32 = 0xe2_e2_e6;
    pub const GRID: u32 = 0xb4_b4_ba;
    pub const AXIS_X: u32 = 0xff_33_52;
    pub const AXIS_Y: u32 = 0x8b_dc_00;
    pub const SELECTED: u32 = 0xf1_58_00;
    pub const ACTIVE: u32 = 0xff_aa_40;
    pub const WIRE: u32 = 0x1a_1a_1a;
    pub const EDIT_VERTEX: u32 = 0x00_00_00;
    pub const OUTSIDE_FRAME: u32 = 0x9a_9a_a0;
    pub const SHAPE_FILL: u32 = 0xc4_c4_cc;
    /// The dark scheme's background, grid and frame shade: dark greys, so the grey surfaces of
    /// solid shading, the axes and the selection read as they do on the light background.
    pub const DARK_BACKGROUND_TOP: u32 = 0x3d_3d_40;
    pub const DARK_BACKGROUND_BOTTOM: u32 = 0x2b_2b_2e;
    pub const DARK_GRID: u32 = 0x60_60_66;
    pub const DARK_OUTSIDE_FRAME: u32 = 0x16_16_18;
}

/// The colour of a viewport surface.
const SOLID_GREY: f32 = 0.6;

/// Draws the viewport: `view` seen at `width` × `height`.
pub fn render_viewport(scene: &Scene, view: &ViewState, width: u32, height: u32, shading: Shading, overlays: &Overlays) -> Canvas {
    render_viewport_cached(scene, view, width, height, shading, overlays, &mut MeshCache::new())
}

/// [`render_viewport`] with meshes kept in `cache` between pictures.
pub fn render_viewport_cached(
    scene: &Scene,
    view: &ViewState,
    width: u32,
    height: u32,
    shading: Shading,
    overlays: &Overlays,
    cache: &mut MeshCache,
) -> Canvas {
    render_viewport_picture(scene, view, width, height, shading, overlays, cache, None)
}

/// A progressively traced picture in linear RGB, potentially smaller than the viewport.
pub struct ViewportPicture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Vec3>,
}

/// Composes a path-traced picture with the ordinary viewport's picking and overlays.
/// Sculpt and weight-paint surfaces retain their working colors over the traced scene.
#[allow(clippy::too_many_arguments)]
pub fn render_viewport_picture(
    scene: &Scene,
    view: &ViewState,
    width: u32,
    height: u32,
    shading: Shading,
    overlays: &Overlays,
    cache: &mut MeshCache,
    picture: Option<&ViewportPicture>,
) -> Canvas {
    cache.retain(scene);
    let projector = view.projector(scene, width, height);
    let mut canvas = Canvas::new(width, height, [0.0, 0.0, 0.0, 1.0]);
    let colors = overlays.scheme.colors();
    let (top, bottom) = (colors.top, colors.bottom);
    for y in 0..height {
        let t = y as f32 / height.max(1) as f32;
        let color = [top[0] + (bottom[0] - top[0]) * t, top[1] + (bottom[1] - top[1]) * t, top[2] + (bottom[2] - top[2]) * t, 1.0];
        let row = y as usize * width as usize;
        if let Some(pixels) = canvas.color.get_mut(row..row + width as usize) {
            pixels.fill(color);
        }
    }
    let wires = viewport_meshes(scene, overlays, cache);
    let mut lights = Lights::of(scene);
    if shading == Shading::Material {
        lights.cast_shadows(scene, &wires, VIEWPORT_SHADOW_SIZE, VIEWPORT_SHADOW_TAPS, false, cache);
        lights.environment = cache.environment(scene);
        if let Some(environment) = &lights.environment {
            fill_environment(&mut canvas, &projector, environment);
        }
    }
    // Lights and the world image vary across a face (falloff, highlights, reflections, and
    // shadows): the material view lights every pixel then, as the GPU viewport does.
    let lit = shading == Shading::Material && (!lights.lights.is_empty() || lights.environment.is_some());
    let flat_fill = !lights.shadowed() && !lit;
    let studio = Studio::new(&projector);
    let eye = projector.eye();
    let order = if shading == Shading::Material { see_through_last(scene, &wires, eye, &mut cache.textures) } else { (0..wires.len()).collect() };
    for (id, prepared, world) in order.into_iter().filter_map(|index| wires.get(index)) {
        let Ok(object) = scene.object(*id) else { continue };
        let world = *world;
        if shading != Shading::Wireframe {
            let surface = Surface::of(scene, object.material, &mut cache.textures);
            let base = match shading {
                Shading::Material => surface.base.into(),
                _ => [SOLID_GREY; 3],
            };
            let edit = overlays.edit.as_ref().filter(|edit| edit.object == object.id);
            // A textured surface changes across a face: shade it per pixel.
            let textured = shading == Shading::Material && (surface.textured() || surface.see_through());
            let flat = !prepared.mesh.smooth && !textured && !lit;
            let mut memo = FaceTerms::default();
            let bent = shading == Shading::Material && surface.normal_texture.is_some();
            if let Some(sculpted) = overlays.sculpt.as_ref().filter(|sculpted| sculpted.object == object.id) {
                // A clay grey, lit by the studio light; the mask rides in the texture
                // coordinate's first component and darkens the surface.
                let mask = if sculpted.mask.len() == prepared.mesh.positions.len() {
                    sculpted.mask.clone()
                } else {
                    vec![0.0; prepared.mesh.positions.len()]
                };
                let tints: Vec<[f32; 3]> =
                    sculpted.face_sets.iter().map(|set| kition_scene_model::sculpt::face_sets::color(*set).unwrap_or([1.0; 3])).collect();
                draw_mesh_weights(&mut canvas, &projector, prepared, world, object.id, &mask, &sculpted.hidden, &mut |fragment, face| {
                    let tint = tints.get(face as usize).copied().unwrap_or([1.0; 3]);
                    let light = studio.shade(fragment, [SOLID_GREY * tint[0], SOLID_GREY * tint[1], SOLID_GREY * tint[2]]);
                    let keep = 1.0 - 0.65 * fragment.uv.x.clamp(0.0, 1.0);
                    [light[0] * keep, light[1] * keep, light[2] * keep, 1.0]
                });
                continue;
            }
            if let Some(painted) =
                overlays.weights.as_ref().filter(|painted| painted.object == object.id && painted.weights.len() == prepared.mesh.positions.len())
            {
                // Weight colours, lit by the studio light; the weight rides in the texture
                // coordinate's first component.
                draw_mesh_weights(&mut canvas, &projector, prepared, world, object.id, &painted.weights, &[], &mut |fragment, _| {
                    let light = studio.shade(fragment, [1.0; 3])[0];
                    let [r, g, b] = kition_scene_model::weight_paint::ramp(fragment.uv.x).map(|channel| channel * (0.35 + 0.65 * light));
                    [r, g, b, 1.0]
                });
                continue;
            }
            draw_mesh(&mut canvas, &projector, prepared, world, object.id, flat_fill && !textured, bent, &mut |fragment, face| {
                let (lit, coverage) = match shading {
                    Shading::Material => {
                        let point = surface.at(&fragment);
                        let terms = memo.get(flat, face, || lights.terms(scene, &projector, surface.bend(fragment), &point));
                        (lights.resolve(terms, fragment, 1.0), surface.coverage(&point, &fragment, eye))
                    }
                    _ => (studio.shade(fragment, base), 1.0),
                };
                let [r, g, b] = match edit {
                    Some(edit) if edit.faces.contains(&face) => mix(lit, hex(palette::SELECTED), 0.35),
                    _ => lit,
                };
                [r, g, b, coverage]
            });
        }
    }
    if let Some(picture) = picture.filter(|picture| picture.width > 0 && picture.height > 0) {
        for (index, pixel) in canvas.color.iter_mut().enumerate() {
            let object = canvas.object.get(index).copied();
            if overlays.sculpt.as_ref().is_some_and(|overlay| Some(overlay.object) == object)
                || overlays.weights.as_ref().is_some_and(|overlay| Some(overlay.object) == object)
            {
                continue;
            }
            let x = (index % width.max(1) as usize) as u64 * u64::from(picture.width) / u64::from(width.max(1));
            let y = (index / width.max(1) as usize) as u64 * u64::from(picture.height) / u64::from(height.max(1));
            let at = y.saturating_mul(u64::from(picture.width)).saturating_add(x);
            if let Some(color) = usize::try_from(at).ok().and_then(|at| picture.pixels.get(at)) {
                let selected = overlays
                    .edit
                    .as_ref()
                    .is_some_and(|edit| Some(edit.object) == object && canvas.face.get(index).is_some_and(|face| edit.faces.contains(face)));
                // The render's colour management shows in the rendered view, as in the reference.
                let display = &scene.render.color_management;
                let color = if display.is_standard() { color.to_array() } else { display.apply(color.to_array()) };
                let [r, g, b] = if selected { mix(color, hex(palette::SELECTED), 0.35) } else { color };
                *pixel = [r, g, b, 1.0];
            }
        }
    }
    if overlays.grid {
        draw_grid(&mut canvas, &projector, view, colors.grid);
    }
    if shading == Shading::Wireframe {
        for (id, prepared, world) in &wires {
            let color = if overlays.active == Some(*id) {
                hex(palette::ACTIVE)
            } else if overlays.selected.contains(id) {
                hex(palette::SELECTED)
            } else {
                hex(palette::WIRE)
            };
            draw_edges(&mut canvas, &projector, &prepared.mesh, *world, color, 0.8, false);
        }
    } else {
        // Wires without faces (an unbevelled curve, a lone edge) show in the shaded views too.
        for (id, prepared, world) in wires.iter().filter(|(_, prepared, _)| !prepared.mesh.loose_edges.is_empty()) {
            let color = if overlays.active == Some(*id) {
                hex(palette::ACTIVE)
            } else if overlays.selected.contains(id) {
                hex(palette::SELECTED)
            } else {
                [0.0; 3]
            };
            for [a, b] in &prepared.mesh.loose_edges {
                let (Some(pa), Some(pb)) = (prepared.mesh.positions.get(*a as usize), prepared.mesh.positions.get(*b as usize)) else { continue };
                canvas.line(&projector, world.transform_point3(*pa), world.transform_point3(*pb), color, 1.0, 1, true);
            }
        }
        outline(&mut canvas, overlays);
    }
    peers::outline_peers(&mut canvas, &overlays.peers, &overlays.selected);
    for object in scene.objects.iter().filter(|object| !overlays.hide_extras && !object.hidden && !object.has_geometry()) {
        let Ok(world) = scene.world_matrix(object.id) else { continue };
        if view.camera == Some(object.id) {
            continue;
        }
        let color = if overlays.active == Some(object.id) {
            hex(palette::ACTIVE)
        } else if overlays.selected.contains(&object.id) {
            hex(palette::SELECTED)
        } else {
            hex(palette::WIRE)
        };
        let bones = overlays.bones.as_ref().filter(|bones| bones.object == object.id);
        draw_gizmo(&mut canvas, &projector, scene, object.id, &object.data, world, color, bones, shading != Shading::Wireframe);
    }
    // Force fields and rigid body constraints, on any object.
    for object in scene.objects.iter().filter(|object| !overlays.hide_extras && !object.hidden && object.physics.is_some()) {
        let color = if overlays.active == Some(object.id) {
            hex(palette::ACTIVE)
        } else if overlays.selected.contains(&object.id) {
            hex(palette::SELECTED)
        } else {
            hex(palette::WIRE)
        };
        for [a, b] in scene.physics_lines(object.id) {
            canvas.line(&projector, a, b, color, 0.8, 1, true);
        }
    }
    if let Some(edit) = &overlays.edit
        && let (Some((_, prepared, world)), Ok(object)) = (wires.iter().find(|(id, _, _)| *id == edit.object), scene.object(edit.object))
        && !object.hidden
    {
        draw_edit(&mut canvas, &projector, &prepared.mesh, *world, edit);
    }
    if let Some(curve) = &overlays.curve
        && let Ok(world) = scene.world_matrix(curve.object)
    {
        draw_curve_points(&mut canvas, &projector, curve, world);
    }
    if let Some(caret) = &overlays.caret
        && let Ok(world) = scene.world_matrix(caret.object)
    {
        let bottom = world.transform_point3(caret.at - Vec3::new(0.0, caret.height * 0.25, 0.0));
        let top = world.transform_point3(caret.at + Vec3::new(0.0, caret.height * 0.85, 0.0));
        canvas.line(&projector, bottom, top, hex(palette::ACTIVE), 1.0, 2, false);
    }
    for id in &overlays.selected {
        if let Ok(world) = scene.world_matrix(*id) {
            let color = if overlays.active == Some(*id) { hex(palette::ACTIVE) } else { hex(palette::SELECTED) };
            canvas.dot(&projector, world.transform_point3(Vec3::ZERO), color, 4, false);
        }
    }
    if let Some([x0, y0, x1, y1]) = projector.frame {
        dim_outside(&mut canvas, [x0, y0, x1, y1], colors.outside);
    }
    canvas
}

/// The meshes the viewport draws, in scene order: each visible object's mesh as prepared (edit
/// mode and sculpt show the object's own mesh, sculpt a multiresolution level when one is
/// sculpted, and weight paint the mesh its weights belong to), with its world matrix. The GPU
/// viewport draws the same ones.
pub fn viewport_meshes(scene: &Scene, overlays: &Overlays, cache: &mut MeshCache) -> Vec<(Id, Arc<Prepared>, Mat4)> {
    scene
        .objects
        .iter()
        .filter(|object| !object.hidden)
        .filter_map(|object| Some((object.id, viewport_mesh(scene, object, overlays, cache)?, scene.world_matrix(object.id).ok()?)))
        .collect()
}

/// One object's mesh as the viewport draws it (see [`viewport_meshes`]).
pub fn viewport_mesh(scene: &Scene, object: &kition_scene_model::Object, overlays: &Overlays, cache: &mut MeshCache) -> Option<Arc<Prepared>> {
    // Edit mode and sculpt work on the object's own mesh: show that (one cache entry per
    // object, so asking for the evaluated mesh too would rebuild both every frame).
    let raw = overlays.edit.as_ref().is_some_and(|edit| edit.object == object.id)
        || overlays.sculpt.as_ref().is_some_and(|sculpted| sculpted.object == object.id);
    let level = overlays.sculpt.as_ref().filter(|sculpted| sculpted.object == object.id).and_then(|sculpted| sculpted.mesh.clone());
    let mut prepared = match level {
        Some(mesh) => cache.given(object.id, &mesh),
        None => cache.get(scene, object, raw)?,
    };
    // Weights are per vertex of the object's own mesh: painted on the deformed surface while
    // its modifiers keep the vertices (an armature), else on the mesh itself.
    if let Some(painted) = overlays.weights.as_ref().filter(|painted| painted.object == object.id)
        && prepared.mesh.positions.len() != painted.weights.len()
        && let Some(own) = cache.get(scene, object, true)
    {
        prepared = own;
    }
    Some(prepared)
}

/// The shadow maps the material viewport gives its lights (in the scene's order of visible
/// lights), as plans: the GPU viewport draws them itself.
pub fn viewport_shadow_plans(scene: &Scene, casters: &[(Id, Arc<Prepared>, Mat4)]) -> Vec<Option<shadow::ShadowPlan>> {
    let lights = Lights::of(scene);
    if casters.is_empty() {
        return lights.lights.iter().map(|_| None).collect();
    }
    let bounds = casters.iter().fold(kition_scene_model::Bounds::EMPTY, |bounds, (_, prepared, world)| bounds.union(prepared.mesh.bounds(*world)));
    let posed: Vec<(Arc<Prepared>, Mat4)> = casters.iter().map(|(_, prepared, world)| (Arc::clone(prepared), *world)).collect();
    lights
        .lights
        .iter()
        .map(|light| {
            shadow::ShadowPlan::new(
                light.kind,
                light.position,
                light.direction,
                light.spot_size,
                light.radius,
                if light.kind == LightKind::Sun { shadow::sun_bounds(&posed, light.direction) } else { bounds },
                VIEWPORT_SHADOW_SIZE,
                VIEWPORT_SHADOW_TAPS,
            )
        })
        .collect()
}

/// Curve edit mode's points: handles as lines to small dots, points as larger dots, selected
/// ones orange; drawn over the surface.
fn draw_curve_points(canvas: &mut Canvas, projector: &Projector, curve: &CurveOverlay, world: Mat4) {
    let (selected, wire, handle) = (hex(palette::SELECTED), hex(palette::WIRE), hex(0x7a_7a_82));
    for point in &curve.points {
        let co = world.transform_point3(point.co);
        if let Some((left, right)) = point.handles {
            for (end, chosen) in [(left, point.selected[1]), (right, point.selected[2])] {
                let end = world.transform_point3(end);
                canvas.line(projector, co, end, if chosen { selected } else { handle }, 0.9, 1, false);
                canvas.dot(projector, end, if chosen { selected } else { handle }, 4, false);
            }
        }
        canvas.dot(projector, co, if point.selected[0] { selected } else { wire }, 6, false);
    }
}

/// Renders the scene through its camera at the render size, `samples` × `samples` per pixel,
/// with shadows and ambient occlusion.
pub fn render_camera(scene: &Scene, samples: u32) -> Result<Canvas, String> {
    render_camera_with(scene, &RenderOptions { samples, ..RenderOptions::default() })
}

/// Renders the scene through its camera at the render size.
pub fn render_camera_with(scene: &Scene, options: &RenderOptions) -> Result<Canvas, String> {
    render_camera_cached(scene, options, &mut MeshCache::new())
}

/// [`render_camera_with`] with meshes kept in `cache` between pictures.
pub fn render_camera_cached(scene: &Scene, options: &RenderOptions, cache: &mut MeshCache) -> Result<Canvas, String> {
    render_camera_light(scene, options, cache, false)
}

/// [`render_camera_cached`], keeping light brighter than white and the depth of each pixel (the
/// nearest of its samples): the passes compositing reads. A plain render clips each sample to
/// white before it averages a pixel's samples.
pub fn render_camera_passes(scene: &Scene, options: &RenderOptions, cache: &mut MeshCache) -> Result<Canvas, String> {
    render_camera_light(scene, options, cache, true)
}

fn render_camera_light(scene: &Scene, options: &RenderOptions, cache: &mut MeshCache, hdr: bool) -> Result<Canvas, String> {
    let camera = scene.camera.ok_or("the scene has no camera")?;
    let view = view::camera_view(scene, camera).ok_or("the scene camera is not a camera")?;
    let samples = options.samples.clamp(1, 4);
    let (width, height) = (scene.render.width, scene.render.height);
    let (big_width, big_height) = (width.saturating_mul(samples), height.saturating_mul(samples));
    let projector = view.fit(big_width, big_height, width, height);
    let background = scene.world.color * scene.world.strength;
    cache.retain(scene);
    let mut meshes: Vec<(&kition_scene_model::Object, Arc<Prepared>, Mat4)> = Vec::new();
    for object in scene.objects.iter().filter(|object| !object.hidden) {
        let Ok(world) = scene.world_matrix(object.id) else { continue };
        let Some(prepared) = cache.get(scene, object, false) else { continue };
        meshes.push((object, prepared, world));
    }
    let mut lights = Lights::of(scene);
    lights.environment = cache.environment(scene);
    let casters: Vec<(Id, Arc<Prepared>, Mat4)> = meshes.iter().map(|(object, prepared, world)| (object.id, Arc::clone(prepared), *world)).collect();
    if options.shadows {
        lights.cast_shadows(scene, &casters, RENDER_SHADOW_SIZE, 4, true, cache);
    }
    let order = see_through_last(scene, &casters, projector.eye(), &mut cache.textures);
    // Ambient occlusion needs the depth picture first: draw depth, estimate, then shade.
    let settings = scene.render.ambient_occlusion;
    let occlusion = if options.ambient_occlusion && settings.enabled && !meshes.is_empty() {
        let mut depth = Canvas::new(big_width, big_height, [0.0; 4]);
        for (object, prepared, world) in &meshes {
            // Only what hides the world gives occlusion.
            if Surface::of(scene, object.material, &mut cache.textures).see_through() {
                continue;
            }
            draw_mesh(&mut depth, &projector, prepared, *world, object.id, true, false, &mut |_, _| [0.0, 0.0, 0.0, 1.0]);
        }
        let radius = settings.distance_for(scene.bounds(None).size().length());
        let mut occlusion = ao::occlusion(&depth.depth, &projector, samples, width, height, radius);
        for factor in &mut occlusion.factor {
            *factor = settings.weaken(*factor);
        }
        Some(occlusion)
    } else {
        None
    };
    let mut canvas = Canvas::new(big_width, big_height, [background.x, background.y, background.z, 1.0]);
    if let Some(environment) = &lights.environment {
        fill_environment(&mut canvas, &projector, environment);
    }
    let eye = projector.eye();
    for (object, prepared, world) in order.into_iter().filter_map(|index| meshes.get(index)) {
        let surface = Surface::of(scene, object.material, &mut cache.textures);
        // A final render lights every pixel: a point light's falloff and angle change across a
        // flat face too (the viewport works a flat face out once).
        let bent = surface.normal_texture.is_some();
        draw_mesh(&mut canvas, &projector, prepared, *world, object.id, false, bent, &mut |fragment, _face| {
            let ambient = occlusion.as_ref().map_or(1.0, |occlusion| occlusion.at(fragment.x, fragment.y, samples));
            let point = surface.at(&fragment);
            let terms = lights.terms(scene, &projector, surface.bend(fragment), &point);
            let [r, g, b] = lights.resolve(&terms, fragment, ambient);
            [r, g, b, surface.coverage(&point, &fragment, eye)]
        });
    }
    Ok(downsample(&canvas, samples, (!hdr).then_some(&scene.render.color_management)))
}

/// The order to draw objects in: opaque ones first, then see-through ones (blended alpha,
/// glass) from the farthest to the nearest, so each blends over what is behind it.
fn see_through_last(scene: &Scene, meshes: &[(Id, Arc<Prepared>, Mat4)], eye: Vec3, textures: &mut texture::TextureCache) -> Vec<usize> {
    let mut opaque = Vec::new();
    let mut clear: Vec<(f32, usize)> = Vec::new();
    for (index, (id, prepared, world)) in meshes.iter().enumerate() {
        let material = scene.object(*id).ok().and_then(|object| object.material);
        if Surface::of(scene, material, textures).see_through() {
            let centre = prepared.mesh.bounds(*world).center();
            clear.push((centre.distance_squared(eye), index));
        } else {
            opaque.push(index);
        }
    }
    clear.sort_by(|a, b| b.0.total_cmp(&a.0));
    opaque.extend(clear.into_iter().map(|(_, index)| index));
    opaque
}

/// Paints the world image behind everything: each pixel the world's radiance along its ray.
fn fill_environment(canvas: &mut Canvas, projector: &Projector, environment: &environment::Environment) {
    let inverse = projector.view_projection().inverse();
    let (width, height) = (canvas.width.max(1) as f32, canvas.height.max(1) as f32);
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            let ndc_x = (x as f32 + 0.5) / width * 2.0 - 1.0;
            let ndc_y = 1.0 - (y as f32 + 0.5) / height * 2.0;
            let near = inverse * glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
            let far = inverse * glam::Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
            let direction = (far.truncate() / far.w - near.truncate() / near.w).normalize_or_zero();
            let radiance = environment.radiance(direction);
            if let Some(pixel) = canvas.color.get_mut(y as usize * canvas.width as usize + x as usize) {
                *pixel = [radiance.x, radiance.y, radiance.z, 1.0];
            }
        }
    }
}

/// What is under pixel (`x`, `y`) of a drawn viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub object: Id,
    pub face: u32,
}

/// The object and face drawn at a pixel, if any.
pub fn hit(canvas: &Canvas, x: u32, y: u32) -> Option<Hit> {
    if x >= canvas.width || y >= canvas.height {
        return None;
    }
    let slot = y as usize * canvas.width as usize + x as usize;
    let object = canvas.object.get(slot).copied().filter(|id| *id != NONE)?;
    Some(Hit { object, face: canvas.face.get(slot).copied().unwrap_or(NONE) })
}

/// The objects drawn near a pixel, nearest first (lights, cameras, and empties have no surface,
/// so a click near their origin selects them).
pub fn pick_object(scene: &Scene, canvas: &Canvas, projector: &Projector, x: f32, y: f32) -> Option<Id> {
    if let Some(found) = hit(canvas, x.max(0.0) as u32, y.max(0.0) as u32) {
        return Some(found.object);
    }
    let mut best: Option<(f32, Id)> = None;
    for object in scene.objects.iter().filter(|object| !object.hidden) {
        let Ok(world) = scene.world_matrix(object.id) else { continue };
        let clip = projector.view_projection() * world.transform_point3(Vec3::ZERO).extend(1.0);
        if clip.w <= 0.0 || clip.z < 0.0 {
            continue;
        }
        let screen = projector.to_screen(clip);
        let distance = ((screen.x - x).powi(2) + (screen.y - y).powi(2)).sqrt();
        if distance <= 12.0 && best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, object.id));
        }
    }
    best.map(|(_, id)| id)
}

/// The visible vertex of `mesh` (object matrix `world`) nearest pixel (`x`, `y`), within
/// `radius` pixels.
pub fn pick_vertex(canvas: &Canvas, projector: &Projector, mesh: &Mesh, world: Mat4, x: f32, y: f32, radius: f32) -> Option<u32> {
    let matrix = projector.view_projection() * world;
    let mut best: Option<(f32, u32)> = None;
    for (index, point) in mesh.positions.iter().enumerate() {
        let clip = matrix * point.extend(1.0);
        if clip.w <= 0.0 || clip.z < 0.0 {
            continue;
        }
        let screen = projector.to_screen(clip);
        let distance = ((screen.x - x).powi(2) + (screen.y - y).powi(2)).sqrt();
        if distance > radius {
            continue;
        }
        let depth = -(projector.view * world).transform_point3(*point).z;
        if !canvas.visible(screen.x.floor() as i64, screen.y.floor() as i64, depth) {
            continue;
        }
        if best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, u32::try_from(index).unwrap_or(u32::MAX)));
        }
    }
    best.map(|(_, index)| index)
}

/// The vertices of `mesh` whose projections fall inside the pixel rectangle (and are visible).
pub fn vertices_in_rect(canvas: &Canvas, projector: &Projector, mesh: &Mesh, world: Mat4, rect: [f32; 4]) -> Vec<u32> {
    let matrix = projector.view_projection() * world;
    let eye = projector.view * world;
    let [x0, y0, x1, y1] = rect;
    let (left, right, top, bottom) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    let mut out = Vec::new();
    for (index, point) in mesh.positions.iter().enumerate() {
        let clip = matrix * point.extend(1.0);
        if clip.w <= 0.0 || clip.z < 0.0 {
            continue;
        }
        let screen = projector.to_screen(clip);
        if screen.x < left || screen.x > right || screen.y < top || screen.y > bottom {
            continue;
        }
        if canvas.visible(screen.x.floor() as i64, screen.y.floor() as i64, -eye.transform_point3(*point).z) {
            out.push(u32::try_from(index).unwrap_or(u32::MAX));
        }
    }
    out
}

/// Draws a mesh with one value per vertex carried in each corner's texture coordinate (x), for
/// weight paint.
fn draw_mesh_weights(
    canvas: &mut Canvas,
    projector: &Projector,
    prepared: &Prepared,
    world: Mat4,
    id: Id,
    values: &[f32],
    hidden: &[bool],
    shade: &mut dyn FnMut(Fragment, u32) -> [f32; 4],
) {
    let mesh = &prepared.mesh;
    let matrix = projector.view_projection() * world;
    let eye_matrix = projector.view * world;
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    let flat_normals: Vec<Vec3> = prepared.face_normals.iter().map(|normal| (normal_matrix * *normal).normalize_or_zero()).collect();
    let smooth_normals: Vec<Vec3> = prepared.vertex_normals.iter().map(|normal| (normal_matrix * *normal).normalize_or_zero()).collect();
    let eye = projector.eye();
    let view_direction = Mat3::from_mat4(projector.view.inverse()) * Vec3::NEG_Z;
    for (number, (triangle, face)) in prepared.triangles.iter().enumerate() {
        if hidden.get(*face as usize).copied().unwrap_or(false) {
            continue;
        }
        let flat = flat_normals.get(*face as usize).copied().unwrap_or(Vec3::Z);
        let corner = |index: u32| -> Option<Corner> {
            let slot = index as usize;
            let point = *mesh.positions.get(slot)?;
            let normal = if mesh.smooth { smooth_normals.get(slot).copied().unwrap_or(flat) } else { flat };
            Some(Corner {
                clip: matrix * point.extend(1.0),
                depth: -eye_matrix.transform_point3(point).z,
                position: world.transform_point3(point),
                normal,
                uv: Vec2::new(values.get(slot).copied().unwrap_or(0.0), 0.0),
                tangent: Vec4::ZERO,
            })
        };
        let (Some(mut a), Some(mut b), Some(mut c)) = (corner(triangle[0]), corner(triangle[1]), corner(triangle[2])) else { continue };
        if let Some(normals) = prepared.split_normals.get(number) {
            let turn = |normal: Vec3| (normal_matrix * normal).normalize_or_zero();
            (a.normal, b.normal, c.normal) = (turn(normals[0]), turn(normals[1]), turn(normals[2]));
        }
        if prepared.closed {
            let toward = if projector.perspective { a.position - eye } else { view_direction };
            if flat.dot(toward) > 0.0 {
                continue;
            }
        }
        let face = *face;
        canvas.triangle(projector, [a, b, c], id, face, false, &mut |fragment| shade(fragment, face));
    }
}

fn draw_mesh(
    canvas: &mut Canvas,
    projector: &Projector,
    prepared: &Prepared,
    world: Mat4,
    id: Id,
    flat_fill: bool,
    tangents: bool,
    shade: &mut dyn FnMut(Fragment, u32) -> [f32; 4],
) {
    let mesh = &prepared.mesh;
    // World tangents (object tangents turned by the object's rotation and scale; a mirroring
    // transform flips the bitangent's sign).
    let turn = Mat3::from_mat4(world);
    let mirror = if world.determinant() < 0.0 { -1.0 } else { 1.0 };
    let triangle_tangents = if tangents { prepared.triangle_tangents() } else { &[] };
    let matrix = projector.view_projection() * world;
    let eye_matrix = projector.view * world;
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    let flat_normals: Vec<Vec3> = prepared.face_normals.iter().map(|normal| (normal_matrix * *normal).normalize_or_zero()).collect();
    let smooth_normals: Vec<Vec3> = prepared.vertex_normals.iter().map(|normal| (normal_matrix * *normal).normalize_or_zero()).collect();
    // Each vertex goes to the screen once.
    let clips: Vec<glam::Vec4> = mesh.positions.iter().map(|point| matrix * point.extend(1.0)).collect();
    let depths: Vec<f32> = mesh.positions.iter().map(|point| -eye_matrix.transform_point3(*point).z).collect();
    let worlds: Vec<Vec3> = mesh.positions.iter().map(|point| world.transform_point3(*point)).collect();
    let eye = projector.eye();
    let view_direction = Mat3::from_mat4(projector.view.inverse()) * Vec3::NEG_Z;
    for (index, (triangle, face)) in prepared.triangles.iter().enumerate() {
        let face = *face;
        let uvs = prepared.triangle_uvs.get(index).copied().unwrap_or([Vec2::ZERO; 3]);
        let flat = flat_normals.get(face as usize).copied().unwrap_or(Vec3::Z);
        let corner = |index: u32| -> Option<Corner> {
            let slot = index as usize;
            let normal = if mesh.smooth { smooth_normals.get(slot).copied().unwrap_or(flat) } else { flat };
            Some(Corner {
                clip: *clips.get(slot)?,
                depth: *depths.get(slot)?,
                position: *worlds.get(slot)?,
                normal,
                uv: Vec2::ZERO,
                tangent: Vec4::ZERO,
            })
        };
        let (Some(mut a), Some(mut b), Some(mut c)) = (corner(triangle[0]), corner(triangle[1]), corner(triangle[2])) else { continue };
        (a.uv, b.uv, c.uv) = (uvs[0], uvs[1], uvs[2]);
        if let Some(normals) = prepared.split_normals.get(index) {
            let turn = |normal: Vec3| (normal_matrix * normal).normalize_or_zero();
            (a.normal, b.normal, c.normal) = (turn(normals[0]), turn(normals[1]), turn(normals[2]));
        }
        if let Some(object_tangents) = triangle_tangents.get(index) {
            let world_tangent = |tangent: Vec4| (turn * tangent.truncate()).normalize_or_zero().extend(tangent.w * mirror);
            (a.tangent, b.tangent, c.tangent) =
                (world_tangent(object_tangents[0]), world_tangent(object_tangents[1]), world_tangent(object_tangents[2]));
        }
        // A closed mesh's faces turned away from the eye are hidden behind its front: skip them.
        if prepared.closed {
            let toward = if projector.perspective { a.position - eye } else { view_direction };
            if flat.dot(toward) > 0.0 {
                continue;
            }
        }
        canvas.triangle(projector, [a, b, c], id, face, flat_fill && !mesh.smooth, &mut |fragment| shade(fragment, face));
    }
}

fn draw_edges(canvas: &mut Canvas, projector: &Projector, mesh: &Mesh, world: Mat4, color: [f32; 3], alpha: f32, depth_test: bool) {
    for [a, b] in mesh.edges() {
        let (Some(pa), Some(pb)) = (mesh.positions.get(a as usize), mesh.positions.get(b as usize)) else { continue };
        canvas.line(projector, world.transform_point3(*pa), world.transform_point3(*pb), color, alpha, 1, depth_test);
    }
}

fn draw_edit(canvas: &mut Canvas, projector: &Projector, mesh: &Mesh, world: Mat4, edit: &EditOverlay) {
    let selected = hex(palette::SELECTED);
    for [a, b] in mesh.edges() {
        let (Some(pa), Some(pb)) = (mesh.positions.get(a as usize), mesh.positions.get(b as usize)) else { continue };
        let on = edit.vertices.contains(&a) && edit.vertices.contains(&b);
        canvas.line(
            projector,
            world.transform_point3(*pa),
            world.transform_point3(*pb),
            if on { selected } else { hex(palette::WIRE) },
            1.0,
            1,
            true,
        );
    }
    for (index, point) in mesh.positions.iter().enumerate() {
        let on = edit.vertices.contains(&u32::try_from(index).unwrap_or(u32::MAX));
        canvas.dot(projector, world.transform_point3(*point), if on { selected } else { hex(palette::EDIT_VERTEX) }, if on { 5 } else { 4 }, true);
    }
}

/// A ground grid on z = 0 whose spacing follows the zoom, with the X and Y axes drawn in colour.
fn draw_grid(canvas: &mut Canvas, projector: &Projector, view: &ViewState, grid: [f32; 3]) {
    let scale = 10f32.powf((view.distance / 4.0).max(0.01).log10().floor());
    let lines = 40;
    let extent = scale * lines as f32;
    let centre = (view.target / scale).round() * scale;
    for step in -lines..=lines {
        let offset = step as f32 * scale;
        let (x, y) = (centre.x + offset, centre.y + offset);
        let fade = 1.0 - (offset.abs() / extent).powi(2);
        let alpha = 0.55 * fade;
        if x.abs() > scale * 0.01 {
            canvas.line(projector, Vec3::new(x, centre.y - extent, 0.0), Vec3::new(x, centre.y + extent, 0.0), grid, alpha, 1, true);
        }
        if y.abs() > scale * 0.01 {
            canvas.line(projector, Vec3::new(centre.x - extent, y, 0.0), Vec3::new(centre.x + extent, y, 0.0), grid, alpha, 1, true);
        }
    }
    canvas.line(projector, Vec3::new(centre.x - extent, 0.0, 0.0), Vec3::new(centre.x + extent, 0.0, 0.0), hex(palette::AXIS_X), 0.9, 1, true);
    canvas.line(projector, Vec3::new(0.0, centre.y - extent, 0.0), Vec3::new(0.0, centre.y + extent, 0.0), hex(palette::AXIS_Y), 0.9, 1, true);
}

/// Line drawings for objects without a surface.
fn draw_gizmo(
    canvas: &mut Canvas,
    projector: &Projector,
    scene: &Scene,
    id: Id,
    data: &ObjectData,
    world: Mat4,
    color: [f32; 3],
    bones: Option<&BoneOverlay>,
    fills: bool,
) {
    if let ObjectData::Armature(armature) = data {
        // Bones are drawn where their constraints put them.
        let posed = kition_scene_model::constraints::bone_matrices(scene, id, 0);
        let mut marks = armature_marks(scene, armature, &posed, world, color, bones);
        if !fills {
            marks.fills.clear();
        }
        draw_marks(canvas, projector, &marks);
        return;
    }
    let mut line = |a: Vec3, b: Vec3| canvas.line(projector, world.transform_point3(a), world.transform_point3(b), color, 1.0, 1, true);
    match data {
        ObjectData::Camera(camera) => {
            // A pyramid to the frame one unit in front, with a triangle marking up.
            let aspect = scene.render.width.max(1) as f32 / scene.render.height.max(1) as f32;
            let (w, h) = camera.frame_half_size(aspect);
            let corners = [Vec3::new(-w, -h, -1.0), Vec3::new(w, -h, -1.0), Vec3::new(w, h, -1.0), Vec3::new(-w, h, -1.0)];
            for (index, corner) in corners.iter().enumerate() {
                let next = corners.get((index + 1) % 4).copied().unwrap_or(*corner);
                line(*corner, next);
                let apex = if camera.orthographic { Vec3::new(corner.x, corner.y, 0.0) } else { Vec3::ZERO };
                line(apex, *corner);
            }
            line(Vec3::new(-w * 0.6, h * 1.1, -1.0), Vec3::new(0.0, h * 1.6, -1.0));
            line(Vec3::new(0.0, h * 1.6, -1.0), Vec3::new(w * 0.6, h * 1.1, -1.0));
            line(Vec3::new(w * 0.6, h * 1.1, -1.0), Vec3::new(-w * 0.6, h * 1.1, -1.0));
        }
        ObjectData::Light(light) => {
            let segments = 16;
            let radius = 0.2;
            let axes = [(Vec3::X, Vec3::Y), (Vec3::X, Vec3::Z), (Vec3::Y, Vec3::Z)];
            for (u, v) in axes {
                for step in 0..segments {
                    let a = std::f32::consts::TAU * step as f32 / segments as f32;
                    let b = std::f32::consts::TAU * (step + 1) as f32 / segments as f32;
                    line((u * a.cos() + v * a.sin()) * radius, (u * b.cos() + v * b.sin()) * radius);
                }
            }
            match light.kind {
                LightKind::Sun | LightKind::Spot | LightKind::Area => line(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.5)),
                LightKind::Point => {}
            }
        }
        ObjectData::Empty { size } => {
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                line(-axis * *size, axis * *size);
            }
        }
        ObjectData::Lattice(lattice) => {
            for (a, b) in lattice.edges() {
                if let (Some(a), Some(b)) = (lattice.points.get(a), lattice.points.get(b)) {
                    line(*a, *b);
                }
            }
        }
        ObjectData::Mesh(_) | ObjectData::Armature(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_) => {}
    }
}

/// Draws an armature's marks: the custom shapes' faces (depth-tested, not picked), then the
/// lines, then the dots over everything.
fn draw_marks(canvas: &mut Canvas, projector: &Projector, marks: &ArmatureMarks) {
    for (corners, shade) in &marks.fills {
        let corners = corners.map(|position| Corner {
            clip: projector.view_projection() * position.extend(1.0),
            depth: -(projector.view.transform_point3(position)).z,
            position,
            normal: Vec3::ZERO,
            uv: Vec2::ZERO,
            tangent: Vec4::ZERO,
        });
        canvas.triangle(projector, corners, draw::KEEP, draw::KEEP, true, &mut |_| [shade[0], shade[1], shade[2], 1.0]);
    }
    for ([a, b], shade) in &marks.lines {
        canvas.line(projector, *a, *b, *shade, 1.0, 1, !marks.front);
    }
    for (point, shade) in &marks.dots {
        canvas.dot(projector, *point, *shade, 4, false);
    }
}

/// What an armature draws, in world space: lines and filled triangles in their colours, and
/// dots at heads and tails while it is edited or posed (`front`: drawn over everything).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArmatureMarks {
    pub lines: Vec<([Vec3; 2], [f32; 3])>,
    /// Faces of custom shapes not drawn as wires, depth-tested.
    pub fills: Vec<([Vec3; 3], [f32; 3])>,
    pub dots: Vec<(Vec3, [f32; 3])>,
    pub front: bool,
}

/// The fill of a custom shape's faces: its line colour over a light grey.
pub fn shape_fill(line: [f32; 3]) -> [f32; 3] {
    mix(line, hex(palette::SHAPE_FILL), 0.7)
}

/// The marks of an armature (object matrix `world`, bones `posed`): octahedra from head to
/// tail, or a bone's custom shape (the shape object's mesh, its edges and, unless drawn as
/// wire, its faces) outside edit mode. `color` is the armature's own colour; the bones of the
/// armature being edited or posed take the selection's colours instead.
pub fn armature_marks(
    scene: &Scene,
    armature: &kition_scene_model::Armature,
    posed: &std::collections::BTreeMap<String, Mat4>,
    world: Mat4,
    color: [f32; 3],
    overlay: Option<&BoneOverlay>,
) -> ArmatureMarks {
    let mut marks = ArmatureMarks { front: overlay.is_some(), ..ArmatureMarks::default() };
    let editing = overlay.is_some_and(|overlay| !overlay.pose);
    for bone in &armature.bones {
        let matrix = if editing { bone.rest_matrix() } else { posed.get(&bone.name).copied().unwrap_or_else(|| bone.rest_matrix()) };
        let length = bone.length();
        let shade = match overlay {
            Some(overlay) if overlay.active.as_deref() == Some(bone.name.as_str()) => hex(palette::ACTIVE),
            Some(overlay) if overlay.selected.contains(&bone.name) => hex(palette::SELECTED),
            Some(_) => hex(palette::WIRE),
            None => color,
        };
        let to_world = world * matrix;
        let point = |x: f32, y: f32, z: f32| to_world.transform_point3(Vec3::new(x, y, z));
        let (head, tail) = (point(0.0, 0.0, 0.0), point(0.0, length, 0.0));
        let shaped =
            bone.shape.as_ref().filter(|_| !editing).and_then(|shape| Some((shape, scene.object(shape.object).ok()?.evaluated_mesh(scene)?)));
        if let Some((shape, mesh)) = shaped {
            let place = world * shape.matrix(matrix, length);
            let at = |vertex: u32| mesh.positions.get(vertex as usize).map(|point| place.transform_point3(*point));
            if !shape.wire {
                let fill = shape_fill(shade);
                for face in &mesh.faces {
                    let Some(first) = face.first().and_then(|vertex| at(*vertex)) else { continue };
                    for pair in face.windows(2).skip(1) {
                        if let [b, c] = pair
                            && let (Some(b), Some(c)) = (at(*b), at(*c))
                        {
                            marks.fills.push(([first, b, c], fill));
                        }
                    }
                }
            }
            for [a, b] in mesh.edges() {
                if let (Some(a), Some(b)) = (at(a), at(b)) {
                    marks.lines.push(([a, b], shade));
                }
            }
        } else {
            let waist = length * 0.1;
            let ring = [point(waist, waist, 0.0), point(0.0, waist, waist), point(-waist, waist, 0.0), point(0.0, waist, -waist)];
            for (index, corner) in ring.iter().enumerate() {
                let next = ring.get((index + 1) % 4).copied().unwrap_or(*corner);
                marks.lines.push(([*corner, next], shade));
                marks.lines.push(([head, *corner], shade));
                marks.lines.push(([*corner, tail], shade));
            }
        }
        if overlay.is_some() {
            marks.dots.push((head, shade));
            marks.dots.push((tail, shade));
        }
    }
    marks
}

/// Draws a two-pixel outline around selected objects, outside their silhouette and over
/// whatever is behind.
fn outline(canvas: &mut Canvas, overlays: &Overlays) {
    if overlays.selected.is_empty() {
        return;
    }
    let (width, height) = (canvas.width as usize, canvas.height as usize);
    // 0 nothing selected, 1 selected, 2 active, per pixel; and the box they cover.
    let mut mask = vec![0u8; width * height];
    let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (slot, object) in canvas.object.iter().enumerate() {
        if *object != NONE && overlays.selected.contains(object) {
            if let Some(cell) = mask.get_mut(slot) {
                *cell = if overlays.active == Some(*object) { 2 } else { 1 };
            }
            let (x, y) = (slot % width.max(1), slot / width.max(1));
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
        }
    }
    if left > right {
        return;
    }
    let reach = 2usize;
    let at = |x: usize, y: usize| mask.get(y * width + x).copied().unwrap_or(0);
    let mut marks: Vec<(usize, usize, bool)> = Vec::new();
    for y in top.saturating_sub(reach)..(bottom + reach + 1).min(height) {
        for x in left.saturating_sub(reach)..(right + reach + 1).min(width) {
            if at(x, y) != 0 {
                continue;
            }
            let mut found = 0u8;
            for dy in -2i64..=2 {
                for dx in -2i64..=2 {
                    if dx * dx + dy * dy > 5 {
                        continue;
                    }
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    found = found.max(at(nx as usize, ny as usize));
                }
            }
            if found != 0 {
                marks.push((x, y, found == 2));
            }
        }
    }
    let (selected, active) = (hex(palette::SELECTED), hex(palette::ACTIVE));
    for (x, y, is_active) in marks {
        canvas.blend(x as i64, y as i64, if is_active { active } else { selected }, 1.0);
    }
}

fn dim_outside(canvas: &mut Canvas, frame: [f32; 4], shade: [f32; 3]) {
    let [x0, y0, x1, y1] = frame;
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            if px < x0 || px > x1 || py < y0 || py > y1 {
                canvas.blend(i64::from(x), i64::from(y), shade, 0.5);
            }
        }
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Viewer-relative studio lighting: a key light from the upper left of the view, a soft fill,
/// and ambient, on both sides of a surface.
struct Studio {
    rotation: Mat3,
    view: Mat4,
    perspective: bool,
    key: Vec3,
    fill: Vec3,
}

impl Studio {
    fn new(projector: &Projector) -> Self {
        Self {
            rotation: Mat3::from_mat4(projector.view),
            view: projector.view,
            perspective: projector.perspective,
            key: Vec3::new(-0.35, 0.55, 0.76).normalize(),
            fill: Vec3::new(0.6, -0.2, 0.4).normalize(),
        }
    }

    fn shade(&self, fragment: Fragment, base: [f32; 3]) -> [f32; 3] {
        let mut normal = (self.rotation * fragment.normal).normalize_or_zero();
        let toward_eye = if self.perspective { -self.view.transform_point3(fragment.position) } else { Vec3::Z };
        if normal.dot(toward_eye) < 0.0 {
            normal = -normal;
        }
        let light = 0.22 + 0.68 * normal.dot(self.key).max(0.0) + 0.18 * normal.dot(self.fill).max(0.0);
        base.map(|channel| channel * light)
    }
}

/// One light, ready for shading.
struct LightInfo {
    kind: LightKind,
    position: Vec3,
    direction: Vec3,
    power: Vec3,
    spot_size: f32,
    radius: f32,
    shadow: Option<Arc<shadow::ShadowMap>>,
}

/// The scene's lights, ready for shading.
struct Lights {
    lights: Vec<LightInfo>,
    /// The world image, lighting surfaces in place of the world colour.
    environment: Option<Arc<environment::Environment>>,
}

impl Lights {
    fn of(scene: &Scene) -> Self {
        let mut lights = Vec::new();
        for object in scene.objects.iter().filter(|object| !object.hidden) {
            let ObjectData::Light(light) = &object.data else { continue };
            let Ok(world) = scene.world_matrix(object.id) else { continue };
            lights.push(LightInfo {
                kind: light.kind,
                position: world.transform_point3(Vec3::ZERO),
                direction: world.transform_vector3(Vec3::NEG_Z).normalize_or_zero(),
                power: light.color * light.power,
                spot_size: light.spot_size,
                radius: light.radius,
                shadow: None,
            });
        }
        Self { lights, environment: None }
    }

    /// Gives every light a shadow map of `casters`, `size` texels per side, reusing the maps in
    /// `cache` when neither the lights nor the casters moved.
    /// Draws each light's shadow map. With `alpha` (final renders), casters with clipped or
    /// blended alpha let light through by it, as the reference's transparent shadows do; the
    /// viewport's maps (which the GPU viewport draws the same way) keep every caster solid.
    fn cast_shadows(&mut self, scene: &Scene, casters: &[(Id, Arc<Prepared>, Mat4)], size: u32, taps: u32, alpha: bool, cache: &mut MeshCache) {
        let alphas: Vec<Option<shadow::CasterAlpha>> = if alpha {
            casters
                .iter()
                .map(|(id, _, _)| {
                    let material = scene.object(*id).ok().and_then(|object| object.material);
                    let surface = Surface::of(scene, material, &mut cache.textures);
                    let cutoff = match surface.alpha_mode {
                        kition_scene_model::AlphaMode::Opaque => return None,
                        kition_scene_model::AlphaMode::Clip => Some(surface.alpha_cutoff),
                        kition_scene_model::AlphaMode::Blend => None,
                    };
                    Some(shadow::CasterAlpha { alpha: surface.alpha, texture: surface.base_texture.clone(), cutoff })
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut key = cache::Fnv::new();
        key.word(size);
        key.word(taps);
        for (caster, (id, _, _)) in alphas.iter().zip(casters) {
            let Some(caster) = caster else { continue };
            key.word(*id);
            key.word(caster.alpha.to_bits());
            key.word(caster.cutoff.map_or(u32::MAX, f32::to_bits));
            let image = scene
                .object(*id)
                .ok()
                .and_then(|object| object.material)
                .and_then(|id| scene.material(id).ok())
                .and_then(|m| m.base_color_texture.as_deref());
            key.bytes(image.unwrap_or("").as_bytes());
        }
        for light in &self.lights {
            key.word(light.kind as u32);
            for value in
                [light.position, light.direction, light.power].iter().flat_map(|vector| vector.to_array()).chain([light.spot_size, light.radius])
            {
                key.word(value.to_bits());
            }
        }
        for (id, _, world) in casters {
            key.word(*id);
            key.long(cache.key(*id).unwrap_or(0));
            for value in world.to_cols_array() {
                key.word(value.to_bits());
            }
        }
        let key = key.finish();
        let maps = match cache.shadows(key) {
            Some(maps) => maps,
            None => {
                let casters: Vec<(Arc<Prepared>, Mat4)> = casters.iter().map(|(_, prepared, world)| (Arc::clone(prepared), *world)).collect();
                let bounds = shadow::caster_bounds(&casters);
                let maps: Vec<Option<Arc<shadow::ShadowMap>>> = self
                    .lights
                    .iter()
                    .map(|light| {
                        shadow::ShadowMap::build(
                            light.kind,
                            light.position,
                            light.direction,
                            light.spot_size,
                            light.radius,
                            if light.kind == LightKind::Sun { shadow::sun_bounds(&casters, light.direction) } else { bounds },
                            &casters,
                            &alphas,
                            size,
                            taps,
                        )
                        .map(Arc::new)
                    })
                    .collect();
                cache.store_shadows(key, maps.clone());
                maps
            }
        };
        for (light, map) in self.lights.iter_mut().zip(maps) {
            light.shadow = map;
        }
    }

    /// Whether any light casts shadows (shading then varies across a face).
    fn shadowed(&self) -> bool {
        self.lights.iter().any(|light| light.shadow.is_some())
    }

    /// Diffuse plus a glossy highlight from every light, with world light as ambient. Point,
    /// spot, and area power spreads as `power / (4π² d²)`; a sun's strength is irradiance, which
    /// a Lambertian surface sends back as `strength / π` per unit albedo (as the reference does).
    /// These are a point's lighting before shadows and occlusion: what varies slowly across a flat face,
    /// so it can be worked out once per face.
    fn terms(&self, scene: &Scene, projector: &Projector, fragment: Fragment, point: &SurfacePoint) -> Terms {
        let SurfacePoint { base, metallic, roughness, emission, .. } = *point;
        if self.lights.is_empty() && self.environment.is_none() {
            let studio = Vec3::from(Studio::new(projector).shade(fragment, base.into())) * 1.4;
            return Terms { ambient: studio, emission: Vec3::ZERO, lights: Vec::new() };
        }
        let eye = projector.eye();
        let to_eye = (eye - fragment.position).normalize_or_zero();
        let mut normal = fragment.normal;
        if normal.dot(to_eye) < 0.0 {
            normal = -normal;
        }
        let diffuse_color = base * (1.0 - metallic);
        let specular_color = Vec3::splat(0.04).lerp(base, metallic);
        let shininess = 2.0 / (roughness.clamp(0.03, 1.0).powi(4)) - 2.0;
        let ambient = match &self.environment {
            // The world image: diffuse light from all around, and the world seen in the surface
            // (sharper the smoother it is).
            Some(environment) => {
                let reflected = (-to_eye).reflect(normal);
                let gloss = (1.0 - roughness.clamp(0.0, 1.0)).powi(2);
                let specular_light = environment.diffuse(normal).lerp(environment.radiance(reflected), gloss);
                let cosine = normal.dot(to_eye).max(0.0);
                let fresnel = specular_color + (Vec3::ONE - specular_color) * (1.0 - cosine).powi(5) * gloss;
                diffuse_color * environment.diffuse(normal) + fresnel * specular_light
            }
            // A plain world colour is the same light from every direction: diffuse light, and
            // the same colour seen in the surface (so metals reflect it and plastics have their
            // sheen at grazing angles).
            None => {
                let light = scene.world.color * scene.world.strength;
                let gloss = (1.0 - roughness.clamp(0.0, 1.0)).powi(2);
                let cosine = normal.dot(to_eye).max(0.0);
                let fresnel = specular_color + (Vec3::ONE - specular_color) * (1.0 - cosine).powi(5) * gloss;
                (diffuse_color + fresnel) * light
            }
        };
        let mut terms = Terms { ambient, emission, lights: Vec::with_capacity(self.lights.len()) };
        for (index, light) in self.lights.iter().enumerate() {
            let (to_light, irradiance) = match light.kind {
                LightKind::Sun => (-light.direction, light.power / std::f32::consts::PI),
                kind => {
                    let offset = light.position - fragment.position;
                    let distance_squared = offset.length_squared().max(1e-4);
                    let to_light = offset / distance_squared.sqrt();
                    let mut strength = light.power / (4.0 * std::f32::consts::PI * std::f32::consts::PI * distance_squared);
                    if kind == LightKind::Spot {
                        let cosine = (-to_light).dot(light.direction);
                        let edge = (light.spot_size / 2.0).cos();
                        strength *= ((cosine - edge) / (1.0 - edge).max(1e-3)).clamp(0.0, 1.0);
                    }
                    if kind == LightKind::Area {
                        strength *= (-to_light).dot(light.direction).max(0.0);
                    }
                    (to_light, strength)
                }
            };
            let lambert = normal.dot(to_light).max(0.0);
            if lambert <= 0.0 {
                continue;
            }
            let half = (to_light + to_eye).normalize_or_zero();
            let gloss = normal.dot(half).max(0.0).powf(shininess) * (shininess + 8.0) / (8.0 * std::f32::consts::PI);
            terms.lights.push((index, (diffuse_color + specular_color * gloss) * irradiance * lambert, to_light));
        }
        terms
    }

    /// The colour at a point from its terms: each light as far as its shadow lets it through,
    /// and the world's light as far as `ambient` (occlusion) does.
    fn resolve(&self, terms: &Terms, fragment: Fragment, ambient: f32) -> [f32; 3] {
        let mut total = terms.ambient * ambient + terms.emission;
        for (index, contribution, to_light) in &terms.lights {
            let visible = self
                .lights
                .get(*index)
                .and_then(|light| light.shadow.as_ref())
                .map_or(1.0, |map| map.visibility(fragment.position, fragment.normal, *to_light));
            total += *contribution * visible;
        }
        total.into()
    }
}

/// What a material gives a point: its colour, metal and roughness, and light.
#[derive(Debug, Clone, Copy)]
struct SurfacePoint {
    base: Vec3,
    metallic: f32,
    roughness: f32,
    emission: Vec3,
    /// The base colour's alpha times the image's.
    alpha: f32,
}

/// An object's material, with its images decoded.
struct Surface {
    base: Vec3,
    metallic: f32,
    roughness: f32,
    emission: Vec3,
    base_texture: Option<Arc<texture::Texture>>,
    metallic_roughness_texture: Option<Arc<texture::Texture>>,
    normal_texture: Option<Arc<texture::Texture>>,
    normal_strength: f32,
    alpha: f32,
    alpha_mode: kition_scene_model::AlphaMode,
    alpha_cutoff: f32,
    transmission: f32,
    ior: f32,
}

impl Surface {
    fn of(scene: &Scene, material: Option<Id>, textures: &mut texture::TextureCache) -> Self {
        let material = material.and_then(|id| scene.material(id).ok());
        let Some(material) = material else {
            return Self {
                base: Vec3::splat(0.8),
                metallic: 0.0,
                roughness: 0.5,
                emission: Vec3::ZERO,
                base_texture: None,
                metallic_roughness_texture: None,
                normal_texture: None,
                normal_strength: 1.0,
                alpha: 1.0,
                alpha_mode: kition_scene_model::AlphaMode::Opaque,
                alpha_cutoff: 0.5,
                transmission: 0.0,
                ior: 1.5,
            };
        };
        Self {
            base: Vec3::new(material.base_color[0], material.base_color[1], material.base_color[2]),
            metallic: material.metallic,
            roughness: material.roughness,
            emission: material.emission * material.emission_strength,
            base_texture: material.base_color_texture.as_deref().and_then(|hash| textures.get(scene, hash, true)),
            metallic_roughness_texture: material.metallic_roughness_texture.as_deref().and_then(|hash| textures.get(scene, hash, false)),
            normal_texture: material.normal_texture.as_deref().and_then(|hash| textures.get(scene, hash, false)),
            normal_strength: material.normal_strength,
            alpha: material.base_color[3],
            alpha_mode: material.alpha_mode,
            alpha_cutoff: material.alpha_cutoff,
            transmission: material.transmission.clamp(0.0, 1.0),
            ior: material.ior,
        }
    }

    /// Whether the surface lets what is behind it show (drawn after opaque surfaces, back to
    /// front): blended alpha, or glass.
    fn see_through(&self) -> bool {
        self.alpha_mode == kition_scene_model::AlphaMode::Blend || self.transmission > 0.0
    }

    /// How much of a pixel the surface covers at `point` seen from `eye`: its alpha by the
    /// material's mode, less what glass lets through (Fresnel keeps grazing glass reflective).
    fn coverage(&self, point: &SurfacePoint, fragment: &Fragment, eye: Vec3) -> f32 {
        let coverage = match self.alpha_mode {
            kition_scene_model::AlphaMode::Opaque => 1.0,
            kition_scene_model::AlphaMode::Clip => {
                if point.alpha >= self.alpha_cutoff {
                    1.0
                } else {
                    0.0
                }
            }
            kition_scene_model::AlphaMode::Blend => point.alpha.clamp(0.0, 1.0),
        };
        if self.transmission <= 0.0 {
            return coverage;
        }
        let cosine = fragment.normal.dot((eye - fragment.position).normalize_or_zero()).abs();
        let f0 = ((self.ior - 1.0) / (self.ior + 1.0)).powi(2);
        let fresnel = f0 + (1.0 - f0) * (1.0 - cosine).powi(5);
        coverage * (1.0 - self.transmission * (1.0 - fresnel) * 0.9)
    }

    fn textured(&self) -> bool {
        self.base_texture.is_some() || self.metallic_roughness_texture.is_some() || self.normal_texture.is_some()
    }

    /// The fragment with its normal bent by the normal map (tangent space: X along the tangent,
    /// Y along the bitangent, Z along the normal; the strength scales X and Y, as glTF's
    /// `scale`). Unchanged without a map or a tangent.
    fn bend(&self, mut fragment: Fragment) -> Fragment {
        let Some(texture) = &self.normal_texture else { return fragment };
        let tangent = fragment.tangent.truncate();
        if tangent.length_squared() < 1e-12 {
            return fragment;
        }
        let normal = fragment.normal;
        let tangent = (tangent - normal * normal.dot(tangent)).normalize_or_zero();
        let bitangent = normal.cross(tangent) * if fragment.tangent.w < 0.0 { -1.0 } else { 1.0 };
        let sample = texture.sample(fragment.uv, fragment.uv_per_pixel);
        let x = (sample.x * 2.0 - 1.0) * self.normal_strength;
        let y = (sample.y * 2.0 - 1.0) * self.normal_strength;
        let z = sample.z * 2.0 - 1.0;
        let bent = (tangent * x + bitangent * y + normal * z).normalize_or_zero();
        if bent != Vec3::ZERO {
            fragment.normal = bent;
        }
        fragment
    }

    /// The material at a fragment: images multiply the material's values.
    fn at(&self, fragment: &Fragment) -> SurfacePoint {
        let mut point =
            SurfacePoint { base: self.base, metallic: self.metallic, roughness: self.roughness, emission: self.emission, alpha: self.alpha };
        if let Some(texture) = &self.base_texture {
            let sample = texture.sample(fragment.uv, fragment.uv_per_pixel);
            point.base *= sample.truncate();
            point.alpha *= sample.w;
        }
        if let Some(texture) = &self.metallic_roughness_texture {
            let sample = texture.sample(fragment.uv, fragment.uv_per_pixel);
            point.roughness *= sample.y;
            point.metallic *= sample.z;
        }
        point
    }
}

/// A point's lighting before shadows and occlusion.
#[derive(Debug, Clone)]
struct Terms {
    /// The world's light (scaled by occlusion).
    ambient: Vec3,
    emission: Vec3,
    /// Each light that reaches the point unshadowed: its index, contribution, and direction.
    lights: Vec<(usize, Vec3, Vec3)>,
}

/// The terms of the face last shaded, reused while the same flat face is drawn.
#[derive(Default)]
struct FaceTerms {
    face: Option<u32>,
    terms: Option<Terms>,
}

impl FaceTerms {
    /// The terms at `fragment` on `face`: worked out once per face of a flat mesh, at every pixel
    /// of a smooth one.
    fn get(&mut self, flat: bool, face: u32, work: impl FnOnce() -> Terms) -> &Terms {
        if !flat || self.face != Some(face) || self.terms.is_none() {
            self.face = Some(face);
            self.terms = Some(work());
        }
        self.terms.get_or_insert_with(|| Terms { ambient: Vec3::ZERO, emission: Vec3::ZERO, lights: Vec::new() })
    }
}

/// A picture `samples` times as large averaged down. With `display`, each sample goes through
/// the render's colour management first (Standard clips it at white), so the average is of what
/// is shown; without it, light brighter than white is kept (the passes compositing reads).
fn downsample(canvas: &Canvas, samples: u32, display: Option<&kition_scene_model::color::ColorManagement>) -> Canvas {
    let standard = display.is_none_or(kition_scene_model::color::ColorManagement::is_standard);
    if samples <= 1 && standard {
        return canvas.clone();
    }
    let samples = samples.max(1);
    let most = if display.is_none() { f32::MAX } else { 1.0 };
    let (width, height) = (canvas.width / samples, canvas.height / samples);
    let mut out = Canvas::new(width, height, [0.0; 4]);
    let count = (samples * samples) as f32;
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            let mut nearest = f32::INFINITY;
            for sy in 0..samples {
                for sx in 0..samples {
                    if let Some(pixel) = canvas.pixel(x * samples + sx, y * samples + sy) {
                        let pixel = match display {
                            Some(display) if !standard => {
                                let [r, g, b] = display.apply([pixel[0], pixel[1], pixel[2]]);
                                [r, g, b, pixel[3]]
                            }
                            _ => pixel,
                        };
                        for (index, (total, value)) in sum.iter_mut().zip(pixel).enumerate() {
                            *total += if value.is_finite() { value.clamp(0.0, if index == 3 { 1.0 } else { most }) } else { 0.0 };
                        }
                    }
                    let at = (y * samples + sy) as usize * canvas.width as usize + (x * samples + sx) as usize;
                    nearest = nearest.min(canvas.depth.get(at).copied().unwrap_or(f32::INFINITY));
                }
            }
            let slot = y as usize * width as usize + x as usize;
            if let Some(pixel) = out.color.get_mut(slot) {
                *pixel = sum.map(|value| value / count);
            }
            if let Some(depth) = out.depth.get_mut(slot) {
                *depth = nearest;
            }
        }
    }
    out
}
