//! Kition scenes as binary FBX 7400, laid out as the common exporters lay it out: a Y-up,
//! centimetre axis system (objects at the root carry the turn and the hundredfold scale), Phong
//! materials with their base colour image embedded, skins as clusters with a bind pose, shape
//! keys as blend shapes, and animation baked to one linear key per frame.

use std::collections::{BTreeMap, HashMap};

use kition_scene_exchange::chosen;
use kition_scene_model::glam::{DMat4, DVec3, Mat4, Vec3};
use kition_scene_model::{Id, LightKind, Mesh, ObjectData, Scene, SceneError};

use crate::import::TICKS_PER_SECOND;
use crate::node::{Node, Prop};

/// File axes and units: Y up, Z front, X right, centimetres (as the scene is read back).
fn to_scene() -> DMat4 {
    DMat4::from_scale(DVec3::splat(0.01)) * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2)
}

fn s(text: &str) -> Prop {
    Prop::String(text.to_string())
}

/// A `Properties70` entry.
fn p(name: &str, kind: &str, label: &str, flags: &str, values: Vec<Prop>) -> Node {
    let mut props = vec![s(name), s(kind), s(label), s(flags)];
    props.extend(values);
    Node::new("P", props)
}

fn p_int(name: &str, value: i32) -> Node {
    p(name, "int", "Integer", "", vec![Prop::I32(value)])
}

fn p_enum(name: &str, value: i32) -> Node {
    p(name, "enum", "", "", vec![Prop::I32(value)])
}

fn p_number(name: &str, value: f64) -> Node {
    p(name, "Number", "", "A", vec![Prop::F64(value)])
}

fn p_double(name: &str, value: f64) -> Node {
    p(name, "double", "Number", "", vec![Prop::F64(value)])
}

fn p_color(name: &str, value: DVec3) -> Node {
    p(name, "Color", "", "A", vec![Prop::F64(value.x), Prop::F64(value.y), Prop::F64(value.z)])
}

fn p_vector(name: &str, value: DVec3) -> Node {
    p(name, name, "", "A", vec![Prop::F64(value.x), Prop::F64(value.y), Prop::F64(value.z)])
}

fn props70(entries: Vec<Node>) -> Node {
    Node::new("Properties70", Vec::new()).with(entries)
}

fn class_name(name: &str, class: &str) -> Prop {
    Prop::String(format!("{name}\u{0}\u{1}{class}"))
}

/// Location, XYZ Euler rotation in degrees, and scale of a file-space local matrix.
fn trs(matrix: DMat4) -> (DVec3, DVec3, DVec3) {
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    let euler = kition_scene_model::scene::matrix_euler(kition_scene_model::glam::DMat3::from_quat(rotation).as_mat3()).as_dvec3();
    (translation, DVec3::new(euler.x.to_degrees(), euler.y.to_degrees(), euler.z.to_degrees()), scale)
}

/// The right factor from an object's frame to its FBX node frame (cameras look down +X, lights
/// down -Y).
fn turn(data: &ObjectData) -> DMat4 {
    match data {
        ObjectData::Camera(_) => DMat4::from_rotation_y(std::f64::consts::FRAC_PI_2),
        ObjectData::Light(_) => DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2),
        _ => DMat4::IDENTITY,
    }
}

struct Ids {
    next: i64,
}

impl Ids {
    fn take(&mut self) -> i64 {
        self.next += 1;
        self.next
    }
}

struct Writer {
    objects: Vec<Node>,
    links: Vec<Node>,
    ids: Ids,
    counts: BTreeMap<&'static str, i32>,
}

impl Writer {
    fn object(&mut self, kind: &'static str, node: Node) {
        *self.counts.entry(kind).or_default() += 1;
        self.objects.push(node);
    }

    fn link(&mut self, child: i64, parent: i64) {
        self.links.push(Node::new("C", vec![s("OO"), Prop::I64(child), Prop::I64(parent)]));
    }

    fn link_property(&mut self, child: i64, parent: i64, property: &str) {
        self.links.push(Node::new("C", vec![s("OP"), Prop::I64(child), Prop::I64(parent), s(property)]));
    }
}

/// The mesh written for an object: the mesh itself when its only modifiers are Armature ones
/// (the file deforms it with a skin), else the evaluated mesh.
fn written_mesh(scene: &Scene, id: Id) -> Option<(Mesh, Option<Id>)> {
    let object = scene.object(id).ok()?;
    // Curves and texts go out as the meshes they make.
    if matches!(object.data, ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_)) {
        return object.evaluated_mesh(scene).map(|mesh| (mesh, None));
    }
    let mesh = object.mesh()?;
    let rigs: Vec<Option<Id>> = object
        .modifiers
        .iter()
        .filter(|modifier| modifier.enabled)
        .filter_map(|modifier| match modifier.kind {
            kition_scene_model::ModifierKind::Armature { object, .. } => Some(object),
            _ => None,
        })
        .collect();
    let only_armatures = !rigs.is_empty() && object.modifiers.iter().filter(|modifier| modifier.enabled).count() == rigs.len();
    if only_armatures {
        let rig = rigs.first().copied().flatten().filter(|rig| kition_scene_model::armature::armature_of(scene, *rig).is_some());
        return Some((mesh.clone(), rig));
    }
    // Without modifiers the mesh goes as it is, its shape keys with it.
    if !object.modifiers.iter().any(|modifier| modifier.enabled) {
        return Some((mesh.clone(), None));
    }
    Some((object.evaluated_mesh(scene)?, None))
}

