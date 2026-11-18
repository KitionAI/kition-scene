//! Rigging round 5 against the reference application: envelope deformation and the Armature
//! modifier's vertex group, envelope, and Multi Modifier options; dual quaternions on turned and
//! scaled bones; the weight Gradient tool; X-mirror painting by position and by topology with
//! mirrored group names; and custom bone shapes as the reference's viewport draws them.
//! `tests/blender/rig5.py` builds every case in Blender 5.2.3; its definitions and results are
//! recorded in `tests/fixtures/rig5-reference.json`, which this test always compares with. When
//! `KITION_BLENDER` names a Blender executable the script runs again and the recording must
//! still match it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::armature::{ArmatureDeform, BoneShape};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::weight_paint::{self, Brush, BrushMode, GradientKind, Mirror};
use kition_scene_model::{Armature, Bone, Id, Mesh, Modifier, ModifierKind, ObjectData, Scene, VertexGroup};
use serde_json::{Value, json};

fn reference() -> Value {
    static REFERENCE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    REFERENCE.get_or_init(load).clone()
}

fn load() -> Value {
    let recorded: Value = serde_json::from_str(include_str!("fixtures/rig5-reference.json")).unwrap();
    if let Some(blender) = std::env::var_os("KITION_BLENDER") {
        let dir = std::env::temp_dir().join(format!("kition-scene-rig5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("out.json");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/rig5.py");
        let status = std::process::Command::new(blender)
            .args(["-b", "--factory-startup", "--python"])
            .arg(&script)
            .arg("--")
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "Blender failed");
        let live: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        same(&recorded, &live, "", 1e-4);
        let _ = std::fs::remove_dir_all(&dir);
    }
    recorded
}

fn same(a: &Value, b: &Value, at: &str, tolerance: f64) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "{at}: keys differ");
            for (key, value) in a {
                same(value, &b[key], &format!("{at}/{key}"), tolerance);
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{at}: lengths differ");
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                same(x, y, &format!("{at}/{index}"), tolerance);
            }
        }
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            assert!((x - y).abs() <= tolerance, "{at}: recorded {x}, Blender now {y}");
        }
        (x, y) => assert_eq!(x, y, "{at}"),
    }
}

fn f(value: &Value) -> f32 {
    value.as_f64().unwrap() as f32
}

fn vec3(value: &Value) -> Vec3 {
    Vec3::new(f(&value[0]), f(&value[1]), f(&value[2]))
}

fn vec3_or(value: &Value, default: Vec3) -> Vec3 {
    if value.is_null() { default } else { vec3(value) }
}

fn number_or(value: &Value, default: f32) -> f32 {
    value.as_f64().map_or(default, |value| value as f32)
}

fn flag(value: &Value, default: bool) -> bool {
    value.as_bool().unwrap_or(default)
}

fn row_major(value: &Value) -> Mat4 {
    let row = |r: usize| [f(&value[r][0]), f(&value[r][1]), f(&value[r][2]), f(&value[r][3])];
    Mat4::from_cols_array_2d(&[row(0), row(1), row(2), row(3)]).transpose()
}

/// An armature object from a case's rig: bones with their envelopes, and their pose.
fn make_rig(scene: &mut Scene, spec: &Value) -> Id {
    let bones = spec["bones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bone| {
            let mut made = Bone::new(bone["name"].as_str().unwrap(), vec3(&bone["head"]), vec3(&bone["tail"]));
            made.roll = number_or(&bone["roll"], 0.0);
            made.parent = bone["parent"].as_str().map(str::to_string);
            made.connected = flag(&bone["connected"], false);
            made.deform = flag(&bone["deform"], true);
            made.head_radius = number_or(&bone["headRadius"], 0.1);
            made.tail_radius = number_or(&bone["tailRadius"], 0.05);
            made.envelope_distance = number_or(&bone["distance"], 0.25);
            made.envelope_weight = number_or(&bone["weight"], 1.0);
            made.envelope_multiply = flag(&bone["multiply"], false);
            if let Some(pose) = spec["pose"].get(bone["name"].as_str().unwrap()) {
                made.pose.location = vec3_or(&pose["location"], Vec3::ZERO);
                made.pose.rotation = vec3_or(&pose["rotation"], Vec3::ZERO);
                made.pose.scale = vec3_or(&pose["scale"], Vec3::ONE);
            }
            made
        })
        .collect();
    let id = scene.add_object(spec["name"].as_str().unwrap(), ObjectData::Armature(Armature { bones }));
    let object = scene.object_mut(id).unwrap();
    object.location = vec3_or(&spec["location"], Vec3::ZERO);
    object.rotation = vec3_or(&spec["rotation"], Vec3::ZERO);
    id
}

