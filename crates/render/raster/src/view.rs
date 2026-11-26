//! Where the viewport looks from: an orbit around a target point (the way a modeller turns the
//! view), or through one of the scene's cameras. Rotations use the same convention as camera
//! objects: X tilts the view up from looking straight down, Z turns it about the up axis.

use std::f32::consts::{FRAC_PI_2, PI};

use glam::{Mat3, Mat4, Vec3};
use kition_scene_model::scene::euler_matrix;
use kition_scene_model::{Camera, Id, ObjectData, Scene};
use serde::{Deserialize, Serialize};

/// The viewport's own view (not saved as a scene object).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewState {
    /// The point the view turns around.
    pub target: Vec3,
    /// How far the eye is from the target, in metres.
    pub distance: f32,
    /// Tilt: 0 looks straight down, π/2 looks level.
    pub tilt: f32,
    /// Turn about the up axis.
    pub turn: f32,
    /// Perspective (true) or orthographic.
    pub perspective: bool,
    /// The viewport lens in millimetres (against a 72 mm sensor).
    pub lens: f32,
    /// Looking through this scene camera instead of the orbit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Id>,
    /// Looking through a camera: how large its frame shows (1 fits the viewport). Moving in the
    /// camera view zooms and moves the frame, not the camera.
    #[serde(default = "unit", skip_serializing_if = "is_unit")]
    pub camera_zoom: f32,
    /// Looking through a camera: where its frame's centre is, as a fraction of half the viewport's
    /// width (x, right) and height (y, up) from the viewport's centre.
    #[serde(default, skip_serializing_if = "is_centred")]
    pub camera_offset: [f32; 2],
}

fn unit() -> f32 {
    1.0
}

fn is_unit(value: &f32) -> bool {
    *value == 1.0
}

fn is_centred(value: &[f32; 2]) -> bool {
    *value == [0.0, 0.0]
}

/// How far the camera frame zooms in and out of the viewport.
pub const CAMERA_ZOOM: (f32, f32) = (0.1, 16.0);
/// How far the camera frame's centre moves from the viewport's centre (in half viewports).
pub const CAMERA_OFFSET: f32 = 4.0;

impl Default for ViewState {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 11.0,
            tilt: 60f32.to_radians(),
            turn: 22f32.to_radians(),
            perspective: true,
            lens: 50.0,
            camera: None,
            camera_zoom: 1.0,
            camera_offset: [0.0, 0.0],
        }
    }
}

/// The sensor width the viewport lens is measured against.
const VIEWPORT_SENSOR: f32 = 72.0;

/// A named direction to look from, as the numeric-keypad views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Axis {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

impl ViewState {
    /// The rotation of the eye (the camera-object convention: it looks down its own -Z).
    pub fn rotation(&self) -> Mat3 {
        euler_matrix(Vec3::new(self.tilt, 0.0, self.turn))
    }

    /// Where the eye is.
    pub fn eye(&self) -> Vec3 {
        self.target + self.rotation() * Vec3::new(0.0, 0.0, self.distance)
    }

    /// Turns the view by screen-space drag amounts (radians).
    pub fn orbit(&mut self, turn: f32, tilt: f32) {
        self.camera = None;
        self.turn = (self.turn + turn).rem_euclid(2.0 * PI);
        self.tilt = (self.tilt + tilt).clamp(0.0, PI);
    }

    /// Moves the target across the screen by a fraction of the view's height.
    pub fn pan(&mut self, right: f32, up: f32) {
        self.camera = None;
        let rotation = self.rotation();
        let height = self.view_height();
        self.target += rotation * Vec3::new(-right * height, -up * height, 0.0);
    }