fn geometry_node(id: i64, name: &str, mesh: &Mesh) -> Node {
    let vertices: Vec<f64> = mesh.positions.iter().flat_map(|point| point.as_dvec3().to_array()).collect();
    let mut index = Vec::new();
    let mut normals: Vec<f64> = Vec::new();
    let corner_normals = mesh.corner_normals();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let flat = mesh.face_normal(face);
        for (corner, vertex) in face.iter().enumerate() {
            let value = i32::try_from(*vertex).unwrap_or(i32::MAX);
            index.push(if corner + 1 == face.len() { -value - 1 } else { value });
            let normal = corner_normals.get(face_index).and_then(|list| list.get(corner)).copied().unwrap_or(flat);
            normals.extend(normal.as_dvec3().to_array());
        }
    }
    let mut children = vec![
        Node::new("GeometryVersion", vec![Prop::I32(124)]),
        Node::new("Vertices", vec![Prop::F64Array(vertices)]),
        Node::new("PolygonVertexIndex", vec![Prop::I32Array(index)]),
        Node::new("LayerElementNormal", vec![Prop::I32(0)]).with(vec![
            Node::new("Version", vec![Prop::I32(101)]),
            Node::new("Name", vec![s("")]),
            Node::new("MappingInformationType", vec![s("ByPolygonVertex")]),
            Node::new("ReferenceInformationType", vec![s("Direct")]),
            Node::new("Normals", vec![Prop::F64Array(normals)]),
        ]),
    ];
    let mut layer = vec![
        Node::new("LayerElement", Vec::new())
            .with(vec![Node::new("Type", vec![s("LayerElementNormal")]), Node::new("TypedIndex", vec![Prop::I32(0)])]),
    ];
    if mesh.has_uvs() && mesh.uvs_aligned() {
        let uvs: Vec<f64> = mesh.uvs.iter().flatten().flat_map(|uv| [f64::from(uv.x), f64::from(uv.y)]).collect();
        let count = uvs.len() / 2;
        children.push(Node::new("LayerElementUV", vec![Prop::I32(0)]).with(vec![
            Node::new("Version", vec![Prop::I32(101)]),
            Node::new("Name", vec![s("UVMap")]),
            Node::new("MappingInformationType", vec![s("ByPolygonVertex")]),
            Node::new("ReferenceInformationType", vec![s("IndexToDirect")]),
            Node::new("UV", vec![Prop::F64Array(uvs)]),
            Node::new("UVIndex", vec![Prop::I32Array((0..count).map(|index| i32::try_from(index).unwrap_or(i32::MAX)).collect())]),
        ]));
        layer.push(
            Node::new("LayerElement", Vec::new())
                .with(vec![Node::new("Type", vec![s("LayerElementUV")]), Node::new("TypedIndex", vec![Prop::I32(0)])]),
        );
    }
    children.push(Node::new("LayerElementMaterial", vec![Prop::I32(0)]).with(vec![
        Node::new("Version", vec![Prop::I32(101)]),
        Node::new("Name", vec![s("")]),
        Node::new("MappingInformationType", vec![s("AllSame")]),
        Node::new("ReferenceInformationType", vec![s("IndexToDirect")]),
        Node::new("Materials", vec![Prop::I32Array(vec![0])]),
    ]));
    layer.push(
        Node::new("LayerElement", Vec::new())
            .with(vec![Node::new("Type", vec![s("LayerElementMaterial")]), Node::new("TypedIndex", vec![Prop::I32(0)])]),
    );
    children.push(Node::new("Layer", vec![Prop::I32(0)]).with(std::iter::once(Node::new("Version", vec![Prop::I32(100)])).chain(layer).collect()));
    Node::new("Geometry", vec![Prop::I64(id), class_name(name, "Geometry"), s("Mesh")]).with(children)
}

fn model_node(id: i64, name: &str, class: &str, local: DMat4) -> Node {
    let (translation, rotation, scale) = trs(local);
    Node::new("Model", vec![Prop::I64(id), class_name(name, "Model"), s(class)]).with(vec![
        Node::new("Version", vec![Prop::I32(232)]),
        props70(vec![
            p_vector("Lcl Translation", translation),
            p_vector("Lcl Rotation", rotation),
            p_vector("Lcl Scaling", scale),
            p_int("DefaultAttributeIndex", 0),
            p_enum("InheritType", 1),
        ]),
        Node::new("MultiLayer", vec![Prop::I32(0)]),
        Node::new("MultiTake", vec![Prop::I32(0)]),
        Node::new("Shading", vec![Prop::Bool(true)]),
        Node::new("Culling", vec![s("CullingOff")]),
    ])
}

fn matrix_prop(matrix: DMat4) -> Prop {
    Prop::F64Array(matrix.to_cols_array().to_vec())
}