fn modifier(spec: &Value, rigs: &[Id]) -> Modifier {
    if spec["type"] == "displace" {
        let kind: ModifierKind = serde_json::from_value(
            json!({ "type": "displace", "strength": spec["strength"], "midLevel": spec["midLevel"], "direction": spec["direction"] }),
        )
        .unwrap();
        return Modifier::new(kind);
    }
    let deform = ArmatureDeform {
        vertex_groups: flag(&spec["vertexGroups"], true),
        envelopes: flag(&spec["envelopes"], false),
        vertex_group: spec["vertexGroup"].as_str().map(str::to_string),
        invert_vertex_group: flag(&spec["invertVertexGroup"], false),
        multi_modifier: flag(&spec["multiModifier"], false),
    };
    let rig = rigs[spec["rig"].as_u64().unwrap() as usize];
    Modifier::new(ModifierKind::Armature { object: Some(rig), preserve_volume: flag(&spec["preserveVolume"], false), deform })
}

#[test]
fn envelopes_groups_and_stacks_match_the_reference() {
    let reference = reference();
    let (mut report, mut failures) = (Vec::new(), Vec::new());
    for (label, case) in reference["deform"].as_object().unwrap() {
        let mut scene = Scene::empty();
        let rigs: Vec<Id> = case["rigs"].as_array().unwrap().iter().map(|spec| make_rig(&mut scene, spec)).collect();
        let positions: Vec<Vec3> = case["points"].as_array().unwrap().iter().map(vec3).collect();
        let groups = case["groups"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, weights)| VertexGroup { name: name.clone(), weights: weights.as_array().unwrap().iter().map(f).collect() })
            .collect();
        let mesh = Mesh { positions: positions.clone(), groups, ..Mesh::default() };
        let id = scene.add_object("Points", ObjectData::Mesh(mesh));
        let object = scene.object_mut(id).unwrap();
        object.location = vec3_or(&case["meshLocation"], Vec3::ZERO);
        object.rotation = vec3_or(&case["meshRotation"], Vec3::ZERO);
        object.modifiers = case["modifiers"].as_array().unwrap().iter().map(|spec| modifier(spec, &rigs)).collect();
        scene.validate().unwrap();
        let evaluated = scene.object(id).unwrap().evaluated_mesh(&scene).unwrap();
        let (mut worst, mut moved, mut far) = (0.0f32, 0.0f32, None);
        for (index, ((got, want), rest)) in evaluated.positions.iter().zip(case["deformed"].as_array().unwrap()).zip(&positions).enumerate() {
            let want = vec3(want);
            if got.distance(want) > worst {
                worst = got.distance(want);
                far = Some(format!("vertex {index}: {got} against {want}"));
            }
            moved = moved.max(want.distance(*rest));
        }
        assert!(moved > 0.1, "{label}: the case moves its vertices ({moved})");
        report.push(format!("{label} {worst:.1e} (moves up to {moved:.2} m)"));
        if worst >= 1e-4 {
            failures.push(format!("{label}, {}", far.unwrap_or_default()));
        }
    }
    eprintln!("deformation, worst vertex against Blender: {}", report.join(", "));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The cases recorded for `anim_blender.rs` where dual quaternions blend a turned and scaled
