//! The primitives a person adds from Add ▸ Mesh, with the parameters and defaults a modeller
//! expects (a 2 m cube, a 32 × 16 UV sphere, ...). Counts and sizes are checked against the
//! reference application in `tests/primitives.rs`.

use std::f32::consts::{PI, TAU};

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;

/// How the ends of a circle, cylinder, or cone are closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fill {
    /// Left open.
    Nothing,
    /// One n-gon.
    #[default]
    Ngon,
    /// A fan of triangles around a centre vertex.
    TriangleFan,
}

/// A primitive and its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Primitive {
    Plane {
        #[serde(default = "two")]
        size: f32,
    },
    Cube {
        #[serde(default = "two")]
        size: f32,
    },
    Circle {
        #[serde(default = "thirty_two")]
        vertices: u32,
        #[serde(default = "one")]
        radius: f32,
        #[serde(default = "nothing")]
        fill: Fill,
    },
    UvSphere {
        #[serde(default = "thirty_two")]
        segments: u32,
        #[serde(default = "sixteen")]
        rings: u32,
        #[serde(default = "one")]
        radius: f32,
    },
    IcoSphere {
        #[serde(default = "two_u")]
        subdivisions: u32,
        #[serde(default = "one")]
        radius: f32,
    },
    Cylinder {
        #[serde(default = "thirty_two")]
        vertices: u32,
        #[serde(default = "one")]
        radius: f32,
        #[serde(default = "two")]
        depth: f32,
        #[serde(default)]
        fill: Fill,
    },
    Cone {
        #[serde(default = "thirty_two")]
        vertices: u32,
        #[serde(default = "one")]
        radius1: f32,
        #[serde(default)]
        radius2: f32,
        #[serde(default = "two")]
        depth: f32,
        #[serde(default)]
        fill: Fill,
    },
    Torus {
        #[serde(default = "forty_eight")]
        major_segments: u32,
        #[serde(default = "twelve")]
        minor_segments: u32,
        #[serde(default = "one")]
        major_radius: f32,
        #[serde(default = "quarter")]
        minor_radius: f32,
    },
    Grid {
        #[serde(default = "ten")]
        x_subdivisions: u32,
        #[serde(default = "ten")]
        y_subdivisions: u32,
        #[serde(default = "two")]
        size: f32,
    },
}

fn one() -> f32 {
    1.0
}
fn two() -> f32 {
    2.0
}
fn quarter() -> f32 {
    0.25
}
fn two_u() -> u32 {
    2
}
fn ten() -> u32 {
    10
}
fn twelve() -> u32 {
    12
}
fn sixteen() -> u32 {
    16
}
fn thirty_two() -> u32 {
    32
}
fn forty_eight() -> u32 {
    48
}
fn nothing() -> Fill {
    Fill::Nothing
}

/// The largest vertex count one primitive may make.
const MAX_VERTICES: u64 = 500_000;

