//! Lights in the reference's units: a point or spot light of P watts has intensity P / 4π
//! (W/sr) in every direction its cone allows, a sun of strength S gives irradiance S, and an
//! area light of P watts and area A is a one-sided Lambertian emitter of radiance P / (πA).
//! Point and spot lights with a radius are spheres; area lights are rectangles facing their -Z.

use std::f32::consts::PI;

use glam::{Mat4, Vec2, Vec3};
use kition_scene_model::{LightKind, ObjectData, Scene};

use crate::sampling::{Frame, cone_pdf, uniform_cone};

/// The reference's default spot blend: the share of the cone the edge softens over.
const SPOT_BLEND: f32 = 0.15;

#[derive(Debug, Clone, Copy)]
pub enum Shape {
    /// A sphere (radius 0: a point) of the given intensity.
    Sphere { centre: Vec3, radius: f32, intensity: Vec3, spot: Option<Spot> },
    /// A directional light: the direction light travels, and irradiance.
    Sun { direction: Vec3, irradiance: Vec3 },
    /// A rectangle: centre, half sides, the direction it emits towards, and radiance.
    Rect { centre: Vec3, half_u: Vec3, half_v: Vec3, normal: Vec3, radiance: Vec3 },
}

#[derive(Debug, Clone, Copy)]
pub struct Spot {
    /// The direction the spot points.
    pub axis: Vec3,
    pub cos_half: f32,
}