/// bone with another, now within 1e-4.
#[test]
fn dual_quaternions_on_turned_and_scaled_bones_match_the_reference() {
    use kition_scene_model::constraints::{ArmatureTarget, Constraint, ConstraintKind, Target};
    let anim: Value = serde_json::from_str(include_str!("fixtures/anim-reference.json")).unwrap();
    let deform_rig = |scene: &mut Scene| {
        let mut b1 = Bone::new("B1", Vec3::ZERO, Vec3::Z);
        b1.roll = 0.3;
        b1.pose.rotation = Vec3::new(0.5, 0.0, 0.2);
        let mut b2 = Bone::new("B2", Vec3::Z, Vec3::new(0.0, 0.2, 2.0));
        b2.parent = Some("B1".into());
        b2.connected = true;
        b2.pose.rotation = Vec3::new(0.0, 0.3, -1.9);
        b2.pose.scale = Vec3::new(1.0, 1.4, 0.8);
        let id = scene.add_object("Rig", ObjectData::Armature(Armature { bones: vec![b1, b2] }));
        let object = scene.object_mut(id).unwrap();
        object.location = Vec3::new(0.5, 0.2, 0.1);
        object.rotation = Vec3::new(0.2, 0.0, 0.4);
        id
    };
    let mut worst = 0.0f32;
    for weights in [&[0.7f32, 0.3][..], &[0.4, 0.4]] {
        let mut scene = Scene::empty();
        let rig = deform_rig(&mut scene);
        let owner = scene.add_object("O", ObjectData::Empty { size: 1.0 });
        let object = scene.object_mut(owner).unwrap();
        object.location = Vec3::new(0.3, 0.4, 1.4);
        object.rotation = Vec3::new(0.1, 0.2, 0.3);
        let targets = weights
            .iter()
            .zip(["B1", "B2"])
            .map(|(weight, bone)| ArmatureTarget { target: Target { object: rig, bone: Some(bone.into()) }, weight: *weight })
            .collect();
        object.constraints.push(Constraint::new(ConstraintKind::Armature { targets, preserve_volume: true }));
        let names: Vec<String> = weights.iter().map(|weight| format!("{weight:?}")).collect();
        let want = row_major(&anim[format!("armature-{}-True-True", names.join("-"))]);
        let got = scene.world_matrix(owner).unwrap();
        for (a, b) in got.to_cols_array().iter().zip(want.to_cols_array()) {
            worst = worst.max((a - b).abs());
            assert!((a - b).abs() < 1e-4, "the Armature constraint with weights {weights:?}: {got} against {want}");
        }
    }
    let case = &anim["dq-mesh-True"];
    let mut scene = Scene::empty();
    let rig = deform_rig(&mut scene);
    let pairs: Vec<[f32; 2]> = case["weights"].as_array().unwrap().iter().map(|pair| [f(&pair[0]), f(&pair[1])]).collect();
    let mesh = Mesh {
        positions: case["positions"].as_array().unwrap().iter().map(vec3).collect(),
        groups: vec![
            VertexGroup { name: "B1".into(), weights: pairs.iter().map(|pair| pair[0]).collect() },
            VertexGroup { name: "B2".into(), weights: pairs.iter().map(|pair| pair[1]).collect() },
        ],
        ..Mesh::default()
    };
    let id = scene.add_object("Tube", ObjectData::Mesh(mesh.clone()));
    let object = scene.object_mut(id).unwrap();
    object.location = Vec3::new(0.5, 0.2, 0.1);
    object.rotation = Vec3::new(0.2, 0.0, 0.4);
    let object = scene.object(id).unwrap().clone();
    let deformed = kition_scene_model::armature::deform(&mesh, &object, &scene, rig, true).unwrap();
    for (index, (got, want)) in deformed.positions.iter().zip(case["deformed"].as_array().unwrap()).enumerate() {
        worst = worst.max(got.distance(vec3(want)));
        assert!(got.distance(vec3(want)) < 1e-4, "tube vertex {index}: {got} against {}", vec3(want));
    }
    eprintln!("dual quaternions on a turned and scaled bone: worst difference {worst:.1e} (was up to 2.6 cm)");
}

fn brush_of(settings: &Value) -> Brush {
    Brush {
        mode: BrushMode::Draw,
        weight: f(&settings["weight"]),
        strength: f(&settings["strength"]),
        radius: number_or(&settings["radius"], 50.0),
        falloff: serde_json::from_value(settings["falloff"].clone()).unwrap(),
        front_faces_only: false,
    }
}

fn screen_of(case: &Value) -> Vec<(f32, f32)> {
    case["screen"].as_array().unwrap().iter().map(|point| (f(&point[0]), f(&point[1]))).collect()
}