impl Primitive {
    /// The primitive's name, as an object made from it is first called.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Plane { .. } => "Plane",
            Self::Cube { .. } => "Cube",
            Self::Circle { .. } => "Circle",
            Self::UvSphere { .. } => "Sphere",
            Self::IcoSphere { .. } => "Icosphere",
            Self::Cylinder { .. } => "Cylinder",
            Self::Cone { .. } => "Cone",
            Self::Torus { .. } => "Torus",
            Self::Grid { .. } => "Grid",
        }
    }

    /// Builds the mesh, centred on the origin.
    pub fn build(&self) -> Result<Mesh> {
        self.check()?;
        let mut mesh = match *self {
            Self::Plane { size } => grid(1, 1, size),
            Self::Cube { size } => cube(size),
            Self::Circle { vertices, radius, fill } => circle(vertices, radius, fill),
            Self::UvSphere { segments, rings, radius } => uv_sphere(segments, rings, radius),
            Self::IcoSphere { subdivisions, radius } => ico_sphere(subdivisions, radius),
            Self::Cylinder { vertices, radius, depth, fill } => cone(vertices, radius, radius, depth, fill),
            Self::Cone { vertices, radius1, radius2, depth, fill } => cone(vertices, radius1, radius2, depth, fill),
            Self::Torus { major_segments, minor_segments, major_radius, minor_radius } => {
                torus(major_segments, minor_segments, major_radius, minor_radius)
            }
            Self::Grid { x_subdivisions, y_subdivisions, size } => grid(x_subdivisions, y_subdivisions, size),
        };
        mesh.smooth = matches!(self, Self::UvSphere { .. } | Self::Torus { .. });
        mesh.uvs = crate::uv::primitive_uvs(self, &mesh);
        Ok(mesh)
    }

    /// About how many vertices the primitive makes.
    fn vertex_estimate(&self) -> u64 {
        match *self {
            Self::Plane { .. } | Self::Cube { .. } => 8,
            Self::Circle { vertices, .. } | Self::Cylinder { vertices, .. } | Self::Cone { vertices, .. } => 2 * u64::from(vertices) + 2,
            Self::UvSphere { segments, rings, .. } => u64::from(segments) * u64::from(rings),
            Self::IcoSphere { subdivisions, .. } => 4u64.saturating_pow(subdivisions.saturating_sub(1)).saturating_mul(10).saturating_add(2),
            Self::Torus { major_segments, minor_segments, .. } => u64::from(major_segments) * u64::from(minor_segments),
            Self::Grid { x_subdivisions, y_subdivisions, .. } => (u64::from(x_subdivisions) + 1) * (u64::from(y_subdivisions) + 1),
        }
    }

    fn check(&self) -> Result<()> {
        if self.vertex_estimate() > MAX_VERTICES {
            return Err(SceneError::params(format!("the {} would have more than {MAX_VERTICES} vertices", self.display_name().to_lowercase())));
        }
        let positive = |name: &str, value: f32| {
            if value.is_finite() && value > 0.0 { Ok(()) } else { Err(SceneError::params(format!("{name} must be a positive number"))) }
        };
        let range = |name: &str, value: u32, low: u32, high: u32| {
            if (low..=high).contains(&value) { Ok(()) } else { Err(SceneError::params(format!("{name} must be between {low} and {high}"))) }
        };
        match *self {
            Self::Plane { size } | Self::Cube { size } => positive("size", size),
            Self::Circle { vertices, radius, .. } => {
                range("vertices", vertices, 3, 10_000)?;
                positive("radius", radius)
            }
            Self::UvSphere { segments, rings, radius } => {
                range("segments", segments, 3, 1_000)?;
                range("rings", rings, 3, 1_000)?;
                positive("radius", radius)
            }
            Self::IcoSphere { subdivisions, radius } => {
                range("subdivisions", subdivisions, 1, 8)?;
                positive("radius", radius)
            }
            Self::Cylinder { vertices, radius, depth, .. } => {
                range("vertices", vertices, 3, 10_000)?;
                positive("radius", radius)?;
                positive("depth", depth)
            }
            Self::Cone { vertices, radius1, radius2, depth, .. } => {
                range("vertices", vertices, 3, 10_000)?;
                if !(radius1.is_finite() && radius2.is_finite() && radius1 >= 0.0 && radius2 >= 0.0 && radius1 + radius2 > 0.0) {
                    return Err(SceneError::params("radius1 and radius2 must be zero or more, and not both zero"));
                }
                positive("depth", depth)
            }
            Self::Torus { major_segments, minor_segments, major_radius, minor_radius } => {
                range("majorSegments", major_segments, 3, 1_000)?;
                range("minorSegments", minor_segments, 3, 1_000)?;
                positive("majorRadius", major_radius)?;
                positive("minorRadius", minor_radius)
            }
            Self::Grid { x_subdivisions, y_subdivisions, size } => {
                range("xSubdivisions", x_subdivisions, 1, 1_000)?;
                range("ySubdivisions", y_subdivisions, 1, 1_000)?;
                positive("size", size)
            }
        }
    }
}

fn index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// A point on the ring of `count` points, the first on +Y, going towards +X.
fn ring_point(slot: u32, count: u32, radius: f32, z: f32) -> Vec3 {
    let angle = TAU * slot as f32 / count as f32;
    Vec3::new(radius * angle.sin(), radius * angle.cos(), z)
}

