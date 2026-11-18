//! A scene as a USDA layer: Z-up metres, one Xform per object (its mesh, camera, or light
//! inside it, and its children nested), preview-surface materials, and transform time samples.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kition_scene_exchange::chosen;
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Id, LightKind, Material, Mesh, ObjectData, Scene, SceneError};

use crate::import::{DISTANT_INTENSITY_PER_STRENGTH, OBJECT_NAME, SPHERE_INTENSITY_PER_WATT};

/// A number as USDA writes it (non-finite values as zero).
fn number(value: f32) -> String {
    if value.is_finite() { format!("{value}") } else { "0".to_string() }
}

fn double(value: f64) -> String {
    if value.is_finite() { format!("{value}") } else { "0".to_string() }
}

fn tuple(values: &[f32]) -> String {
    format!("({})", values.iter().map(|value| number(*value)).collect::<Vec<_>>().join(", "))
}

fn vec3(value: Vec3) -> String {
    tuple(&value.to_array())
}

fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A valid prim name: letters, digits, and underscores, not starting with a digit.
fn identifier(name: &str) -> String {
    let mut out: String = name.chars().map(|character| if character.is_ascii_alphanumeric() { character } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|character: char| character.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// A name unique among `taken` (which it joins).
fn unique(name: String, taken: &mut BTreeSet<String>) -> String {
    let mut candidate = name.clone();
    let mut counter = 1;
    while taken.contains(&candidate) {
        candidate = format!("{name}_{counter}");
        counter += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

/// Translation, XYZ Euler rotation in degrees, and scale.
type Parts = (Vec3, Vec3, Vec3);

fn decompose(matrix: Mat4) -> Option<Parts> {
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    let rebuilt = Mat4::from_scale_rotation_translation(scale, rotation, translation);
    let tolerance = 1e-4 * matrix.to_cols_array().iter().fold(1.0f32, |most, value| most.max(value.abs()));
    if !rebuilt.abs_diff_eq(matrix, tolerance) || !scale.is_finite() {
        return None;
    }
    let euler = kition_scene_model::scene::matrix_euler(kition_scene_model::glam::Mat3::from_quat(rotation));
    Some((translation, Vec3::new(euler.x.to_degrees(), euler.y.to_degrees(), euler.z.to_degrees()), scale))
}

fn matrix4d(matrix: Mat4) -> String {
    let columns = matrix.to_cols_array_2d();
    format!("( {} )", columns.iter().map(|column| tuple(column)).collect::<Vec<_>>().join(", "))
}

struct Writer<'a> {
    scene: &'a Scene,
    out: String,
    children: BTreeMap<Option<Id>, Vec<Id>>,
    /// Material prim paths by material.
    materials: HashMap<Id, String>,
    /// Local matrices per frame for animated objects.
    samples: HashMap<Id, Vec<(i64, Mat4)>>,
    /// The aspect ratio (height over width) of the render.
    aspect: f32,
    /// Each object's prim path.
    paths: HashMap<Id, String>,
    /// The Skeleton prim each exported armature writes.
    skeletons: HashMap<Id, String>,
    /// The blend-shape-only skeleton each shape-keyed mesh without an armature writes.
    carriers: HashMap<Id, String>,
    /// The frames sampled for animation (first, last), when anything is keyed.
    frames: Option<(i64, i64)>,
    /// Whether the layer goes in a package, so its materials' images go beside it.
    package: bool,
    /// The images a package carries: (path in the package, image hash).
    files: Vec<(String, String)>,
}

/// Joint translations, rotations, and scales, written out.
type JointValues = (Vec<String>, Vec<String>, Vec<String>);

/// How a mesh object is written: its base mesh with its skinning and shape keys, or baked.
struct Skin {
    mesh: Mesh,
    /// The exported armature that deforms it.
    rig: Option<Id>,
}

impl Writer<'_> {
    fn line(&mut self, depth: usize, text: &str) {
        for _ in 0..if text.is_empty() { 0 } else { depth } {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn local(&self, id: Id) -> Result<Mat4, SceneError> {
        let object = self.scene.object(id)?;
        let world = self.scene.world_matrix(id)?;
        let parent = object.parent.filter(|parent| self.children.get(&Some(*parent)).is_some_and(|children| children.contains(&id)));
        Ok(match parent {
            Some(parent) => self.scene.world_matrix(parent)?.inverse() * world,
            None => world,
        })
    }

    fn transform_ops(&mut self, depth: usize, id: Id) -> Result<(), SceneError> {
        let local = self.local(id)?;
        let samples = self.samples.get(&id).cloned().unwrap_or_default();
        let decomposed: Option<Vec<(i64, Parts)>> = samples.iter().map(|(frame, matrix)| decompose(*matrix).map(|parts| (*frame, parts))).collect();
        match (decompose(local), samples.is_empty()) {
            (Some((translate, rotate, scale)), true) => {
                let translate =
                    format!("({}, {}, {})", double(f64::from(translate.x)), double(f64::from(translate.y)), double(f64::from(translate.z)));
                self.line(depth, &format!("float3 xformOp:rotateXYZ = {}", vec3(rotate)));
                self.line(depth, &format!("float3 xformOp:scale = {}", vec3(scale)));
                self.line(depth, &format!("double3 xformOp:translate = {translate}"));
                self.line(depth, "uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:rotateXYZ\", \"xformOp:scale\"]");
            }
            (_, false) if decomposed.is_some() => {
                let decomposed = decomposed.unwrap_or_default();
                // Keep Euler angles continuous between frames.
                let mut rotations: Vec<Vec3> = decomposed.iter().map(|(_, parts)| parts.1).collect();
                for index in 1..rotations.len() {
                    let (Some(previous), Some(current)) = (rotations.get(index - 1).copied(), rotations.get_mut(index)) else { continue };
                    let turns = ((previous - *current) / 360.0).round();
                    *current += turns * 360.0;
                }
                let samples_of = |pick: &dyn Fn(usize, &Parts) -> String| -> Vec<String> {
                    decomposed.iter().enumerate().map(|(index, (frame, parts))| format!("{frame}: {},", pick(index, parts))).collect()
                };
                let blocks = [
                    ("float3 xformOp:rotateXYZ", samples_of(&|index, _| vec3(rotations.get(index).copied().unwrap_or(Vec3::ZERO)))),
                    ("float3 xformOp:scale", samples_of(&|_, parts| vec3(parts.2))),
                    (
                        "double3 xformOp:translate",
                        samples_of(&|_, parts| {
                            format!("({}, {}, {})", double(f64::from(parts.0.x)), double(f64::from(parts.0.y)), double(f64::from(parts.0.z)))
                        }),
                    ),
                ];
                for (attribute, values) in blocks {
                    self.line(depth, &format!("{attribute}.timeSamples = {{"));
                    for value in values {
                        self.line(depth + 1, &value);
                    }
                    self.line(depth, "}");
                }
                self.line(depth, "uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:rotateXYZ\", \"xformOp:scale\"]");
            }
            _ => {
                if samples.is_empty() {
                    self.line(depth, &format!("matrix4d xformOp:transform = {}", matrix4d(local)));
                } else {
                    self.line(depth, "matrix4d xformOp:transform.timeSamples = {");
                    for (frame, matrix) in &samples {
                        self.line(depth + 1, &format!("{frame}: {},", matrix4d(*matrix)));
                    }
                    self.line(depth, "}");
                }
                self.line(depth, "uniform token[] xformOpOrder = [\"xformOp:transform\"]");
            }
        }
        Ok(())
    }

    fn object(&mut self, depth: usize, id: Id, prim: &str, path: &str) -> Result<(), SceneError> {
        let object = self.scene.object(id)?.clone();
        // Armatures, and shape-keyed meshes animated through a carrier skeleton, are skeleton roots.
        let kind = if self.skeletons.contains_key(&id) || self.carriers.contains_key(&id) { "SkelRoot" } else { "Xform" };
        self.line(depth, &format!("def {kind} {}", quoted(prim)));
        self.line(depth, "{");
        if object.name != prim {
            self.line(depth + 1, &format!("custom string {OBJECT_NAME} = {}", quoted(&object.name)));
        }
        self.transform_ops(depth + 1, id)?;
        match &object.data {
            // Curves and texts go out as the meshes they make.
            ObjectData::Mesh(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_) => self.mesh(depth + 1, id, prim)?,
            ObjectData::Camera(camera) => {
                let width_scale = |value: f32| value / 100.0;
                self.line(depth + 1, "");
                self.line(depth + 1, &format!("def Camera {}", quoted(prim)));
                self.line(depth + 1, "{");
                self.line(depth + 2, &format!("float2 clippingRange = {}", tuple(&[camera.clip_start, camera.clip_end])));
                self.line(depth + 2, &format!("float focalLength = {}", number(width_scale(camera.lens))));
                let (half_width, half_height) = camera.frame_half_size(1.0 / self.aspect);
                let unit = if camera.orthographic { 20.0 } else { camera.lens / 50.0 };
                let aperture = half_width * unit;
                self.line(depth + 2, &format!("float horizontalAperture = {}", number(aperture)));
                self.line(depth + 2, &format!("token projection = {}", quoted(if camera.orthographic { "orthographic" } else { "perspective" })));
                self.line(depth + 2, &format!("float verticalAperture = {}", number(half_height * unit)));
                self.line(depth + 1, "}");
            }
            ObjectData::Light(light) => {
                let (kind, schemas) = match light.kind {
                    LightKind::Sun => ("DistantLight", "\"ColorSpaceAPI\""),
                    LightKind::Area => ("RectLight", "\"ColorSpaceAPI\""),
                    LightKind::Spot => ("SphereLight", "\"ShapingAPI\", \"ColorSpaceAPI\""),
                    LightKind::Point => ("SphereLight", "\"ColorSpaceAPI\""),
                };
                let intensity = match light.kind {
                    LightKind::Sun => f64::from(light.power) * DISTANT_INTENSITY_PER_STRENGTH,
                    _ => f64::from(light.power) * SPHERE_INTENSITY_PER_WATT,
                };
                self.line(depth + 1, "");
                self.line(depth + 1, &format!("def {kind} {} (", quoted(prim)));
                self.line(depth + 2, &format!("prepend apiSchemas = [{schemas}]"));
                self.line(depth + 1, ")");
                self.line(depth + 1, "{");
                self.line(depth + 2, &format!("color3f inputs:color = {}", vec3(light.color)));
                if light.kind == LightKind::Area {
                    self.line(depth + 2, &format!("float inputs:height = {}", number(light.radius)));
                }
                self.line(depth + 2, &format!("float inputs:intensity = {}", double(intensity)));
                self.line(depth + 2, "bool inputs:normalize = 1");
                if matches!(light.kind, LightKind::Point | LightKind::Spot) {
                    self.line(depth + 2, &format!("float inputs:radius = {}", number(light.radius)));
                }
                if light.kind == LightKind::Spot {
                    self.line(depth + 2, &format!("float inputs:shaping:cone:angle = {}", number(light.spot_size.to_degrees() / 2.0)));
                }
                if light.kind == LightKind::Area {
                    self.line(depth + 2, &format!("float inputs:width = {}", number(light.radius)));
                }
                if matches!(light.kind, LightKind::Point | LightKind::Spot) && light.radius <= 0.0 {
                    self.line(depth + 2, "bool treatAsPoint = 1");
                }
                self.line(depth + 1, "}");
            }
            ObjectData::Armature(_) => self.skeleton(depth + 1, id)?,
            ObjectData::Empty { .. } | ObjectData::Lattice(_) => {}
        }
        if self.carriers.contains_key(&id) {
            self.carrier(depth + 1, id)?;
        }
        for child in self.children.get(&Some(id)).cloned().unwrap_or_default() {
            let child_path = self.paths.get(&child).cloned().unwrap_or_default();
            let child_prim = child_path.rsplit('/').next().unwrap_or_default().to_string();
            self.line(depth + 1, "");
            self.object(depth + 1, child, &child_prim, &format!("{path}/{child_prim}"))?;
        }
        self.line(depth, "}");
        Ok(())
    }

    /// Gives `ids` (siblings under `parent`) and their descendants their prim paths, and the
    /// exported armatures and blend-shape carriers their Skeleton prims.
    fn assign(&mut self, ids: &[Id], parent: &str, taken: &mut BTreeSet<String>) -> Result<(), SceneError> {
        for id in ids {
            let prim = unique(identifier(&self.scene.object(*id)?.name), taken);
            let path = format!("{parent}/{prim}");
            self.paths.insert(*id, path.clone());
            // Names inside the object: its data prim, its skeleton, then its children.
            let mut inside = BTreeSet::from([prim.clone()]);
            // An armature's Skeleton is its data prim, named as the object (as meshes are).
            if matches!(self.scene.object(*id)?.data, ObjectData::Armature(_)) {
                self.skeletons.insert(*id, format!("{path}/{prim}"));
            }
            let children = self.children.get(&Some(*id)).cloned().unwrap_or_default();
            // Reserve the carrier's name before children take theirs.
            let carrier = unique("Skel".to_string(), &mut inside);
            self.assign(&children, &path, &mut inside)?;
            self.carriers.insert(*id, format!("{path}/{carrier}"));
        }
        Ok(())
    }

    /// Drops carriers for meshes that need none (no shape keys, or an armature binds them);
    /// run once every armature has its skeleton.
    fn settle_carriers(&mut self) {
        let ids: Vec<Id> = self.carriers.keys().copied().collect();
        for id in ids {
            let needs = self.skin(id).is_some_and(|skin| skin.rig.is_none() && !skin.mesh.shape_keys.is_empty());
            if !needs {
                self.carriers.remove(&id);
            }
        }
    }

    fn mesh(&mut self, depth: usize, id: Id, prim: &str) -> Result<(), SceneError> {
        let object = self.scene.object(id)?;
        let skin = self.skin(id);
        let Some(mesh) = skin.as_ref().map(|skin| skin.mesh.clone()).or_else(|| object.evaluated_mesh(self.scene)).or_else(|| object.mesh().cloned())
        else {
            return Ok(());
        };
        let material = object.material.and_then(|material| self.materials.get(&material).cloned());
        let counts: Vec<String> = mesh.faces.iter().map(|face| face.len().to_string()).collect();
        let indices: Vec<String> = mesh.faces.iter().flatten().map(u32::to_string).collect();
        let points: Vec<String> = mesh.positions.iter().map(|point| vec3(*point)).collect();
        let normals: Vec<String> = mesh.corner_normals().into_iter().flatten().map(vec3).collect();
        let bounds = mesh.bounds(Mat4::IDENTITY);
        self.line(depth, "");
        self.line(depth, &format!("def Mesh {} (", quoted(prim)));
        let bound = skin.is_some() && (skin.as_ref().is_some_and(|skin| skin.rig.is_some()) || self.carriers.contains_key(&id));
        let schemas: Vec<&str> = [(material.is_some(), "\"MaterialBindingAPI\""), (bound, "\"SkelBindingAPI\"")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, name)| *name)
            .collect();
        if !schemas.is_empty() {
            self.line(depth + 1, &format!("prepend apiSchemas = [{}]", schemas.join(", ")));
        }
        self.line(depth, ")");
        self.line(depth, "{");
        if !bounds.is_empty() {
            self.line(depth + 1, &format!("float3[] extent = [{}, {}]", vec3(bounds.min), vec3(bounds.max)));
        }
        self.line(depth + 1, &format!("int[] faceVertexCounts = [{}]", counts.join(", ")));
        self.line(depth + 1, &format!("int[] faceVertexIndices = [{}]", indices.join(", ")));
        if let Some(material) = material {
            self.line(depth + 1, &format!("rel material:binding = <{material}>"));
        }
        self.line(depth + 1, &format!("normal3f[] normals = [{}] (", normals.join(", ")));
        self.line(depth + 2, "interpolation = \"faceVarying\"");
        self.line(depth + 1, ")");
        self.line(depth + 1, &format!("point3f[] points = [{}]", points.join(", ")));
        if mesh.has_uvs() && mesh.uvs_aligned() {
            let uvs: Vec<String> = mesh.uvs.iter().flatten().map(|uv| tuple(&uv.to_array())).collect();
            self.line(depth + 1, &format!("texCoord2f[] primvars:st = [{}] (", uvs.join(", ")));
            self.line(depth + 2, "interpolation = \"faceVarying\"");
            self.line(depth + 1, ")");
        }
        if let (Some(skin), true) = (&skin, bound) {
            self.binding(depth + 1, id, skin, &mesh)?;
        }
        self.line(depth + 1, "uniform token subdivisionScheme = \"none\"");
        if skin.is_some() && bound {
            let mesh_path = format!("{}/{}", self.paths.get(&id).cloned().unwrap_or_default(), prim);
            self.blend_shapes(depth + 1, &mesh, &mesh_path);
        }
        self.line(depth, "}");
        Ok(())
    }

    /// The mesh written unbaked with its skinning and shape keys: when its only enabled
    /// modifiers are Armature modifiers (deforming by an exported armature) and it has either.
    fn skin(&self, id: Id) -> Option<Skin> {
        let object = self.scene.object(id).ok()?;
        let mesh = object.mesh()?;
        let enabled: Vec<&kition_scene_model::Modifier> = object.modifiers.iter().filter(|modifier| modifier.enabled).collect();
        let mut rigs = Vec::new();
        for modifier in &enabled {
            match modifier.kind {
                kition_scene_model::ModifierKind::Armature { object: Some(rig), .. } => rigs.push(rig),
                _ => return None,
            }
        }
        let rig = rigs.into_iter().find(|rig| self.skeletons.contains_key(rig));
        (rig.is_some() || !mesh.shape_keys.is_empty()).then(|| Skin { mesh: mesh.clone(), rig })
    }

    /// A bound mesh's skeleton, geometry bind transform, and joint indices and weights.
    fn binding(&mut self, depth: usize, id: Id, skin: &Skin, mesh: &Mesh) -> Result<(), SceneError> {
        let count = mesh.positions.len();
        let (skeleton, bind, indices, weights, size) = match skin.rig {
            Some(rig) => {
                let armature = kition_scene_model::armature::armature_of(self.scene, rig).ok_or_else(|| SceneError::not_found("an armature"))?;
                let joints: Vec<&str> = armature.bones.iter().map(|bone| bone.name.as_str()).collect();
                // Each vertex's strongest deforming bones, normalised.
                let mut influences: Vec<Vec<(usize, f32)>> = vec![Vec::new(); count];
                for group in &mesh.groups {
                    let Some(joint) = joints.iter().position(|name| *name == group.name) else { continue };
                    if !armature.bones.get(joint).is_some_and(|bone| bone.deform) {
                        continue;
                    }
                    for (vertex, weight) in group.weights.iter().enumerate() {
                        if *weight > 0.0
                            && let Some(list) = influences.get_mut(vertex)
                        {
                            list.push((joint, *weight));
                        }
                    }
                }
                let size = influences.iter().map(Vec::len).max().unwrap_or(1).clamp(1, crate::skel::MAX_INFLUENCES);
                let mut indices = Vec::with_capacity(count * size);
                let mut weights = Vec::with_capacity(count * size);
                for list in &mut influences {
                    list.sort_by(|a, b| b.1.total_cmp(&a.1));
                    list.truncate(size);
                    let total: f32 = list.iter().map(|(_, weight)| weight).sum();
                    for slot in 0..size {
                        let (joint, weight) = list.get(slot).copied().unwrap_or((0, 0.0));
                        indices.push(joint.to_string());
                        weights.push(number(if total > 0.0 { weight / total } else { 0.0 }));
                    }
                }
                let rig_world = self.scene.world_matrix(rig)?;
                let bind = rig_world.inverse() * self.scene.world_matrix(id)?;
                (self.skeletons.get(&rig).cloned().unwrap_or_default(), bind, indices, weights, size)
            }
            // Shape keys alone: every point on the carrier's one joint.
            None => {
                (self.carriers.get(&id).cloned().unwrap_or_default(), Mat4::IDENTITY, vec!["0".to_string(); count], vec!["1".to_string(); count], 1)
            }
        };
        self.line(depth, &format!("matrix4d primvars:skel:geomBindTransform = {}", matrix4d(bind)));
        self.line(depth, &format!("int[] primvars:skel:jointIndices = [{}] (", indices.join(", ")));
        self.line(depth + 1, &format!("elementSize = {size}"));
        self.line(depth + 1, "interpolation = \"vertex\"");
        self.line(depth, ")");
        self.line(depth, &format!("float[] primvars:skel:jointWeights = [{}] (", weights.join(", ")));
        self.line(depth + 1, &format!("elementSize = {size}"));
        self.line(depth + 1, "interpolation = \"vertex\"");
        self.line(depth, ")");
        if !mesh.shape_keys.is_empty() {
            let names: Vec<String> = mesh.shape_keys.iter().map(|key| quoted(&identifier(&key.name))).collect();
            self.line(depth, &format!("uniform token[] skel:blendShapes = [{}]", names.join(", ")));
        }
        self.line(depth, &format!("rel skel:skeleton = <{skeleton}>"));
        Ok(())
    }

    /// A mesh's shape keys as BlendShape children (the targets its `skel:blendShapeTargets`
    /// names).
    fn blend_shapes(&mut self, depth: usize, mesh: &Mesh, mesh_path: &str) {
        if mesh.shape_keys.is_empty() {
            return;
        }
        let prims: Vec<String> = mesh.shape_keys.iter().map(|key| identifier(&key.name)).collect();
        let targets: Vec<String> = prims.iter().map(|prim| format!("<{mesh_path}/{prim}>")).collect();
        self.line(depth, &format!("rel skel:blendShapeTargets = [{}]", targets.join(", ")));
        for (key, prim) in mesh.shape_keys.iter().zip(&prims) {
            let moved: Vec<usize> = key.offsets.iter().enumerate().filter(|(_, offset)| offset.length() > 0.0).map(|(index, _)| index).collect();
            let offsets: Vec<String> = moved.iter().filter_map(|index| key.offsets.get(*index)).map(|offset| vec3(*offset)).collect();
            let indices: Vec<String> = moved.iter().map(usize::to_string).collect();
            self.line(depth, "");
            self.line(depth, &format!("def BlendShape {}", quoted(prim)));
            self.line(depth, "{");
            self.line(depth + 1, &format!("uniform vector3f[] offsets = [{}]", offsets.join(", ")));
            self.line(depth + 1, &format!("uniform int[] pointIndices = [{}]", indices.join(", ")));
            self.line(depth, "}");
        }
    }

    /// The bone names of an armature as joint paths ("Root/Arm/Hand").
    fn joint_paths(armature: &kition_scene_model::armature::Armature) -> Vec<String> {
        let names: Vec<String> = armature.bones.iter().map(|bone| identifier(&bone.name)).collect();
        armature
            .bones
            .iter()
            .enumerate()
            .map(|(index, bone)| {
                let mut path = vec![names.get(index).cloned().unwrap_or_default()];
                let mut parent = bone.parent.clone();
                let mut guard = 0;
                while let Some(name) = parent {
                    let Some(at) = armature.bones.iter().position(|other| other.name == name) else { break };
                    path.push(names.get(at).cloned().unwrap_or_default());
                    parent = armature.bones.get(at).and_then(|bone| bone.parent.clone());
                    guard += 1;
                    if guard > armature.bones.len() {
                        break;
                    }
                }
                path.reverse();
                path.join("/")
            })
            .collect()
    }

    /// Each bone's transform relative to its parent (or the armature for a root) from pose
    /// matrices in armature space.
    fn locals(armature: &kition_scene_model::armature::Armature, poses: &BTreeMap<String, Mat4>) -> Vec<Mat4> {
        armature
            .bones
            .iter()
            .map(|bone| {
                let pose = poses.get(&bone.name).copied().unwrap_or_else(|| bone.rest_matrix());
                match bone
                    .parent
                    .as_ref()
                    .and_then(|parent| poses.get(parent).copied().or_else(|| armature.bone(parent).map(|parent| parent.rest_matrix())))
                {
                    Some(parent) => parent.inverse() * pose,
                    None => pose,
                }
            })
            .collect()
    }

    /// An armature's Skeleton prim and its SkelAnimation.
    fn skeleton(&mut self, depth: usize, id: Id) -> Result<(), SceneError> {
        let Some(path) = self.skeletons.get(&id).cloned() else { return Ok(()) };
        let armature = kition_scene_model::armature::armature_of(self.scene, id).ok_or_else(|| SceneError::not_found("an armature"))?.clone();
        let prim = path.rsplit('/').next().unwrap_or("Armature").to_string();
        let joints: Vec<String> = Self::joint_paths(&armature).iter().map(|joint| quoted(joint)).collect();
        let rests: Vec<Mat4> = armature.bones.iter().map(|bone| bone.rest_matrix()).collect();
        let rest_poses: BTreeMap<String, Mat4> = armature.bones.iter().zip(&rests).map(|(bone, rest)| (bone.name.clone(), *rest)).collect();
        let binds: Vec<String> = rests.iter().map(|rest| matrix4d(*rest)).collect();
        let locals: Vec<String> = Self::locals(&armature, &rest_poses).into_iter().map(matrix4d).collect();
        let lengths: Vec<String> = armature.bones.iter().map(|bone| number(bone.length())).collect();
        self.line(depth, "");
        self.line(depth, &format!("def Skeleton {} (", quoted(&prim)));
        self.line(depth + 1, "prepend apiSchemas = [\"SkelBindingAPI\"]");
        self.line(depth, ")");
        self.line(depth, "{");
        self.line(depth + 1, &format!("uniform matrix4d[] bindTransforms = [{}]", binds.join(", ")));
        self.line(depth + 1, &format!("uniform token[] joints = [{}]", joints.join(", ")));
        self.line(depth + 1, &format!("float[] {} = [{}] (", crate::skel::BONE_LENGTHS, lengths.join(", ")));
        self.line(depth + 2, "interpolation = \"uniform\"");
        self.line(depth + 1, ")");
        self.line(depth + 1, &format!("uniform matrix4d[] restTransforms = [{}]", locals.join(", ")));
        self.line(depth + 1, &format!("rel skel:animationSource = <{path}/Animation>"));
        self.animation(depth + 1, Some((id, &joints)), &self.bound_shapes(Some(id)));
        self.line(depth, "}");
        Ok(())
    }

    /// A carrier skeleton for a shape-keyed mesh no armature deforms: one joint, at rest, whose
    /// animation holds the mesh's blend shape weights.
    fn carrier(&mut self, depth: usize, id: Id) -> Result<(), SceneError> {
        let Some(path) = self.carriers.get(&id).cloned() else { return Ok(()) };
        let prim = path.rsplit('/').next().unwrap_or("Skel").to_string();
        self.line(depth, "");
        self.line(depth, &format!("def Skeleton {} (", quoted(&prim)));
        self.line(depth + 1, "prepend apiSchemas = [\"SkelBindingAPI\"]");
        self.line(depth, ")");
        self.line(depth, "{");
        self.line(depth + 1, &format!("uniform matrix4d[] bindTransforms = [{}]", matrix4d(Mat4::IDENTITY)));
        self.line(depth + 1, "uniform token[] joints = [\"joint1\"]");
        self.line(depth + 1, &format!("uniform matrix4d[] restTransforms = [{}]", matrix4d(Mat4::IDENTITY)));
        self.line(depth + 1, &format!("rel skel:animationSource = <{path}/Animation>"));
        self.animation(depth + 1, None, &[(id, self.shape_names(id))]);
        self.line(depth, "}");
        Ok(())
    }

    fn shape_names(&self, id: Id) -> Vec<String> {
        self.scene
            .object(id)
            .ok()
            .and_then(|object| object.mesh())
            .map(|mesh| mesh.shape_keys.iter().map(|key| key.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The shape-keyed meshes an armature (or, with `None`, nothing) deforms, with their keys.
    fn bound_shapes(&self, rig: Option<Id>) -> Vec<(Id, Vec<String>)> {
        self.paths
            .keys()
            .copied()
            .filter(|id| self.skin(*id).is_some_and(|skin| skin.rig == rig && rig.is_some()))
            .map(|id| (id, self.shape_names(id)))
            .filter(|(_, names)| !names.is_empty())
            .collect()
    }

    /// A SkelAnimation: the joints' transforms (when `joints` names an armature) and the blend
    /// shape weights of `shapes`, now and, when anything is keyed, at every frame.
    fn animation(&mut self, depth: usize, joints: Option<(Id, &[String])>, shapes: &[(Id, Vec<String>)]) {
        let shape_names: Vec<String> = shapes.iter().flat_map(|(_, names)| names.iter().map(|name| quoted(&identifier(name)))).collect();
        let joint_values = |scene: &Scene| -> Option<JointValues> {
            let (rig, _) = joints?;
            let posed = kition_scene_model::armature::armature_of(scene, rig)?;
            let poses = kition_scene_model::constraints::bone_matrices(scene, rig, 0);
            let locals = Self::locals(posed, &poses);
            let mut translations = Vec::new();
            let mut rotations = Vec::new();
            let mut scales = Vec::new();
            for local in locals {
                let (scale, rotation, translation) = local.to_scale_rotation_translation();
                translations.push(vec3(translation));
                rotations.push(format!("({}, {}, {}, {})", number(rotation.w), number(rotation.x), number(rotation.y), number(rotation.z)));
                scales.push(vec3(scale));
            }
            Some((translations, rotations, scales))
        };
        let weight_values = |scene: &Scene| -> Vec<String> {
            shapes
                .iter()
                .flat_map(|(id, names)| {
                    let mesh = scene.object(*id).ok().and_then(|object| object.mesh());
                    names
                        .iter()
                        .map(move |name| {
                            number(mesh.and_then(|mesh| mesh.shape_keys.iter().find(|key| key.name == *name)).map_or(0.0, |key| key.value))
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        self.line(depth, "");
        self.line(depth, "def SkelAnimation \"Animation\"");
        self.line(depth, "{");
        if !shape_names.is_empty() {
            self.line(depth + 1, &format!("uniform token[] blendShapes = [{}]", shape_names.join(", ")));
            self.line(depth + 1, &format!("float[] blendShapeWeights = [{}]", weight_values(self.scene).join(", ")));
        }
        if let Some((_, names)) = joints {
            self.line(depth + 1, &format!("uniform token[] joints = [{}]", names.join(", ")));
            if let Some((translations, rotations, scales)) = joint_values(self.scene) {
                self.line(depth + 1, &format!("quatf[] rotations = [{}]", rotations.join(", ")));
                self.line(depth + 1, &format!("half3[] scales = [{}]", scales.join(", ")));
                self.line(depth + 1, &format!("float3[] translations = [{}]", translations.join(", ")));
            }
        }
        if let Some((first, last)) = self.frames {
            let mut stepper = self.scene.clone();
            let mut joint_rows: Vec<(i64, JointValues)> = Vec::new();
            let mut weight_rows: Vec<(i64, Vec<String>)> = Vec::new();
            for frame in first..=last {
                stepper.go_to_frame(frame as f32);
                if let Some(values) = joint_values(&stepper) {
                    joint_rows.push((frame, values));
                }
                if !shape_names.is_empty() {
                    weight_rows.push((frame, weight_values(&stepper)));
                }
            }
            if !weight_rows.is_empty() {
                self.line(depth + 1, "float[] blendShapeWeights.timeSamples = {");
                for (frame, weights) in &weight_rows {
                    self.line(depth + 2, &format!("{frame}: [{}],", weights.join(", ")));
                }
                self.line(depth + 1, "}");
            }
            if !joint_rows.is_empty() {
                for (attribute, pick) in [("quatf[] rotations", 1usize), ("half3[] scales", 2), ("float3[] translations", 0)] {
                    self.line(depth + 1, &format!("{attribute}.timeSamples = {{"));
                    for (frame, (translations, rotations, scales)) in &joint_rows {
                        let values = match pick {
                            0 => translations,
                            1 => rotations,
                            _ => scales,
                        };
                        self.line(depth + 2, &format!("{frame}: [{}],", values.join(", ")));
                    }
                    self.line(depth + 1, "}");
                }
            }
        }
        self.line(depth, "}");
    }

    fn material(&mut self, depth: usize, material: &Material, prim: &str, path: &str) {
        let surface = format!("{path}/PreviewSurface");
        self.line(depth, "");
        self.line(depth, &format!("def Material {}", quoted(prim)));
        self.line(depth, "{");
        if material.name != prim {
            self.line(depth + 1, &format!("custom string {OBJECT_NAME} = {}", quoted(&material.name)));
        }
        self.line(depth + 1, &format!("token outputs:surface.connect = <{surface}.outputs:surface>"));
        self.line(depth + 1, "");
        self.line(depth + 1, "def Shader \"PreviewSurface\"");
        self.line(depth + 1, "{");
        self.line(depth + 2, "uniform token info:id = \"UsdPreviewSurface\"");
        let [red, green, blue, alpha] = material.base_color;
        // Images (in a package): texture shaders whose scale carries the factors that multiply them.
        type Texture = (&'static str, String, &'static str, [f32; 4], Option<[f32; 4]>);
        let mut textures: Vec<Texture> = Vec::new();
        if let Some(file) = self.texture_file(material.base_color_texture.as_ref()) {
            self.line(depth + 2, &format!("color3f inputs:diffuseColor.connect = <{path}/BaseColorTexture.outputs:rgb>"));
            textures.push(("BaseColorTexture", file, "sRGB", [red, green, blue, 1.0], None));
        }
        if let Some(file) = self.texture_file(material.metallic_roughness_texture.as_ref()) {
            self.line(depth + 2, &format!("float inputs:metallic.connect = <{path}/MetallicRoughnessTexture.outputs:b>"));
            self.line(depth + 2, &format!("float inputs:roughness.connect = <{path}/MetallicRoughnessTexture.outputs:g>"));
            textures.push(("MetallicRoughnessTexture", file, "raw", [1.0, material.roughness, material.metallic, 1.0], None));
        }
        if let Some(file) = self.texture_file(material.normal_texture.as_ref()) {
            self.line(depth + 2, &format!("normal3f inputs:normal.connect = <{path}/NormalTexture.outputs:rgb>"));
            let strength = material.normal_strength;
            textures.push(("NormalTexture", file, "raw", [2.0 * strength, 2.0 * strength, 2.0, 1.0], Some([-strength, -strength, -1.0, 0.0])));
        }
        self.line(depth + 2, &format!("color3f inputs:diffuseColor = {}", tuple(&[red, green, blue])));
        self.line(depth + 2, &format!("color3f inputs:emissiveColor = {}", vec3(material.emission * material.emission_strength)));
        self.line(depth + 2, &format!("float inputs:ior = {}", number(material.ior)));
        self.line(depth + 2, &format!("float inputs:metallic = {}", number(material.metallic)));
        self.line(depth + 2, &format!("float inputs:opacity = {}", number(alpha)));
        self.line(depth + 2, &format!("float inputs:roughness = {}", number(material.roughness)));
        self.line(depth + 2, "token outputs:surface");
        self.line(depth + 1, "}");
        if !textures.is_empty() {
            self.line(depth + 1, "");
            self.line(depth + 1, "def Shader \"UvMap\"");
            self.line(depth + 1, "{");
            self.line(depth + 2, "uniform token info:id = \"UsdPrimvarReader_float2\"");
            self.line(depth + 2, "string inputs:varname = \"st\"");
            self.line(depth + 2, "float2 outputs:result");
            self.line(depth + 1, "}");
        }
        for (prim, file, color_space, scale, bias) in &textures {
            self.line(depth + 1, "");
            self.line(depth + 1, &format!("def Shader \"{prim}\""));
            self.line(depth + 1, "{");
            self.line(depth + 2, "uniform token info:id = \"UsdUVTexture\"");
            if let Some(bias) = bias {
                self.line(depth + 2, &format!("float4 inputs:bias = {}", tuple(bias)));
            }
            self.line(depth + 2, &format!("asset inputs:file = @{file}@"));
            self.line(depth + 2, &format!("float4 inputs:scale = {}", tuple(scale)));
            self.line(depth + 2, &format!("token inputs:sourceColorSpace = \"{color_space}\""));
            self.line(depth + 2, &format!("float2 inputs:st.connect = <{path}/UvMap.outputs:result>"));
            self.line(depth + 2, "token inputs:wrapS = \"repeat\"");
            self.line(depth + 2, "token inputs:wrapT = \"repeat\"");
            self.line(depth + 2, "float3 outputs:rgb");
            self.line(depth + 2, "float outputs:r");
            self.line(depth + 2, "float outputs:g");
            self.line(depth + 2, "float outputs:b");
            self.line(depth + 2, "float outputs:a");
            self.line(depth + 1, "}");
        }
        self.line(depth, "}");
    }

    /// The package path of image `hash` (added to the package's files once), when the layer
    /// goes in a package and the scene has the image.
    fn texture_file(&mut self, hash: Option<&String>) -> Option<String> {
        let hash = hash?;
        if !self.package {
            return None;
        }
        if let Some((file, _)) = self.files.iter().find(|(_, known)| known == hash) {
            return Some(file.clone());
        }
        let image = self.scene.image(hash)?;
        let stem = identifier(image.name.rsplit_once('.').map_or(image.name.as_str(), |(stem, _)| stem));
        let mut file = format!("textures/{stem}.{}", image.format.extension());
        let mut counter = 1;
        while self.files.iter().any(|(known, _)| *known == file) {
            file = format!("textures/{stem}_{counter}.{}", image.format.extension());
            counter += 1;
        }
        self.files.push((file.clone(), hash.clone()));
        Some(file)
    }
}

/// The frames from the first key to the last, when anything is keyed (and the range is sane).
fn key_range(scene: &Scene) -> Option<(i64, i64)> {
    let channels = &scene.animation.channels;
    let first = channels.iter().filter_map(|channel| channel.keys.first().map(|key| key.frame)).fold(f32::INFINITY, f32::min).floor() as i64;
    let last = channels.iter().filter_map(|channel| channel.keys.last().map(|key| key.frame)).fold(f32::NEG_INFINITY, f32::max).ceil() as i64;
    (!channels.is_empty() && first <= last && last - first <= 100_000).then_some((first, last))
}

/// Local matrices at every frame of the animation, for objects whose transforms are keyed.
fn sample_animation(scene: &Scene, ids: &[Id], children: &BTreeMap<Option<Id>, Vec<Id>>) -> Result<HashMap<Id, Vec<(i64, Mat4)>>, SceneError> {
    let channels = &scene.animation.channels;
    let mut samples: HashMap<Id, Vec<(i64, Mat4)>> = HashMap::new();
    let animated: Vec<Id> = ids
        .iter()
        .copied()
        .filter(|id| {
            channels.iter().any(|channel| {
                channel.target == *id
                    && matches!(
                        channel.property,
                        kition_scene_model::Property::Location | kition_scene_model::Property::Rotation | kition_scene_model::Property::Scale
                    )
            })
        })
        .collect();
    if animated.is_empty() {
        return Ok(samples);
    }
    let Some((first, last)) = key_range(scene) else { return Ok(samples) };
    let mut stepper = scene.clone();
    for frame in first..=last {
        stepper.go_to_frame(frame as f32);
        for id in &animated {
            let world = stepper.world_matrix(*id)?;
            let object = stepper.object(*id)?;
            let parent = object.parent.filter(|parent| children.get(&Some(*parent)).is_some_and(|list| list.contains(id)));
            let local = match parent {
                Some(parent) => stepper.world_matrix(parent)?.inverse() * world,
                None => world,
            };
            samples.entry(*id).or_default().push((frame, local));
        }
    }
    Ok(samples)
}

/// Writes the scene's visible objects (or `only` and their ancestors) as USDA text. With
/// `package`, materials' images are referenced at package paths, returned with their hashes.
pub fn export(scene: &Scene, only: Option<&[Id]>, package: bool) -> Result<(String, Vec<(String, String)>), SceneError> {
    // The NLA and F-curve modifiers go out as the keys they make on every frame.
    let baked = scene.baked_animation()?;
    let scene = baked.as_ref().unwrap_or(scene);
    let ids = chosen(scene, only, true)?;
    let mut children: BTreeMap<Option<Id>, Vec<Id>> = BTreeMap::new();
    for id in &ids {
        let parent = scene.object(*id)?.parent.filter(|parent| ids.contains(parent));
        children.entry(parent).or_default().push(*id);
    }
    let timeline = &scene.animation.timeline;
    let mut writer = Writer {
        scene,
        out: String::new(),
        materials: HashMap::new(),
        samples: sample_animation(scene, &ids, &children)?,
        children,
        paths: HashMap::new(),
        skeletons: HashMap::new(),
        carriers: HashMap::new(),
        frames: key_range(scene),
        package,
        files: Vec::new(),
        aspect: if scene.render.width > 0 { scene.render.height as f32 / scene.render.width as f32 } else { 9.0 / 16.0 },
    };
    let fps = timeline.fps.max(1.0);
    writer.line(0, "#usda 1.0");
    writer.line(0, "(");
    writer.line(1, "defaultPrim = \"root\"");
    writer.line(1, "doc = \"Kition Scene\"");
    writer.line(1, &format!("endTimeCode = {}", timeline.end));
    writer.line(1, &format!("framesPerSecond = {}", number(fps)));
    writer.line(1, "metersPerUnit = 1");
    writer.line(1, &format!("startTimeCode = {}", timeline.start));
    writer.line(1, &format!("timeCodesPerSecond = {}", number(fps)));
    writer.line(1, "upAxis = \"Z\"");
    writer.line(0, ")");
    writer.line(0, "");
    // The root only groups the scene: readers that know the mark make no object of it.
    writer.line(0, "def Xform \"root\" (");
    writer.line(1, "customData = {");
    writer.line(2, "dictionary Blender = {");
    writer.line(3, "bool generated = 1");
    writer.line(2, "}");
    writer.line(1, "}");
    writer.line(0, ")");
    writer.line(0, "{");
    // Materials first get their paths; they are written after the objects.
    let mut used: Vec<Id> = Vec::new();
    for id in &ids {
        let object = scene.object(*id)?;
        if let (ObjectData::Mesh(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_), Some(material)) =
            (&object.data, object.material)
            && scene.material(material).is_ok()
            && !used.contains(&material)
        {
            used.push(material);
        }
    }
    let mut taken = BTreeSet::from(["_materials".to_string()]);
    let mut material_prims: Vec<(Id, String)> = Vec::new();
    let mut material_names = BTreeSet::new();
    for material in &used {
        let prim = unique(identifier(&scene.material(*material)?.name), &mut material_names);
        writer.materials.insert(*material, format!("/root/_materials/{prim}"));
        material_prims.push((*material, prim));
    }
    writer.assign(&writer.children.get(&None).cloned().unwrap_or_default(), "/root", &mut taken)?;
    writer.settle_carriers();
    for id in writer.children.get(&None).cloned().unwrap_or_default() {
        let prim = writer.paths.get(&id).and_then(|path| path.rsplit('/').next()).unwrap_or_default().to_string();
        writer.line(1, "");
        writer.object(1, id, &prim, &format!("/root/{prim}"))?;
    }
    if !material_prims.is_empty() {
        writer.line(1, "");
        writer.line(1, "def Scope \"_materials\"");
        writer.line(1, "{");
        for (material, prim) in &material_prims {
            let material = scene.material(*material)?.clone();
            writer.material(2, &material, prim, &format!("/root/_materials/{prim}"));
        }
        writer.line(1, "}");
    }
    writer.line(0, "}");
    Ok((writer.out, writer.files))
}