#[test]
fn the_gradient_tool_matches_the_reference() {
    let reference = reference();
    let mut worst = 0.0f32;
    for (label, case) in reference["gradient"].as_object().unwrap() {
        let screen = screen_of(case);
        let mut mesh = Mesh {
            positions: vec![Vec3::ZERO; screen.len()],
            groups: vec![VertexGroup { name: "Group".into(), weights: case["before"].as_array().unwrap().iter().map(f).collect() }],
            ..Mesh::default()
        };
        let line = &case["line"];
        let kind = if case["kind"] == "radial" { GradientKind::Radial } else { GradientKind::Linear };
        let places: Vec<Option<(f32, f32)>> = screen.iter().copied().map(Some).collect();
        weight_paint::gradient(&mut mesh, "Group", &places, (f(&line[0]), f(&line[1])), (f(&line[2]), f(&line[3])), kind, &brush_of(&case["brush"]))
            .unwrap();
        let (mut changed, after) = (0, case["after"].as_array().unwrap());
        for (index, ((got, want), before)) in mesh.groups[0].weights.iter().zip(after).zip(case["before"].as_array().unwrap()).enumerate() {
            worst = worst.max((got - f(want)).abs());
            changed += usize::from((f(want) - f(before)).abs() > 1e-3);
            assert!((got - f(want)).abs() < 1e-4, "{label}, vertex {index}: {got} against {}", f(want));
        }
        assert!(changed > 10, "{label}: the gradient changes {changed} weights");
    }
    eprintln!("gradient: worst weight difference {worst:.1e}");
}

#[test]
fn x_mirror_painting_matches_the_reference() {
    let reference = reference();
    let mut worst = 0.0f32;
    for (label, case) in reference["mirror"].as_object().unwrap() {
        let positions: Vec<Vec3> = case["positions"].as_array().unwrap().iter().map(vec3).collect();
        let faces: Vec<Vec<u32>> = case["faces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|face| face.as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect())
            .collect();
        let groups = case["before"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, weights)| VertexGroup { name: name.clone(), weights: weights.as_array().unwrap().iter().map(f).collect() })
            .collect();
        let mut mesh = Mesh { positions, faces, groups, ..Mesh::default() };
        let mirror = Mirror { partners: weight_paint::mirror_map(&mesh, flag(&case["topology"], false)), names: flag(&case["names"], true) };
        let brush = brush_of(&case["brush"]);
        let screen = screen_of(case);
        let active = case["active"].as_str().unwrap();
        for dab in case["dabs"].as_array().unwrap() {
            let (x, y) = (f(&dab[0]), f(&dab[1]));
            let touches: Vec<(u32, f32)> = screen
                .iter()
                .enumerate()
                .filter_map(|(vertex, (sx, sy))| {
                    let touch = brush.touch((sx - x).hypot(sy - y));
                    (touch > 0.0).then_some((vertex as u32, touch))
                })
                .collect();
            weight_paint::dab_mirrored(&mut mesh, active, &touches, &brush, &mirror).unwrap();
        }
        let after = case["after"].as_object().unwrap();
        let mut names: Vec<&str> = mesh.groups.iter().map(|group| group.name.as_str()).collect();
        names.sort_unstable();
        let mut theirs: Vec<&str> = after.keys().map(String::as_str).collect();
        theirs.sort_unstable();
        assert_eq!(names, theirs, "{label}: the groups");
        for group in &mesh.groups {
            for (index, (got, want)) in group.weights.iter().zip(after[&group.name].as_array().unwrap()).enumerate() {
                worst = worst.max((got - f(want)).abs());
                assert!((got - f(want)).abs() < 1e-4, "{label}, {} vertex {index}: {got} against {}", group.name, f(want));
            }
        }
    }
    eprintln!("x mirror: worst weight difference {worst:.1e}");
}

#[test]
fn names_flip_as_the_reference_flips_them() {
    let reference = reference();
    for (name, flipped) in reference["names"].as_object().unwrap() {
        assert_eq!(weight_paint::flip_name(name), flipped.as_str().unwrap(), "{name}");
    }
}