fn cube(size: f32) -> Mesh {
    let h = size / 2.0;
    let positions = (0..8u32)
        .map(|bits| Vec3::new(if bits & 4 == 0 { -h } else { h }, if bits & 2 == 0 { -h } else { h }, if bits & 1 == 0 { -h } else { h }))
        .collect();
    Mesh {
        positions,
        faces: vec![vec![0, 1, 3, 2], vec![2, 3, 7, 6], vec![6, 7, 5, 4], vec![4, 5, 1, 0], vec![2, 6, 4, 0], vec![7, 3, 1, 5]],
        ..Default::default()
    }
}

fn grid(columns: u32, rows: u32, size: f32) -> Mesh {
    let h = size / 2.0;
    let mut mesh = Mesh::default();
    for row in 0..=rows {
        for column in 0..=columns {
            mesh.positions.push(Vec3::new(-h + size * column as f32 / columns as f32, -h + size * row as f32 / rows as f32, 0.0));
        }
    }
    let at = |column: u32, row: u32| row * (columns + 1) + column;
    for row in 0..rows {
        for column in 0..columns {
            mesh.faces.push(vec![at(column, row), at(column + 1, row), at(column + 1, row + 1), at(column, row + 1)]);
        }
    }
    mesh
}

fn circle(count: u32, radius: f32, fill: Fill) -> Mesh {
    let mut mesh = Mesh { positions: (0..count).map(|slot| ring_point(slot, count, radius, 0.0)).collect(), ..Default::default() };
    // The ring runs clockwise seen from above; faces are reversed to face up.
    match fill {
        Fill::Nothing => mesh.loose_edges = (0..count).map(|slot| [slot, (slot + 1) % count]).collect(),
        Fill::Ngon => mesh.faces.push((0..count).rev().collect()),
        Fill::TriangleFan => {
            let centre = mesh.vertex_count();
            mesh.positions.push(Vec3::ZERO);
            mesh.faces.extend((0..count).map(|slot| vec![centre, (slot + 1) % count, slot]));
        }
    }
    mesh
}

fn uv_sphere(segments: u32, rings: u32, radius: f32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.positions.push(Vec3::new(0.0, 0.0, radius));
    for ring in 1..rings {
        let polar = PI * ring as f32 / rings as f32;
        for segment in 0..segments {
            mesh.positions.push(ring_point(segment, segments, radius * polar.sin(), radius * polar.cos()));
        }
    }
    let bottom = mesh.vertex_count();
    mesh.positions.push(Vec3::new(0.0, 0.0, -radius));
    let at = |ring: u32, segment: u32| 1 + (ring - 1) * segments + segment % segments;
    for segment in 0..segments {
        mesh.faces.push(vec![0, at(1, segment + 1), at(1, segment)]);
        mesh.faces.push(vec![bottom, at(rings - 1, segment), at(rings - 1, segment + 1)]);
    }
    for ring in 1..rings - 1 {
        for segment in 0..segments {
            mesh.faces.push(vec![at(ring, segment), at(ring, segment + 1), at(ring + 1, segment + 1), at(ring + 1, segment)]);
        }
    }
    orient_outward(&mut mesh, |_| Vec3::ZERO);
    mesh
}

/// The regular icosahedron with a vertex on each pole and an edge of its upper ring on +X.
const ICOSAHEDRON: [[f32; 3]; 12] = [
    [0.0, 0.0, -1.0],
    [0.723_6, -0.525_72, -0.447_21],
    [-0.276_39, -0.850_64, -0.447_21],
    [-0.894_42, 0.0, -0.447_21],
    [-0.276_39, 0.850_64, -0.447_21],
    [0.723_6, 0.525_72, -0.447_21],
    [0.276_39, -0.850_64, 0.447_21],
    [-0.723_6, -0.525_72, 0.447_21],
    [-0.723_6, 0.525_72, 0.447_21],
    [0.276_39, 0.850_64, 0.447_21],
    [0.894_42, 0.0, 0.447_21],
    [0.0, 0.0, 1.0],
];