    /// Moves the eye towards (factor below 1) or away from the target.
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.distance = (self.distance * factor).clamp(0.01, 100_000.0);
        }
    }

    /// The height of the view at the target, in metres.
    fn view_height(&self) -> f32 {
        let fov = 2.0 * (VIEWPORT_SENSOR / (2.0 * self.lens)).atan();
        2.0 * self.distance * (fov / 2.0).tan()
    }

    /// Looks along an axis, orthographically, as a modeller's front, right, and top views do.
    pub fn align(&mut self, axis: Axis) {
        self.camera = None;
        let (tilt, turn) = match axis {
            Axis::Front => (FRAC_PI_2, 0.0),
            Axis::Back => (FRAC_PI_2, PI),
            Axis::Right => (FRAC_PI_2, FRAC_PI_2),
            Axis::Left => (FRAC_PI_2, -FRAC_PI_2),
            Axis::Top => (0.0, 0.0),
            Axis::Bottom => (PI, 0.0),
        };
        self.tilt = tilt;
        self.turn = turn.rem_euclid(2.0 * PI);
        self.perspective = false;
    }

    /// Frames a box (centre and radius) so it fills most of the view.
    pub fn frame(&mut self, centre: Vec3, radius: f32) {
        if !(centre.is_finite() && radius.is_finite()) {
            return;
        }
        let radius = radius.max(0.1);
        let fov = 2.0 * (VIEWPORT_SENSOR / (2.0 * self.lens)).atan();
        self.target = centre;
        self.distance = radius / (fov / 2.0).sin() * 1.1;
    }

    /// Fits `points` into a picture of `aspect` (width over height), looking the way the view
    /// looks, with `margin` (1.1 leaves a tenth) around them; a pure rule, so the same points
    /// always give the same view. Orthographic: the points' extent across the view is centred
    /// and fills the picture's tighter side. Perspective: the sphere round the points' box fills
    /// the picture's narrower field of view. A lone point is framed as a box 0.2 m across.
    pub fn fit(&mut self, points: &[Vec3], aspect: f32, margin: f32) {
        let finite: Vec<Vec3> = points.iter().copied().filter(|point| point.is_finite()).collect();
        let (Some(first), true) = (finite.first().copied(), aspect.is_finite() && aspect > 0.0 && margin.is_finite() && margin > 0.0) else { return };
        let (low, high) = finite.iter().fold((first, first), |(low, high), point| (low.min(*point), high.max(*point)));
        let centre = (low + high) / 2.0;
        let fov_x = 2.0 * (VIEWPORT_SENSOR / (2.0 * self.lens)).atan();
        let tan_x = (fov_x / 2.0).tan();
        // tan(half the vertical field of view): the lens covers the longer side.
        let tan_y = if aspect >= 1.0 { tan_x / aspect } else { tan_x };
        if self.perspective {
            let radius = finite.iter().map(|point| point.distance(centre)).fold(0.1f32, f32::max);
            let half_short = (tan_x * aspect.min(1.0 / aspect)).atan();
            self.target = centre;
            self.distance = (radius * margin / half_short.sin()).clamp(0.01, 100_000.0);
            return;
        }
        let rotation = self.rotation();
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        let across = |axis: Vec3| {
            finite.iter().map(|point| (*point - centre).dot(axis)).fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)))
        };
        let ((x0, x1), (y0, y1)) = (across(right), across(up));
        self.target = centre + right * ((x0 + x1) / 2.0) + up * ((y0 + y1) / 2.0);
        let (half_width, half_height) = (((x1 - x0) / 2.0).max(0.1), ((y1 - y0) / 2.0).max(0.1));
        let half = half_height.max(half_width / aspect) * margin;
        self.distance = (half / tan_y).clamp(0.01, 100_000.0);
    }

    /// The projection the viewport uses for a `width` × `height` picture.
    pub fn projector(&self, scene: &Scene, width: u32, height: u32) -> Projector {
        if let Some(camera) = self.camera.and_then(|id| camera_view(scene, id)) {
            let projector = camera.fit(width, height, scene.render.width, scene.render.height);
            return projector.framed(self.camera_zoom, self.camera_offset);
        }
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        let view = Mat4::from_rotation_translation(glam::Quat::from_mat3(&self.rotation()), self.eye()).inverse();
        let fov_x = 2.0 * (VIEWPORT_SENSOR / (2.0 * self.lens)).atan();
        // The lens covers the longer side.
        let fov_y = if aspect >= 1.0 { 2.0 * ((fov_x / 2.0).tan() / aspect).atan() } else { fov_x };
        let (near, far) = ((self.distance * 0.001).max(0.001), (self.distance * 100.0).max(1000.0));
        let projection = if self.perspective {
            Mat4::perspective_rh(fov_y, aspect, near, far)
        } else {
            let half_height = self.distance * (fov_y / 2.0).tan();
            Mat4::orthographic_rh(-half_height * aspect, half_height * aspect, -half_height, half_height, -far, far)
        };
        Projector { view, projection, width, height, frame: None, perspective: self.perspective }
    }
}

/// A camera object's view.
#[derive(Debug, Clone)]
pub struct CameraView {
    pub view: Mat4,
    pub camera: Camera,
}

/// The view through a scene camera, without the object's scale.
pub fn camera_view(scene: &Scene, id: Id) -> Option<CameraView> {
    let object = scene.object(id).ok()?;
    let ObjectData::Camera(camera) = &object.data else { return None };
    let world = scene.world_matrix(id).ok()?;
    let (_, rotation, translation) = world.to_scale_rotation_translation();
    Some(CameraView { view: Mat4::from_rotation_translation(rotation, translation).inverse(), camera: camera.clone() })
}