/// Writes the scene's visible objects (or `only` and their ancestors) as binary FBX 7400.
pub fn export(scene: &Scene, only: Option<&[Id]>) -> Result<Vec<u8>, SceneError> {
    // The NLA and F-curve modifiers go out as the keys they make on every frame.
    let baked = scene.baked_animation()?;
    let scene = baked.as_ref().unwrap_or(scene);
    let ids = chosen(scene, only, true)?;
    let mut writer = Writer { objects: Vec::new(), links: Vec::new(), ids: Ids { next: 1_000_000 }, counts: BTreeMap::new() };
    let from_scene = to_scene().inverse();
    // File-space world matrices (node frames) by object.
    let mut fbx_world: HashMap<Id, DMat4> = HashMap::new();
    let mut model_ids: HashMap<Id, i64> = HashMap::new();
    let mut material_ids: HashMap<Id, i64> = HashMap::new();
    let mut bone_ids: HashMap<(Id, String), (i64, DMat4)> = HashMap::new();
    for id in &ids {
        let object = scene.object(*id)?;
        let world = scene.world_matrix(*id)?.as_dmat4() * turn(&object.data);
        let file_world = from_scene * world;
        fbx_world.insert(*id, file_world);
        // Skinned meshes sit at the root, as the common exporters write them (their armature
        // moves them through the skin).
        let skinned = matches!(written_mesh(scene, *id), Some((_, Some(_))));
        let parent = object.parent.filter(|parent| ids.contains(parent) && !skinned);
        let parent_world = parent.and_then(|parent| fbx_world.get(&parent).copied()).unwrap_or(DMat4::IDENTITY);
        let local = parent_world.inverse() * file_world;
        let model = writer.ids.take();
        model_ids.insert(*id, model);
        let class = match &object.data {
            ObjectData::Mesh(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_) => "Mesh",
            ObjectData::Camera(_) => "Camera",
            ObjectData::Light(_) => "Light",
            ObjectData::Empty { .. } | ObjectData::Armature(_) | ObjectData::Lattice(_) => "Null",
        };
        writer.object("Model", model_node(model, &object.name, class, local));
        writer.link(model, parent.and_then(|parent| model_ids.get(&parent).copied()).unwrap_or(0));
        match &object.data {
            ObjectData::Mesh(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_) => {
                let Some((mesh, _)) = written_mesh(scene, *id) else { continue };
                let geometry = writer.ids.take();
                writer.object("Geometry", geometry_node(geometry, &object.name, &mesh));
                writer.link(geometry, model);
                write_shape_keys(&mut writer, geometry, &mesh);
                let material = object.material.and_then(|material| scene.material(material).ok());
                if let Some(material) = material {
                    let material_id = match material_ids.get(&material.id) {
                        Some(existing) => *existing,
                        None => {
                            let made = write_material(&mut writer, scene, material);
                            material_ids.insert(material.id, made);
                            made
                        }
                    };
                    writer.link(material_id, model);
                }
            }
            ObjectData::Camera(camera) => {
                let attribute = writer.ids.take();
                let unit = 100.0;
                let aspect = f64::from(scene.render.width.max(1)) / f64::from(scene.render.height.max(1));
                let (half_width, half_height) = camera.frame_half_size(aspect as f32);
                // FBX importers do not consistently retain fit policy. Bake this frame
                // into the Auto-sized aperture and scale observed in the reference app.
                let span = 2.0 * half_width.max(half_height);
                let film_width = if camera.orthographic { f64::from(camera.sensor) } else { f64::from(span * camera.lens) } / 25.4;
                writer.object(
                    "NodeAttribute",
                    Node::new("NodeAttribute", vec![Prop::I64(attribute), class_name(&object.name, "NodeAttribute"), s("Camera")]).with(vec![
                        Node::new("TypeFlags", vec![s("Camera")]),
                        Node::new("GeometryVersion", vec![Prop::I32(124)]),
                        props70(vec![
                            p_enum("CameraProjectionType", i32::from(camera.orthographic)),
                            p_double("AspectWidth", f64::from(scene.render.width)),
                            p_double("AspectHeight", f64::from(scene.render.height)),
                            p_double("FilmWidth", film_width),
                            p_double("FilmHeight", film_width / aspect),
                            p_enum("ApertureMode", 3),
                            p_number("FocalLength", f64::from(camera.lens)),
                            p_double("OrthoZoom", f64::from(if camera.orthographic { span } else { camera.ortho_scale })),
                            p_double("NearPlane", f64::from(camera.clip_start) * unit),
                            p_double("FarPlane", f64::from(camera.clip_end) * unit),
                        ]),
                    ]),
                );
                writer.link(attribute, model);
            }
            ObjectData::Light(light) => {
                let attribute = writer.ids.take();
                let kind = match light.kind {
                    LightKind::Point => 0,
                    LightKind::Sun => 1,
                    LightKind::Spot => 2,
                    LightKind::Area => 3,
                };
                let mut entries = vec![
                    p_enum("LightType", kind),
                    p("CastLight", "bool", "", "", vec![Prop::I32(1)]),
                    p_color("Color", light.color.as_dvec3()),
                    p_number("Intensity", f64::from(light.power) * 100.0),
                    p("CastShadows", "bool", "", "", vec![Prop::I32(1)]),
                ];
                if light.kind == LightKind::Spot {
                    let angle = f64::from(light.spot_size).to_degrees();
                    entries.push(p_double("OuterAngle", angle));
                    entries.push(p_double("InnerAngle", angle * 0.85));
                }
                writer.object(
                    "NodeAttribute",
                    Node::new("NodeAttribute", vec![Prop::I64(attribute), class_name(&object.name, "NodeAttribute"), s("Light")])
                        .with(vec![Node::new("GeometryVersion", vec![Prop::I32(124)]), props70(entries)]),
                );
                writer.link(attribute, model);
            }
            ObjectData::Empty { .. } | ObjectData::Armature(_) | ObjectData::Lattice(_) => {
                let attribute = writer.ids.take();
                writer.object(
                    "NodeAttribute",
                    Node::new("NodeAttribute", vec![Prop::I64(attribute), class_name(&object.name, "NodeAttribute"), s("Null")])
                        .with(vec![Node::new("TypeFlags", vec![s("Null")])]),
                );
                writer.link(attribute, model);
            }
        }
        if let ObjectData::Armature(armature) = &object.data {
            // Bones: rest matrices relative to their parent bone (or the armature).
            for bone in parents_first(armature) {
                let rest = bone.rest_matrix().as_dmat4();
                let parent_rest =
                    bone.parent.as_deref().and_then(|name| armature.bone(name)).map_or(DMat4::IDENTITY, |parent| parent.rest_matrix().as_dmat4());
                let bone_id = writer.ids.take();
                writer.object("Model", model_node(bone_id, &bone.name, "LimbNode", parent_rest.inverse() * rest));
                let parent_id = bone.parent.as_ref().and_then(|name| bone_ids.get(&(*id, name.clone())).map(|entry| entry.0)).unwrap_or(model);
                writer.link(bone_id, parent_id);
                let attribute = writer.ids.take();
                writer.object(
                    "NodeAttribute",
                    Node::new("NodeAttribute", vec![Prop::I64(attribute), class_name(&bone.name, "NodeAttribute"), s("LimbNode")])
                        .with(vec![Node::new("TypeFlags", vec![s("Skeleton")]), props70(vec![p_double("Size", f64::from(bone.length()) * 100.0)])]),
                );
                writer.link(attribute, bone_id);
                bone_ids.insert((*id, bone.name.clone()), (bone_id, file_world * rest));
                // A leaf bone's tail, as an end node (as the common exporters write it).
                if !armature.bones.iter().any(|other| other.parent.as_deref() == Some(bone.name.as_str())) {
                    let end = writer.ids.take();
                    let name = format!("{}_end", bone.name);
                    writer
                        .object("Model", model_node(end, &name, "LimbNode", DMat4::from_translation(DVec3::new(0.0, f64::from(bone.length()), 0.0))));
                    writer.link(end, bone_id);
                    let attribute = writer.ids.take();
                    writer.object(
                        "NodeAttribute",
                        Node::new("NodeAttribute", vec![Prop::I64(attribute), class_name(&name, "NodeAttribute"), s("LimbNode")])
                            .with(vec![Node::new("TypeFlags", vec![s("Skeleton")])]),
                    );
                    writer.link(attribute, end);
                }
            }
        }
    }
    // Skins.
    for id in &ids {
        let Some((mesh, Some(rig))) = written_mesh(scene, *id) else { continue };
        let (Some(model), Some(mesh_world)) = (model_ids.get(id).copied(), fbx_world.get(id).copied()) else { continue };
        let Some(geometry) = writer.links.iter().find_map(|link| match link.props.as_slice() {
            [Prop::String(kind), Prop::I64(child), Prop::I64(parent)]
                if kind == "OO"
                    && *parent == model
                    && writer.objects.iter().any(|node| node.name == "Geometry" && node.prop(0) == Some(&Prop::I64(*child))) =>
            {
                Some(*child)
            }
            _ => None,
        }) else {
            continue;
        };
        // The armature's world: the model the clusters' links are associated with.
        let rig_world = fbx_world.get(&rig).copied().unwrap_or(DMat4::IDENTITY);
        let skin = writer.ids.take();
        writer.object(
            "Deformer",
            Node::new("Deformer", vec![Prop::I64(skin), class_name("Armature", "Deformer"), s("Skin")])
                .with(vec![Node::new("Version", vec![Prop::I32(101)]), Node::new("Link_DeformAcuracy", vec![Prop::F64(50.0)])]),
        );
        writer.link(skin, geometry);
        let mut pose_nodes = vec![
            Node::new("PoseNode", Vec::new())
                .with(vec![Node::new("Node", vec![Prop::I64(model)]), Node::new("Matrix", vec![matrix_prop(mesh_world)])]),
        ];
        // The armature's node too (readers place the skeleton by it).
        if let (Some(rig_model), Some(rig_world)) = (model_ids.get(&rig).copied(), fbx_world.get(&rig).copied()) {
            pose_nodes.push(
                Node::new("PoseNode", Vec::new())
                    .with(vec![Node::new("Node", vec![Prop::I64(rig_model)]), Node::new("Matrix", vec![matrix_prop(rig_world)])]),
            );
        }
        for group in &mesh.groups {
            let Some((bone_id, link)) = bone_ids.get(&(rig, group.name.clone())).copied() else { continue };
            let (indexes, weights): (Vec<i32>, Vec<f64>) = group
                .weights
                .iter()
                .enumerate()
                .filter(|(_, weight)| **weight > 0.0)
                .map(|(index, weight)| (i32::try_from(index).unwrap_or(i32::MAX), f64::from(*weight)))
                .unzip();
            let cluster = writer.ids.take();
            writer.object(
                "Deformer",
                Node::new("Deformer", vec![Prop::I64(cluster), class_name(&group.name, "SubDeformer"), s("Cluster")]).with(vec![
                    Node::new("Version", vec![Prop::I32(100)]),
                    Node::new("UserData", vec![s(""), s("")]),
                    Node::new("Indexes", vec![Prop::I32Array(indexes)]),
                    Node::new("Weights", vec![Prop::F64Array(weights)]),
                    Node::new("Transform", vec![matrix_prop(link.inverse() * mesh_world)]),
                    Node::new("TransformLink", vec![matrix_prop(link)]),
                    Node::new("TransformAssociateModel", vec![matrix_prop(rig_world)]),
                ]),
            );
            writer.link(cluster, skin);
            writer.link(bone_id, cluster);
            pose_nodes.push(
                Node::new("PoseNode", Vec::new())
                    .with(vec![Node::new("Node", vec![Prop::I64(bone_id)]), Node::new("Matrix", vec![matrix_prop(link)])]),
            );
        }
        let pose = writer.ids.take();
        let count = i32::try_from(pose_nodes.len()).unwrap_or(i32::MAX);
        let mut children = vec![
            Node::new("Type", vec![s("BindPose")]),
            Node::new("Version", vec![Prop::I32(100)]),
            Node::new("NbPoseNodes", vec![Prop::I32(count)]),
        ];
        children.extend(pose_nodes);
        let name = scene.object(*id).map(|object| object.name.clone()).unwrap_or_default();
        writer.object("Pose", Node::new("Pose", vec![Prop::I64(pose), class_name(&name, "Pose"), s("BindPose")]).with(children));
    }
    write_animation(&mut writer, scene, &ids, &model_ids, &bone_ids)?;
    Ok(crate::node::write_binary(&document(scene, writer)))
}