/// The shape's edges, placed by Kition, against the pixels the reference's viewport drew for
/// them: every drawn pixel within 1.5 pixels of an edge, and every point along an edge (inside
/// the picture) within 1.5 pixels of a drawn one.
#[test]
fn custom_shapes_sit_where_the_reference_draws_them() {
    let reference = reference();
    let size = reference["shapeSize"].as_u64().unwrap() as f32;
    let projection = row_major(&reference["shapeProjection"]);
    let shape: Vec<Vec3> = reference["shapeMesh"]["positions"].as_array().unwrap().iter().map(vec3).collect();
    let edges: Vec<(usize, usize)> = reference["shapeMesh"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e[0].as_u64().unwrap() as usize, e[1].as_u64().unwrap() as usize))
        .collect();
    let mut report = Vec::new();
    for (label, case) in reference["shapes"].as_object().unwrap() {
        let mut scene = Scene::empty();
        let mesh = Mesh { positions: shape.clone(), loose_edges: edges.iter().map(|(a, b)| [*a as u32, *b as u32]).collect(), ..Mesh::default() };
        let shape_id = scene.add_object("Shape", ObjectData::Mesh(mesh));
        let rig = make_rig(&mut scene, &case["rig"]);
        let ObjectData::Armature(armature) = &mut scene.object_mut(rig).unwrap().data else { panic!() };
        for (name, settings) in case["shapes"].as_object().unwrap() {
            let bone = armature.bone_mut(name).unwrap();
            bone.shape = Some(BoneShape {
                translation: vec3_or(&settings["translation"], Vec3::ZERO),
                rotation: vec3_or(&settings["rotation"], Vec3::ZERO),
                scale: vec3_or(&settings["scale"], Vec3::ONE),
                bone_size: flag(&settings["boneSize"], true),
                ..BoneShape::new(shape_id)
            });
        }
        scene.validate().unwrap();
        let world = scene.world_matrix(rig).unwrap();
        let posed = kition_scene_model::constraints::bone_matrices(&scene, rig, 0);
        let armature = kition_scene_model::armature::armature_of(&scene, rig).unwrap();
        let mut worst = 0.0f32;
        for (view_name, view) in reference["shapeViews"].as_object().unwrap() {
            let to_clip = projection * row_major(view);
            let pixel = |point: Vec3| {
                let clip = to_clip * point.extend(1.0);
                ((clip.x / clip.w + 1.0) * size / 2.0 - 0.5, (clip.y / clip.w + 1.0) * size / 2.0 - 0.5)
            };
            let mut segments = Vec::new();
            for bone in armature.bones.iter().filter(|bone| bone.shape.is_some()) {
                let matrix = world * bone.shape.as_ref().unwrap().matrix(posed[&bone.name], bone.length());
                for (a, b) in &edges {
                    segments.push((pixel(matrix.transform_point3(shape[*a])), pixel(matrix.transform_point3(shape[*b]))));
                }
            }
            let drawn: Vec<(f32, f32)> = case["pixels"][view_name].as_array().unwrap().iter().map(|p| (f(&p[0]), f(&p[1]))).collect();
            assert!(drawn.len() > 50, "{label} {view_name}: the reference drew the shape");
            let to_segment = |(px, py): (f32, f32), ((ax, ay), (bx, by)): ((f32, f32), (f32, f32))| {
                let (dx, dy) = (bx - ax, by - ay);
                let length = dx * dx + dy * dy;
                let t = if length > 0.0 { (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, 1.0) } else { 0.0 };
                (px - ax - t * dx).hypot(py - ay - t * dy)
            };
            for point in &drawn {
                let nearest = segments.iter().map(|segment| to_segment(*point, *segment)).fold(f32::INFINITY, f32::min);
                worst = worst.max(nearest);
                assert!(nearest <= 1.5, "{label} {view_name}: pixel {point:?} is {nearest} px from Kition's shape");
            }
            for ((ax, ay), (bx, by)) in &segments {
                for step in 0..=20 {
                    let t = step as f32 / 20.0;
                    let point = (ax + (bx - ax) * t, ay + (by - ay) * t);
                    if !(0.0..size).contains(&point.0) || !(0.0..size).contains(&point.1) {
                        continue;
                    }
                    let nearest = drawn.iter().map(|(x, y)| (x - point.0).hypot(y - point.1)).fold(f32::INFINITY, f32::min);
                    worst = worst.max(nearest);
                    assert!(nearest <= 1.5, "{label} {view_name}: Kition's edge at {point:?} is {nearest} px from the drawing");
                }
            }
        }
        report.push(format!("{label} {worst:.2} px"));
    }
    eprintln!("custom shapes, farthest pixel: {}", report.join(", "));
}