impl CameraView {
    /// The projection for a `width` × `height` picture showing the camera's `render_width` ×
    /// `render_height` frame, centred and as large as fits (the frame rectangle is recorded so
    /// the viewport can dim what is outside it).
    pub fn fit(&self, width: u32, height: u32, render_width: u32, render_height: u32) -> Projector {
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        let render_aspect = render_width.max(1) as f32 / render_height.max(1) as f32;
        // The frame's size in the picture, in pixels.
        let (frame_width, frame_height) =
            if render_aspect >= aspect { (width as f32, width as f32 / render_aspect) } else { (height as f32 * render_aspect, height as f32) };
        let scale = width as f32 / frame_width;
        let camera = &self.camera;
        let near = camera.clip_start.max(1e-4);
        let far = camera.clip_end.max(near * 2.0);
        // Fit the sensor to the render frame before fitting that frame into the viewport.
        let (half_frame_width, _) = camera.frame_half_size(render_aspect);
        let projection = if camera.orthographic {
            let half_width = half_frame_width * scale;
            let half_height = half_width / aspect;
            Mat4::orthographic_rh(-half_width, half_width, -half_height, half_height, near, far)
        } else {
            let half_width = half_frame_width * scale;
            let half_height = half_width / aspect;
            let depth = far / (near - far);
            // Reciprocal extents avoid rounding a near-PI f32 field of view back through
            // tan. Camera fit can produce very wide gates at legal portrait aspects.
            Mat4::from_cols(
                glam::Vec4::new(half_width.recip(), 0.0, 0.0, 0.0),
                glam::Vec4::new(0.0, half_height.recip(), 0.0, 0.0),
                glam::Vec4::new(0.0, 0.0, depth, -1.0),
                glam::Vec4::new(0.0, 0.0, near * depth, 0.0),
            )
        };
        let frame = [
            (width as f32 - frame_width) / 2.0,
            (height as f32 - frame_height) / 2.0,
            (width as f32 + frame_width) / 2.0,
            (height as f32 + frame_height) / 2.0,
        ];
        Projector { view: self.view, projection, width, height, frame: Some(frame), perspective: !camera.orthographic }
    }
}

/// World to screen for one picture.
#[derive(Debug, Clone)]
pub struct Projector {
    /// World to eye.
    pub view: Mat4,
    /// Eye to clip (depth from 0 at the near plane to 1 at the far plane).
    pub projection: Mat4,
    pub width: u32,
    pub height: u32,
    /// The render frame within the picture (x0, y0, x1, y1), when looking through a camera.
    pub frame: Option<[f32; 4]>,
    pub perspective: bool,
}

impl Projector {
    /// The same picture with its camera frame zoomed by `zoom` about the viewport's centre and
    /// moved by `offset` (half viewports, y up): the frame as a person zooms and moves it in the
    /// camera view. Unchanged at zoom 1 and offset 0, or for values out of range.
    pub fn framed(mut self, zoom: f32, offset: [f32; 2]) -> Projector {
        let [x, y] = offset;
        let usable = zoom.is_finite() && x.is_finite() && y.is_finite() && zoom > 0.0;
        if !usable || (zoom == 1.0 && x == 0.0 && y == 0.0) {
            return self;
        }
        // In clip space: x' = zoom·x + offset·w, so the picture scales about its centre and moves.
        let shift = Mat4::from_cols(
            glam::Vec4::new(zoom, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, zoom, 0.0, 0.0),
            glam::Vec4::Z,
            glam::Vec4::new(x, y, 0.0, 1.0),
        );
        self.projection = shift * self.projection;
        let (half_width, half_height) = (self.width as f32 / 2.0, self.height as f32 / 2.0);
        self.frame = self.frame.map(|[x0, y0, x1, y1]| {
            let across = |px: f32| half_width + (px - half_width) * zoom + x * half_width;
            let down = |py: f32| half_height + (py - half_height) * zoom - y * half_height;
            [across(x0), down(y0), across(x1), down(y1)]
        });
        self
    }

    /// World to clip.
    pub fn view_projection(&self) -> Mat4 {
        self.projection * self.view
    }

    /// The pixel position and depth of a clip-space point (x right, y down).
    pub fn to_screen(&self, clip: glam::Vec4) -> Vec3 {
        let ndc = clip.truncate() / clip.w;
        Vec3::new((ndc.x + 1.0) * 0.5 * self.width as f32, (1.0 - ndc.y) * 0.5 * self.height as f32, ndc.z)
    }

    /// The world point at `depth` (0 near, 1 far) under pixel (`x`, `y`).
    pub fn unproject(&self, x: f32, y: f32, depth: f32) -> Vec3 {
        let ndc = glam::Vec4::new(x / self.width.max(1) as f32 * 2.0 - 1.0, 1.0 - y / self.height.max(1) as f32 * 2.0, depth, 1.0);
        let world = self.view_projection().inverse() * ndc;
        world.truncate() / world.w
    }