const ICOSAHEDRON_FACES: [[u32; 3]; 20] = [
    [0, 1, 2],
    [1, 0, 5],
    [0, 2, 3],
    [0, 3, 4],
    [0, 4, 5],
    [1, 5, 10],
    [2, 1, 6],
    [3, 2, 7],
    [4, 3, 8],
    [5, 4, 9],
    [1, 10, 6],
    [2, 6, 7],
    [3, 7, 8],
    [4, 8, 9],
    [5, 9, 10],
    [6, 10, 11],
    [7, 6, 11],
    [8, 7, 11],
    [9, 8, 11],
    [10, 9, 11],
];

fn ico_sphere(subdivisions: u32, radius: f32) -> Mesh {
    let mut positions: Vec<Vec3> = ICOSAHEDRON.iter().map(|point| Vec3::from(*point).normalize()).collect();
    let mut faces: Vec<[u32; 3]> = ICOSAHEDRON_FACES.to_vec();
    for _ in 1..subdivisions {
        let mut middles = std::collections::HashMap::new();
        let mut middle = |a: u32, b: u32, positions: &mut Vec<Vec3>| -> u32 {
            let key = (a.min(b), a.max(b));
            *middles.entry(key).or_insert_with(|| {
                let pa = positions.get(a as usize).copied().unwrap_or(Vec3::Z);
                let pb = positions.get(b as usize).copied().unwrap_or(Vec3::Z);
                positions.push(((pa + pb) * 0.5).normalize_or_zero());
                index(positions.len() - 1)
            })
        };
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let ab = middle(a, b, &mut positions);
            let bc = middle(b, c, &mut positions);
            let ca = middle(c, a, &mut positions);
            next.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        faces = next;
    }
    let mut mesh = Mesh {
        positions: positions.into_iter().map(|point| point * radius).collect(),
        faces: faces.into_iter().map(Vec::from).collect(),
        ..Default::default()
    };
    orient_outward(&mut mesh, |_| Vec3::ZERO);
    mesh
}

fn cone(count: u32, bottom_radius: f32, top_radius: f32, depth: f32, fill: Fill) -> Mesh {
    let h = depth / 2.0;
    let mut mesh = Mesh::default();
    // A zero radius closes that end in one vertex.
    let ring = |mesh: &mut Mesh, radius: f32, z: f32| -> Vec<u32> {
        if radius == 0.0 {
            mesh.positions.push(Vec3::new(0.0, 0.0, z));
            vec![mesh.vertex_count() - 1; count as usize]
        } else {
            (0..count)
                .map(|slot| {
                    mesh.positions.push(ring_point(slot, count, radius, z));
                    mesh.vertex_count() - 1
                })
                .collect()
        }
    };
    let bottom = ring(&mut mesh, bottom_radius, -h);
    let top = ring(&mut mesh, top_radius, h);
    let get = |ring: &[u32], slot: u32| ring.get((slot % count) as usize).copied().unwrap_or(0);
    for slot in 0..count {
        let quad = [get(&bottom, slot), get(&bottom, slot + 1), get(&top, slot + 1), get(&top, slot)];
        let mut face: Vec<u32> = Vec::with_capacity(4);
        for corner in quad {
            if !face.contains(&corner) {
                face.push(corner);
            }
        }
        if face.len() >= 3 {
            mesh.faces.push(face);
        }
    }
    for (ring, radius, z) in [(&bottom, bottom_radius, -h), (&top, top_radius, h)] {
        if radius == 0.0 {
            continue;
        }
        match fill {
            Fill::Nothing => {}
            Fill::Ngon => mesh.faces.push(ring.clone()),
            Fill::TriangleFan => {
                mesh.positions.push(Vec3::new(0.0, 0.0, z));
                let centre = mesh.vertex_count() - 1;
                mesh.faces.extend((0..count).map(|slot| vec![centre, get(ring, slot), get(ring, slot + 1)]));
            }
        }
    }
    orient_outward(&mut mesh, |_| Vec3::ZERO);
    mesh
}