/// An armature's bones, every parent before its children.
fn parents_first(armature: &kition_scene_model::armature::Armature) -> Vec<&kition_scene_model::armature::Bone> {
    let depth = |bone: &kition_scene_model::armature::Bone| {
        let mut depth = 0usize;
        let mut current = bone.parent.as_deref();
        while let Some(name) = current {
            depth += 1;
            if depth > armature.bones.len() {
                break;
            }
            current = armature.bone(name).and_then(|parent| parent.parent.as_deref());
        }
        depth
    };
    let mut bones: Vec<_> = armature.bones.iter().collect();
    bones.sort_by_key(|bone| depth(bone));
    bones
}

fn write_shape_keys(writer: &mut Writer, geometry: i64, mesh: &Mesh) {
    if mesh.shape_keys.is_empty() {
        return;
    }
    let blend = writer.ids.take();
    writer.object(
        "Deformer",
        Node::new("Deformer", vec![Prop::I64(blend), class_name("Shapes", "Deformer"), s("BlendShape")])
            .with(vec![Node::new("Version", vec![Prop::I32(100)])]),
    );
    writer.link(blend, geometry);
    for key in &mesh.shape_keys {
        let (indexes, offsets): (Vec<i32>, Vec<f64>) = key.offsets.iter().enumerate().filter(|(_, offset)| offset.length_squared() > 0.0).fold(
            (Vec::new(), Vec::new()),
            |(mut indexes, mut offsets), (index, offset)| {
                indexes.push(i32::try_from(index).unwrap_or(i32::MAX));
                offsets.extend(offset.as_dvec3().to_array());
                (indexes, offsets)
            },
        );
        let count = indexes.len();
        let channel = writer.ids.take();
        writer.object(
            "Deformer",
            Node::new("Deformer", vec![Prop::I64(channel), class_name(&key.name, "SubDeformer"), s("BlendShapeChannel")]).with(vec![
                Node::new("Version", vec![Prop::I32(100)]),
                Node::new("DeformPercent", vec![Prop::F64(f64::from(key.value) * 100.0)]),
                Node::new("FullWeights", vec![Prop::F64Array(vec![100.0; count])]),
            ]),
        );
        writer.link(channel, blend);
        let shape = writer.ids.take();
        writer.object(
            "Geometry",
            Node::new("Geometry", vec![Prop::I64(shape), class_name(&key.name, "Geometry"), s("Shape")]).with(vec![
                Node::new("Version", vec![Prop::I32(100)]),
                Node::new("Indexes", vec![Prop::I32Array(indexes)]),
                Node::new("Vertices", vec![Prop::F64Array(offsets)]),
                Node::new("Normals", vec![Prop::F64Array(vec![0.0; count * 3])]),
            ]),
        );
        writer.link(shape, channel);
    }
}