    /// The world-space ray under pixel (`x`, `y`): origin and unit direction.
    pub fn ray(&self, x: f32, y: f32) -> (Vec3, Vec3) {
        // Keep the inverse and subtraction in double precision: in a translated camera,
        // rounding two nearby world points to f32 first can noticeably turn the ray.
        let inverse = (self.projection.as_dmat4() * self.view.as_dmat4()).inverse();
        let ndc_x = f64::from(x) / f64::from(self.width.max(1)) * 2.0 - 1.0;
        let ndc_y = 1.0 - f64::from(y) / f64::from(self.height.max(1)) * 2.0;
        let at = |depth| {
            let point = inverse * glam::DVec4::new(ndc_x, ndc_y, depth, 1.0);
            point.truncate() / point.w
        };
        let near = at(0.0);
        (near.as_vec3(), (at(0.5) - near).normalize_or_zero().as_vec3())
    }

    /// The eye position in world space.
    pub fn eye(&self) -> Vec3 {
        self.view.inverse().transform_point3(Vec3::ZERO)
    }

    /// The world-space distance one pixel covers at `point`.
    pub fn pixel_size_at(&self, point: Vec3) -> f32 {
        let a = self.unproject_at(point, 0.0);
        let b = self.unproject_at(point, 1.0);
        a.distance(b)
    }

    fn unproject_at(&self, point: Vec3, dx: f32) -> Vec3 {
        let clip = self.view_projection() * point.extend(1.0);
        let screen = self.to_screen(clip);
        self.unproject(screen.x + dx, screen.y, screen.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_projects_to_the_centre() {
        let view = ViewState::default();
        let projector = view.projector(&Scene::empty(), 800, 600);
        let screen = projector.to_screen(projector.view_projection() * Vec3::ZERO.extend(1.0));
        assert!((screen.x - 400.0).abs() < 1e-2 && (screen.y - 300.0).abs() < 1e-2, "{screen}");
    }

    #[test]
    fn the_front_view_looks_along_y_with_z_up() {
        let mut view = ViewState::default();
        view.align(Axis::Front);
        let projector = view.projector(&Scene::empty(), 800, 600);
        let up = projector.to_screen(projector.view_projection() * Vec3::Z.extend(1.0));
        let right = projector.to_screen(projector.view_projection() * Vec3::X.extend(1.0));
        assert!(up.y < 300.0 && (up.x - 400.0).abs() < 1e-2, "+Z is up: {up}");
        assert!(right.x > 400.0 && (right.y - 300.0).abs() < 1e-2, "+X is right: {right}");
    }

    #[test]
    fn a_zoomed_camera_frame_scales_about_the_centre_and_moves() {
        let scene = Scene::starter();
        let camera = scene.camera.unwrap();
        let fitted = camera_view(&scene, camera).unwrap().fit(800, 600, 1920, 1080);
        let framed = fitted.clone().framed(2.0, [0.5, 0.0]);
        let [x0, y0, x1, y1] = fitted.frame.unwrap();
        let [a0, b0, a1, b1] = framed.frame.unwrap();
        assert!(((a1 - a0) - 2.0 * (x1 - x0)).abs() < 1e-3 && ((b1 - b0) - 2.0 * (y1 - y0)).abs() < 1e-3);
        assert!(((a0 + a1) / 2.0 - 600.0).abs() < 1e-3 && ((b0 + b1) / 2.0 - 300.0).abs() < 1e-3);
        // The cube's centre moves with the frame.
        let before = fitted.to_screen(fitted.view_projection() * Vec3::ZERO.extend(1.0));
        let after = framed.to_screen(framed.view_projection() * Vec3::ZERO.extend(1.0));
        assert!((after.x - (400.0 + (before.x - 400.0) * 2.0 + 200.0)).abs() < 1e-2, "{before} {after}");
        assert!((after.y - (300.0 + (before.y - 300.0) * 2.0)).abs() < 1e-2, "{before} {after}");
        let same = fitted.clone().framed(1.0, [0.0, 0.0]);
        assert_eq!(same.projection, fitted.projection);
    }

    #[test]
    fn the_starter_camera_sees_the_cube() {
        let scene = Scene::starter();
        let camera = scene.camera.unwrap();
        let projector = camera_view(&scene, camera).unwrap().fit(1920, 1080, 1920, 1080);
        let screen = projector.to_screen(projector.view_projection() * Vec3::ZERO.extend(1.0));
        assert!((0.0..1920.0).contains(&screen.x) && (0.0..1080.0).contains(&screen.y) && (0.0..1.0).contains(&screen.z), "{screen}");
        assert!((projector.unproject(screen.x, screen.y, screen.z)).length() < 1e-3);
    }
}
