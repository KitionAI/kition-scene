//! The Scene kit's shader nodes: a material's shader group (`Tree::Shader`) compiled once into a
//! [`Program`] and evaluated at surface points into a [`Closure`] (the lobes a renderer scatters
//! light with, the light the surface sends out, and what it lets through). The path tracer draws
//! closures as they are; the fast renderer, the GPU viewport, and exports draw the one principled
//! surface that stands for them ([`Closure::flatten`]), per pixel or baked into images.
//!
//! The nodes follow the reference application's shader nodes (version 5): their outputs were
//! measured from its renders and bakes (`tests/shader_blender.rs`), never read from its source.

pub mod closure;
pub mod noise;
pub mod program;
pub mod textures;
pub mod voronoi;

use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

pub use closure::{Closure, Flat, Lobe, Phase, Principled, VolumeLobe};
pub use program::Program;
pub use textures::{Picture, Pictures};

/// Where an object's surface points sit: its transform and the box generated coordinates span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Space {
    world_to_object: Mat4,
    object_to_world: Mat4,
    /// Object-space normals to world space.
    normal_to_world: Mat3,
    min: Vec3,
    size: Vec3,
}

impl Space {
    /// An object's space: its world matrix and its (undeformed) bounds in object space.
    pub fn new(object_to_world: Mat4, min: Vec3, max: Vec3) -> Self {
        let world_to_object = object_to_world.inverse();
        let world_to_object = if world_to_object.is_finite() { world_to_object } else { Mat4::IDENTITY };
        let normal_to_world = Mat3::from_mat4(world_to_object).transpose();
        Self { world_to_object, object_to_world, normal_to_world, min, size: max - min }
    }

    /// World space as the object's, over the unit box around the origin.
    pub fn identity() -> Self {
        Self::new(Mat4::IDENTITY, Vec3::splat(-1.0), Vec3::splat(1.0))
    }

    /// A world point in object space.
    pub fn object(&self, world: Vec3) -> Vec3 {
        self.world_to_object.transform_point3(world)
    }

    /// A world point in generated coordinates: 0 to 1 across the bounds (0.5 across a flat axis).
    pub fn generated(&self, world: Vec3) -> Vec3 {
        let local = self.object(world);
        let axis = |value: f32, min: f32, size: f32| if size.abs() > 1e-8 { (value - min) / size } else { 0.5 };
        Vec3::new(axis(local.x, self.min.x, self.size.x), axis(local.y, self.min.y, self.size.y), axis(local.z, self.min.z, self.size.z))
    }

    /// A world direction (a normal) in object space.
    pub fn object_normal(&self, world: Vec3) -> Vec3 {
        (Mat3::from_mat4(self.object_to_world).transpose() * world).normalize_or_zero()
    }

    /// An object-space normal in world space.
    pub fn world_normal(&self, object: Vec3) -> Vec3 {
        (self.normal_to_world * object).normalize_or_zero()
    }
}

/// A surface point being shaded, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub position: Vec3,
    /// The shading normal (interpolated), facing the side the point is seen from.
    pub normal: Vec3,
    /// The face's own normal, on the same side.
    pub true_normal: Vec3,
    /// Towards the viewer.
    pub incoming: Vec3,
    pub uv: Vec2,
    /// How the position changes along U and V (zero without texture coordinates).
    pub dpdu: Vec3,
    pub dpdv: Vec3,
    /// The texture coordinates' tangent with the bitangent's sign in `w` (zero without one).
    pub tangent: Vec4,
    /// Seen from behind the face.
    pub backfacing: bool,
}

impl Point {
    /// A point at `position` facing `normal`, seen along it, with no texture coordinates.
    pub fn at(position: Vec3, normal: Vec3) -> Self {
        Self {
            position,
            normal,
            true_normal: normal,
            incoming: normal,
            uv: Vec2::ZERO,
            dpdu: Vec3::ZERO,
            dpdv: Vec3::ZERO,
            tangent: Vec4::ZERO,
            backfacing: false,
        }
    }

    /// The point moved by `offset` along the surface: its texture coordinates follow.
    pub fn moved(&self, offset: Vec3) -> Self {
        let mut out = *self;
        out.position += offset;
        let (uu, uv, vv) = (self.dpdu.dot(self.dpdu), self.dpdu.dot(self.dpdv), self.dpdv.dot(self.dpdv));
        let det = uu * vv - uv * uv;
        if det.abs() > 1e-20 {
            let (a, b) = (self.dpdu.dot(offset), self.dpdv.dot(offset));
            out.uv += Vec2::new((vv * a - uv * b) / det, (uu * b - uv * a) / det);
        }
        out
    }

    /// A tangent along the surface: the texture coordinates' when there is one, else one around
    /// the object's up axis.
    pub fn tangent_axis(&self) -> Vec3 {
        let normal = self.normal;
        let along = |t: Vec3| (t - normal * normal.dot(t)).normalize_or_zero();
        let from_uv = along(self.tangent.truncate());
        if from_uv != Vec3::ZERO {
            return from_uv;
        }
        let around = along(Vec3::Z.cross(self.position));
        if around != Vec3::ZERO { around } else { along(if normal.x.abs() < 0.9 { Vec3::X } else { Vec3::Y }) }
    }
}