fn write_material(writer: &mut Writer, scene: &Scene, material: &kition_scene_model::Material) -> i64 {
    let id = writer.ids.take();
    let base = Vec3::new(material.base_color[0], material.base_color[1], material.base_color[2]).as_dvec3();
    let shininess = (10.0 * (1.0 - f64::from(material.roughness))).powi(2);
    let mut entries = vec![
        p_color("DiffuseColor", base),
        p_number("DiffuseFactor", 1.0),
        p_color("EmissiveColor", material.emission.as_dvec3()),
        p_number("EmissiveFactor", f64::from(material.emission_strength)),
        p_color("SpecularColor", DVec3::splat(0.8)),
        p_number("SpecularFactor", 0.25),
        p_number("Shininess", shininess),
        p_number("ShininessExponent", shininess),
        p_number("ReflectionFactor", f64::from(material.metallic)),
    ];
    let alpha = f64::from(material.base_color[3]);
    if alpha < 0.999 {
        entries.push(p_number("TransparencyFactor", 1.0 - alpha));
        entries.push(p_number("Opacity", alpha));
    }
    writer.object(
        "Material",
        Node::new("Material", vec![Prop::I64(id), class_name(&material.name, "Material"), s("")]).with(vec![
            Node::new("Version", vec![Prop::I32(102)]),
            Node::new("ShadingModel", vec![s("Phong")]),
            Node::new("MultiLayer", vec![Prop::I32(0)]),
            props70(entries),
        ]),
    );
    for (hash, property) in [(&material.base_color_texture, "DiffuseColor"), (&material.normal_texture, "NormalMap")] {
        let Some(image) = hash.as_ref().and_then(|hash| scene.image(hash)) else { continue };
        let file = format!("{}.{}", image.name.trim_end_matches(&format!(".{}", image.format.extension())), image.format.extension());
        let texture = writer.ids.take();
        writer.object(
            "Texture",
            Node::new("Texture", vec![Prop::I64(texture), class_name(&image.name, "Texture"), s("")]).with(vec![
                Node::new("Type", vec![s("TextureVideoClip")]),
                Node::new("Version", vec![Prop::I32(202)]),
                Node::new("TextureName", vec![class_name(&image.name, "Texture")]),
                Node::new("Media", vec![class_name(&image.name, "Video")]),
                Node::new("FileName", vec![s(&file)]),
                Node::new("RelativeFilename", vec![s(&file)]),
                props70(vec![p("UseMaterial", "bool", "", "", vec![Prop::I32(1)])]),
            ]),
        );
        let video = writer.ids.take();
        writer.object(
            "Video",
            Node::new("Video", vec![Prop::I64(video), class_name(&image.name, "Video"), s("Clip")]).with(vec![
                Node::new("Type", vec![s("Clip")]),
                props70(vec![p("Path", "KString", "XRefUrl", "", vec![s(&file)])]),
                Node::new("UseMipMap", vec![Prop::I32(0)]),
                Node::new("Filename", vec![s(&file)]),
                Node::new("RelativeFilename", vec![s(&file)]),
                Node::new("Content", vec![Prop::Raw(image.bytes.to_vec())]),
            ]),
        );
        writer.link_property(texture, id, property);
        writer.link(video, texture);
    }
    id
}