impl Spot {
    /// The cone's attenuation of light leaving along `direction`.
    pub fn mask(&self, direction: Vec3) -> f32 {
        let cos = direction.dot(self.axis);
        if cos <= self.cos_half {
            return 0.0;
        }
        let spread = ((1.0 - self.cos_half) * SPOT_BLEND).max(1e-6);
        let t = ((cos - self.cos_half) / spread).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// A sample of light reaching a point.
#[derive(Debug, Clone, Copy)]
pub struct LightSample {
    /// Towards the light.
    pub direction: Vec3,
    pub distance: f32,
    /// Radiance (area and sphere lights) or irradiance-equivalent (delta lights), divided by
    /// `pdf` already.
    pub contribution: Vec3,
    /// Solid-angle density; zero for delta lights (point, sun), which no other strategy can find.
    pub pdf: f32,
}

/// The scene's visible lights.
pub fn collect(scene: &Scene) -> Vec<Shape> {
    let mut out = Vec::new();
    for object in scene.objects.iter().filter(|object| !object.hidden) {
        let ObjectData::Light(light) = &object.data else { continue };
        let Ok(world) = scene.world_matrix(object.id) else { continue };
        let colour = light.color.max(Vec3::ZERO);
        let centre = world.transform_point3(Vec3::ZERO);
        let axis = world.transform_vector3(Vec3::NEG_Z).normalize_or_zero();
        match light.kind {
            LightKind::Point | LightKind::Spot => {
                let spot = (light.kind == LightKind::Spot).then(|| Spot { axis, cos_half: (light.spot_size.clamp(1e-3, PI) / 2.0).cos() });
                out.push(Shape::Sphere { centre, radius: light.radius.max(0.0), intensity: colour * (light.power / (4.0 * PI)), spot });
            }
            LightKind::Sun => out.push(Shape::Sun { direction: axis, irradiance: colour * light.power }),
            LightKind::Area => {
                let side = light.radius.max(1e-4);
                let (half_u, half_v) = (scaled(world, Vec3::X * side / 2.0), scaled(world, Vec3::Y * side / 2.0));
                let area = 4.0 * half_u.length() * half_v.length();
                if area > 0.0 {
                    out.push(Shape::Rect { centre, half_u, half_v, normal: axis, radiance: colour * (light.power / (PI * area)) });
                }
            }
        }
    }
    out
}

fn scaled(world: Mat4, v: Vec3) -> Vec3 {
    world.transform_vector3(v)
}

impl Shape {
    /// Samples light reaching `point`.
    pub fn sample(&self, point: Vec3, u: Vec2) -> Option<LightSample> {
        match *self {
            Shape::Sphere { centre, radius, intensity, spot } => {
                let offset = centre - point;
                let distance2 = offset.length_squared();
                let distance = distance2.sqrt();
                if distance <= radius || distance <= 1e-6 {
                    return None;
                }
                let to_centre = offset / distance;
                let mask = spot.map_or(1.0, |spot| spot.mask(-to_centre));
                if mask <= 0.0 {
                    return None;
                }
                if radius <= 1e-5 {
                    return Some(LightSample { direction: to_centre, distance, contribution: intensity * mask / distance2, pdf: 0.0 });
                }
                let sin2 = (radius * radius / distance2).min(1.0);
                let cos_max = (1.0 - sin2).max(0.0).sqrt();
                let local = uniform_cone(u, cos_max);
                let direction = Frame::new(to_centre).to_world(local).normalize_or_zero();
                // The distance to the sphere along the sampled direction.
                let b = direction.dot(offset);
                let c = distance2 - radius * radius;
                let hit = b - (b * b - c).max(0.0).sqrt();
                let pdf = cone_pdf(cos_max);
                let radiance = intensity * mask / (PI * radius * radius);
                Some(LightSample { direction, distance: hit.max(0.0), contribution: radiance / pdf, pdf })
            }
            Shape::Sun { direction, irradiance } => {
                Some(LightSample { direction: -direction, distance: f32::INFINITY, contribution: irradiance, pdf: 0.0 })
            }
            Shape::Rect { centre, half_u, half_v, normal, radiance } => {
                let target = centre + half_u * (2.0 * u.x - 1.0) + half_v * (2.0 * u.y - 1.0);
                let offset = target - point;
                let distance2 = offset.length_squared();
                let distance = distance2.sqrt();
                if distance <= 1e-6 {
                    return None;
                }
                let direction = offset / distance;
                let cos_light = (-direction).dot(normal);
                if cos_light <= 0.0 {
                    return None;
                }
                let area = 4.0 * half_u.length() * half_v.length();
                let pdf = distance2 / (area * cos_light);
                Some(LightSample { direction, distance, contribution: radiance / pdf, pdf })
            }
        }
    }

    /// Where a ray from `origin` along `direction` meets the light first, with its radiance and
    /// the density [`Shape::sample`] would have drawn that direction with.
    pub fn hit(&self, origin: Vec3, direction: Vec3) -> Option<(f32, Vec3, f32)> {
        match *self {
            Shape::Sphere { centre, radius, intensity, spot } => {
                if radius <= 1e-5 {
                    return None;
                }
                let offset = centre - origin;
                let b = direction.dot(offset);
                let c = offset.length_squared() - radius * radius;
                let disc = b * b - c;
                if c <= 0.0 || disc < 0.0 {
                    return None;
                }
                let t = b - disc.sqrt();
                if t <= 0.0 {
                    return None;
                }
                let distance = offset.length();
                let mask = spot.map_or(1.0, |spot| spot.mask(-offset / distance));
                if mask <= 0.0 {
                    return None;
                }
                let cos_max = (1.0 - (radius * radius / offset.length_squared()).min(1.0)).max(0.0).sqrt();
                Some((t, intensity * mask / (PI * radius * radius), cone_pdf(cos_max)))
            }
            Shape::Sun { .. } => None,
            Shape::Rect { centre, half_u, half_v, normal, radiance } => {
                let denominator = direction.dot(normal);
                // Only the side the light faces emits.
                if denominator >= -1e-6 {
                    return None;
                }
                let t = (centre - origin).dot(normal) / denominator;
                if t <= 0.0 {
                    return None;
                }
                let point = origin + direction * t - centre;
                let (lu, lv) = (half_u.length_squared(), half_v.length_squared());
                if lu <= 0.0 || lv <= 0.0 || point.dot(half_u).abs() > lu || point.dot(half_v).abs() > lv {
                    return None;
                }
                let area = 4.0 * lu.sqrt() * lv.sqrt();
                let pdf = t * t / (area * -denominator);
                Some((t, radiance, pdf))
            }
        }
    }
}