fn torus(major: u32, minor: u32, major_radius: f32, minor_radius: f32) -> Mesh {
    let mut mesh = Mesh::default();
    for step in 0..major {
        let around = TAU * step as f32 / major as f32;
        let (direction, centre) = (Vec3::new(around.cos(), around.sin(), 0.0), Vec3::new(around.cos(), around.sin(), 0.0) * major_radius);
        for tube in 0..minor {
            let angle = TAU * tube as f32 / minor as f32;
            mesh.positions.push(centre + direction * (minor_radius * angle.cos()) + Vec3::Z * (minor_radius * angle.sin()));
        }
    }
    let at = |step: u32, tube: u32| (step % major) * minor + tube % minor;
    for step in 0..major {
        for tube in 0..minor {
            mesh.faces.push(vec![at(step, tube), at(step + 1, tube), at(step + 1, tube + 1), at(step, tube + 1)]);
        }
    }
    orient_outward(&mut mesh, |centre| {
        let flat = Vec3::new(centre.x, centre.y, 0.0).normalize_or_zero();
        flat * major_radius
    });
    mesh
}

/// Reverses faces whose normal points towards `inside(face centre)`.
fn orient_outward(mesh: &mut Mesh, inside: impl Fn(Vec3) -> Vec3) {
    let flips: Vec<bool> = mesh
        .faces
        .iter()
        .map(|face| {
            let centre = mesh.face_center(face);
            mesh.face_normal(face).dot(centre - inside(centre)) < 0.0
        })
        .collect();
    for (face, flip) in mesh.faces.iter_mut().zip(flips) {
        if flip {
            face.reverse();
        }
    }
}

/// A photographer's backdrop (a cyclorama): a floor `depth` metres deep centred on the origin,
/// curving up through a quarter circle of `radius` behind it into a wall `height` metres high,
/// `width` metres across X, smooth, with no edge between floor and wall, so a render shows no
/// horizon. The floor faces up and the wall faces the front (−Y); texture coordinates run
/// across (U) and along the profile (V).
pub fn backdrop(width: f32, depth: f32, height: f32, radius: f32, segments: u32) -> Result<Mesh> {
    let sizes = [width, depth, height, radius];
    if sizes.iter().any(|size| !size.is_finite() || *size <= 0.0 || *size > 100_000.0) {
        return Err(SceneError::InvalidParams("the backdrop's width, depth, height, and radius must be more than 0".into()));
    }
    if height < radius {
        return Err(SceneError::InvalidParams("the backdrop's height must be at least its radius".into()));
    }
    if !(1..=256).contains(&segments) {
        return Err(SceneError::InvalidParams("the backdrop's curve takes 1 to 256 segments".into()));
    }
    // The profile in (y, z): the floor's front edge, the curve, the top of the wall.
    let front = -depth / 2.0;
    let start = depth / 2.0;
    let mut profile = vec![(front, 0.0)];
    for step in 0..=segments {
        let angle = step as f32 / segments as f32 * PI / 2.0;
        profile.push((start + radius * angle.sin(), radius - radius * angle.cos()));
    }
    if height > radius {
        profile.push((start + radius, height));
    }
    let mut along = vec![0.0f32];
    for pair in profile.windows(2) {
        if let [(y0, z0), (y1, z1)] = pair {
            let last = along.last().copied().unwrap_or(0.0);
            along.push(last + ((y1 - y0).powi(2) + (z1 - z0).powi(2)).sqrt());
        }
    }
    let total = along.last().copied().unwrap_or(1.0).max(f32::EPSILON);
    let mut mesh = Mesh::default();
    for (y, z) in &profile {
        mesh.positions.push(Vec3::new(-width / 2.0, *y, *z));
        mesh.positions.push(Vec3::new(width / 2.0, *y, *z));
    }
    for index in 0..profile.len().saturating_sub(1) {
        let a = u32::try_from(index * 2).map_err(|_| SceneError::InvalidParams("the backdrop is too large".into()))?;
        mesh.faces.push(vec![a, a + 1, a + 3, a + 2]);
        let (v0, v1) = (along.get(index).copied().unwrap_or(0.0) / total, along.get(index + 1).copied().unwrap_or(1.0) / total);
        mesh.uvs.push(vec![glam::Vec2::new(0.0, v0), glam::Vec2::new(1.0, v0), glam::Vec2::new(1.0, v1), glam::Vec2::new(0.0, v1)]);
    }
    mesh.smooth = true;
    Ok(mesh)
}