/// Every frame of every animated object (and its bones and shape keys), as linear curves.
fn write_animation(
    writer: &mut Writer,
    scene: &Scene,
    ids: &[Id],
    model_ids: &HashMap<Id, i64>,
    bone_ids: &HashMap<(Id, String), (i64, DMat4)>,
) -> Result<(), SceneError> {
    let channels = &scene.animation.channels;
    // Simulated rigid bodies move over their bake.
    let simulated: BTreeMap<Id, (f32, f32)> = ids.iter().filter_map(|id| Some((*id, scene.simulated_range(*id)?))).collect();
    if channels.is_empty() && simulated.is_empty() {
        return Ok(());
    }
    let first = channels
        .iter()
        .filter_map(|channel| channel.keys.first().map(|key| key.frame))
        .chain(simulated.values().map(|range| range.0))
        .fold(f32::INFINITY, f32::min)
        .floor() as i64;
    let last = channels
        .iter()
        .filter_map(|channel| channel.keys.last().map(|key| key.frame))
        .chain(simulated.values().map(|range| range.1))
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil() as i64;
    if first > last || last - first > 100_000 {
        return Ok(());
    }
    let fps = f64::from(scene.animation.timeline.fps.max(1.0));
    let ticks = |frame: i64| (((frame - 1) as f64) / fps * TICKS_PER_SECOND).round() as i64;
    // What is animated: objects (their transforms), armatures (their bones), shape keys.
    let animated_objects: Vec<Id> =
        ids.iter().copied().filter(|id| simulated.contains_key(id) || channels.iter().any(|channel| channel.target == *id)).collect();
    if animated_objects.is_empty() {
        return Ok(());
    }
    let from_scene = to_scene().inverse();
    // Samples: (model id, T R S per frame) and (channel id, percent per frame).
    let mut transforms: BTreeMap<i64, Vec<(DVec3, DVec3, DVec3)>> = BTreeMap::new();
    let mut percents: BTreeMap<(Id, String), Vec<f64>> = BTreeMap::new();
    let mut stepper = scene.clone();
    for frame in first..=last {
        stepper.go_to_frame(frame as f32);
        for id in &animated_objects {
            let object = stepper.object(*id)?;
            let object_channels: Vec<_> = channels.iter().filter(|channel| channel.target == *id).collect();
            if simulated.contains_key(id)
                || object_channels.iter().any(|channel| {
                    matches!(
                        channel.property,
                        kition_scene_model::Property::Location | kition_scene_model::Property::Rotation | kition_scene_model::Property::Scale
                    )
                })
            {
                let world = from_scene * stepper.world_matrix(*id)?.as_dmat4() * turn(&object.data);
                let parent_world = object.parent.filter(|parent| ids.contains(parent)).map_or(Ok(DMat4::IDENTITY), |parent| {
                    stepper
                        .object(parent)
                        .and_then(|parent_object| Ok(from_scene * stepper.world_matrix(parent)?.as_dmat4() * turn(&parent_object.data)))
                })?;
                if let Some(model) = model_ids.get(id) {
                    transforms.entry(*model).or_default().push(trs(parent_world.inverse() * world));
                }
            }
            if let ObjectData::Armature(armature) = &object.data
                && object_channels.iter().any(|channel| !channel.path.is_empty())
            {
                let poses = kition_scene_model::constraints::bone_matrices(&stepper, *id, 0);
                for bone in &armature.bones {
                    let Some(pose) = poses.get(&bone.name) else { continue };
                    let parent_pose = bone.parent.as_deref().and_then(|name| poses.get(name)).copied().unwrap_or(Mat4::IDENTITY);
                    if let Some((model, _)) = bone_ids.get(&(*id, bone.name.clone())) {
                        transforms.entry(*model).or_default().push(trs((parent_pose.inverse() * *pose).as_dmat4()));
                    }
                }
            }
            if let Some(mesh) = object.mesh() {
                for key in &mesh.shape_keys {
                    if object_channels.iter().any(|channel| channel.property == kition_scene_model::Property::ShapeKey && channel.path == key.name) {
                        percents.entry((*id, key.name.clone())).or_default().push(f64::from(key.value) * 100.0);
                    }
                }
            }
        }
    }
    let stack = writer.ids.take();
    let span = (ticks(first), ticks(last));
    writer.object(
        "AnimationStack",
        Node::new("AnimationStack", vec![Prop::I64(stack), class_name("Scene", "AnimStack"), s("")]).with(vec![props70(vec![
            p("LocalStart", "KTime", "Time", "", vec![Prop::I64(span.0)]),
            p("LocalStop", "KTime", "Time", "", vec![Prop::I64(span.1)]),
            p("ReferenceStart", "KTime", "Time", "", vec![Prop::I64(span.0)]),
            p("ReferenceStop", "KTime", "Time", "", vec![Prop::I64(span.1)]),
        ])]),
    );
    let layer = writer.ids.take();
    writer.object("AnimationLayer", Node::new("AnimationLayer", vec![Prop::I64(layer), class_name("Scene", "AnimLayer"), s("")]));
    writer.link(layer, stack);
    let times: Vec<i64> = (first..=last).map(ticks).collect();
    let curve = |writer: &mut Writer, node: i64, component: &str, values: Vec<f64>| {
        let id = writer.ids.take();
        let count = values.len();
        writer.object(
            "AnimationCurve",
            Node::new("AnimationCurve", vec![Prop::I64(id), class_name("", "AnimCurve"), s("")]).with(vec![
                Node::new("Default", vec![Prop::F64(values.first().copied().unwrap_or(0.0))]),
                Node::new("KeyVer", vec![Prop::I32(4008)]),
                Node::new("KeyTime", vec![Prop::I64Array(times.clone())]),
                Node::new("KeyValueFloat", vec![Prop::F32Array(values.iter().map(|value| *value as f32).collect())]),
                // Linear keys (interpolation flag 0x4), one attribute shared by all.
                Node::new("KeyAttrFlags", vec![Prop::I32Array(vec![0x4 | 0x100 | 0x2000 | 0x4000])]),
                Node::new("KeyAttrDataFloat", vec![Prop::F32Array(vec![0.0, 0.0, 0.0, 0.0])]),
                Node::new("KeyAttrRefCount", vec![Prop::I32Array(vec![i32::try_from(count).unwrap_or(i32::MAX)])]),
            ]),
        );
        writer.link_property(id, node, component);
    };
    for (model, samples) in transforms {
        let mut rotations: Vec<DVec3> = samples.iter().map(|(_, rotation, _)| *rotation).collect();
        // Continuous angles (no 360° jumps between frames).
        for index in 1..rotations.len() {
            let previous = rotations.get(index - 1).copied().unwrap_or(DVec3::ZERO);
            if let Some(current) = rotations.get_mut(index) {
                for (value, before) in [(&mut current.x, previous.x), (&mut current.y, previous.y), (&mut current.z, previous.z)] {
                    while *value - before > 180.0 {
                        *value -= 360.0;
                    }
                    while before - *value > 180.0 {
                        *value += 360.0;
                    }
                }
            }
        }
        for (property, values) in [
            ("T", samples.iter().map(|sample| sample.0).collect::<Vec<_>>()),
            ("R", rotations),
            ("S", samples.iter().map(|sample| sample.2).collect()),
        ] {
            let node = writer.ids.take();
            let first = values.first().copied().unwrap_or(DVec3::ZERO);
            writer.object(
                "AnimationCurveNode",
                Node::new("AnimationCurveNode", vec![Prop::I64(node), class_name(property, "AnimCurveNode"), s("")]).with(vec![props70(vec![
                    p("d|X", "Number", "", "A", vec![Prop::F64(first.x)]),
                    p("d|Y", "Number", "", "A", vec![Prop::F64(first.y)]),
                    p("d|Z", "Number", "", "A", vec![Prop::F64(first.z)]),
                ])]),
            );
            writer.link(node, layer);
            let target = match property {
                "T" => "Lcl Translation",
                "R" => "Lcl Rotation",
                _ => "Lcl Scaling",
            };
            writer.link_property(node, model, target);
            curve(writer, node, "d|X", values.iter().map(|value| value.x).collect());
            curve(writer, node, "d|Y", values.iter().map(|value| value.y).collect());
            curve(writer, node, "d|Z", values.iter().map(|value| value.z).collect());
        }
    }
    for ((_, name), values) in percents {
        // The channel deformer of that key (written under the object's blend shape).
        let Some(channel) = writer.objects.iter().find_map(|node| {
            (node.name == "Deformer" && node.prop(2) == Some(&s("BlendShapeChannel")) && node.prop(1) == Some(&class_name(&name, "SubDeformer")))
                .then(|| node.prop(0).and_then(Prop::as_i64))
                .flatten()
        }) else {
            continue;
        };
        let node = writer.ids.take();
        writer.object(
            "AnimationCurveNode",
            Node::new("AnimationCurveNode", vec![Prop::I64(node), class_name("DeformPercent", "AnimCurveNode"), s("")]).with(vec![props70(vec![p(
                "d|DeformPercent",
                "Number",
                "",
                "A",
                vec![Prop::F64(values.first().copied().unwrap_or(0.0))],
            )])]),
        );
        writer.link(node, layer);
        writer.link_property(node, channel, "DeformPercent");
        curve(writer, node, "d|DeformPercent", values);
    }
    Ok(())
}

/// The whole document around the objects and connections.
fn document(scene: &Scene, writer: Writer) -> Vec<Node> {
    let fps = f64::from(scene.animation.timeline.fps);
    let time_mode = [(24.0, 11), (30.0, 6), (25.0, 10), (60.0, 3), (50.0, 4), (120.0, 1)]
        .iter()
        .find(|(rate, _)| (fps - rate).abs() < 1e-6)
        .map_or(14, |(_, mode)| *mode);
    let stop = (f64::from((scene.animation.timeline.end - scene.animation.timeline.start).max(0)) / fps.max(1.0) * TICKS_PER_SECOND).round() as i64;
    let creator = "Kition Scene";
    let mut definitions =
        vec![Node::new("Version", vec![Prop::I32(100)]), Node::new("Count", vec![Prop::I32(writer.counts.values().sum::<i32>() + 1)])];
    definitions.push(Node::new("ObjectType", vec![s("GlobalSettings")]).with(vec![Node::new("Count", vec![Prop::I32(1)])]));
    for (kind, count) in &writer.counts {
        definitions.push(Node::new("ObjectType", vec![s(kind)]).with(vec![Node::new("Count", vec![Prop::I32(*count)])]));
    }
    vec![
        Node::new("FBXHeaderExtension", Vec::new()).with(vec![
            Node::new("FBXHeaderVersion", vec![Prop::I32(1003)]),
            Node::new("FBXVersion", vec![Prop::I32(7400)]),
            Node::new("EncryptionType", vec![Prop::I32(0)]),
            Node::new("CreationTimeStamp", Vec::new()).with(vec![
                Node::new("Version", vec![Prop::I32(1000)]),
                Node::new("Year", vec![Prop::I32(1970)]),
                Node::new("Month", vec![Prop::I32(1)]),
                Node::new("Day", vec![Prop::I32(1)]),
                Node::new("Hour", vec![Prop::I32(0)]),
                Node::new("Minute", vec![Prop::I32(0)]),
                Node::new("Second", vec![Prop::I32(0)]),
                Node::new("Millisecond", vec![Prop::I32(0)]),
            ]),
            Node::new("Creator", vec![s(creator)]),
        ]),
        Node::new("FileId", vec![Prop::Raw(b"KitionSceneFbx74".to_vec())]),
        Node::new("CreationTime", vec![s("1970-01-01 10:00:00:000")]),
        Node::new("Creator", vec![s(creator)]),
        Node::new("GlobalSettings", Vec::new()).with(vec![
            Node::new("Version", vec![Prop::I32(1000)]),
            props70(vec![
                p_int("UpAxis", 1),
                p_int("UpAxisSign", 1),
                p_int("FrontAxis", 2),
                p_int("FrontAxisSign", 1),
                p_int("CoordAxis", 0),
                p_int("CoordAxisSign", 1),
                p_int("OriginalUpAxis", 2),
                p_int("OriginalUpAxisSign", 1),
                p_double("UnitScaleFactor", 1.0),
                p_double("OriginalUnitScaleFactor", 1.0),
                p("AmbientColor", "ColorRGB", "Color", "", vec![Prop::F64(0.0), Prop::F64(0.0), Prop::F64(0.0)]),
                p("DefaultCamera", "KString", "", "", vec![s("Producer Perspective")]),
                p_enum("TimeMode", time_mode),
                p("TimeSpanStart", "KTime", "Time", "", vec![Prop::I64(0)]),
                p("TimeSpanStop", "KTime", "Time", "", vec![Prop::I64(stop)]),
                p_double("CustomFrameRate", fps),
            ]),
        ]),
        Node::new("Documents", Vec::new()).with(vec![
            Node::new("Count", vec![Prop::I32(1)]),
            Node::new("Document", vec![Prop::I64(1), s("Scene"), s("Scene")]).with(vec![Node::new("RootNode", vec![Prop::I64(0)])]),
        ]),
        Node::new("References", Vec::new()),
        Node::new("Definitions", Vec::new()).with(definitions),
        Node::new("Objects", Vec::new()).with(writer.objects),
        Node::new("Connections", Vec::new()).with(writer.links),
        Node::new("Takes", Vec::new()).with(vec![Node::new("Current", vec![s("")])]),
    ]
}
