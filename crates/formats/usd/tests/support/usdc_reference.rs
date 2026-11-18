//! Compare against the public-API reports recorded by tests/blender/usdc.py.
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{ObjectData, Scene};
use serde_json::{Value, json};
use std::collections::BTreeMap;
const FRAMES: [f32; 3] = [1.0, 11.0, 21.0];
/// Our report of a scene, in the same shape as the script's.
pub fn report(scene: &Scene) -> Value {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    for frame in FRAMES {
        let at = scene.at_frame(frame);
        for object in &at.objects {
            let entry = out.entry(object.name.clone()).or_insert_with(|| json!({ "worlds": [], "deformed": [] }));
            let world = at.world_matrix(object.id).unwrap();
            entry["worlds"].as_array_mut().unwrap().push(json!(world.to_cols_array()));
            match &object.data {
                // Imports never make curves, texts or surfaces (they come in as meshes).
                ObjectData::Curve(_) | ObjectData::Text(_) => entry["type"] = json!("CURVE"),
                ObjectData::Surface(_) => entry["type"] = json!("SURFACE"),
                ObjectData::Mesh(mesh) => {
                    let evaluated = object.evaluated_mesh(&at).unwrap();
                    entry["deformed"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(evaluated.positions.iter().map(|point| world.transform_point3(*point).to_array()).collect::<Vec<_>>()));
                    entry["type"] = json!("MESH");
                    // A base colour image: a PNG's size from its header.
                    if let Some(image) = object
                        .material
                        .and_then(|id| at.material(id).ok())
                        .and_then(|material| material.base_color_texture.as_ref())
                        .and_then(|hash| at.image(hash))
                    {
                        let field = |at: usize| u32::from_be_bytes(image.bytes[at..at + 4].try_into().unwrap());
                        entry["texture"] = json!([field(16), field(20)]);
                    }
                    entry["vertices"] = json!(mesh.positions.len());
                    entry["faces"] = json!(mesh.faces.len());
                    let mut groups: BTreeMap<String, Vec<f32>> =
                        mesh.groups.iter().map(|group| (group.name.clone(), vec![0.0; mesh.positions.len()])).collect();
                    for vertex in 0..mesh.positions.len() {
                        let total: f32 = mesh.groups.iter().filter_map(|group| group.weights.get(vertex)).sum();
                        for group in &mesh.groups {
                            if let (Some(weight), Some(list)) = (group.weights.get(vertex), groups.get_mut(&group.name))
                                && total > 0.0
                            {
                                list[vertex] = weight / total;
                            }
                        }
                    }
                    entry["groups"] = json!(groups);
                    let material = object.material.and_then(|id| at.material(id).ok()).filter(|material| material.base_color_texture.is_none());
                    entry["material"] = json!(material.map(|material| [
                        material.base_color[0],
                        material.base_color[1],
                        material.base_color[2],
                        material.metallic,
                        material.roughness
                    ]));
                }
                ObjectData::Light(light) => {
                    entry["type"] = json!("LIGHT");
                    entry["light"] =
                        json!([format!("{:?}", light.kind).to_uppercase(), light.power, light.radius, light.spot_size, light.color.to_array()]);
                }
                ObjectData::Camera(camera) => {
                    entry["type"] = json!("CAMERA");
                    entry["camera"] = json!([if camera.orthographic { "ORTHO" } else { "PERSP" }, camera.lens, camera.sensor, camera.ortho_scale]);
                }
                ObjectData::Armature(_) => entry["type"] = json!("ARMATURE"),
                ObjectData::Empty { .. } | ObjectData::Lattice(_) => entry["type"] = json!("EMPTY"),
            }
        }
    }
    json!(out)
}

fn matrix(value: &Value) -> Mat4 {
    let values: Vec<f32> = value.as_array().unwrap().iter().map(|value| value.as_f64().unwrap() as f32).collect();
    Mat4::from_cols_slice(&values)
}

fn points(value: &Value) -> Vec<Vec3> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|point| Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32))
        .collect()
}

fn close(label: &str, a: &Value, b: &Value) {
    let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
    assert!((a - b).abs() < 1e-3 * a.abs().max(1.0), "{label}: {a} and {b}");
}

/// Every object of `reference` agrees with `ours`.
pub fn compare(label: &str, reference: &Value, ours: &Value) {
    for (name, theirs) in reference.as_object().unwrap() {
        let Some(mut mine) = ours.get(name) else {
            panic!("{label}: {name} is missing; ours has {:?}", ours.as_object().unwrap().keys().collect::<Vec<_>>())
        };
        let kind = theirs["type"].as_str().unwrap();
        // The reference's reader splits an object with both data and children into an empty and
        // a ".001" object holding the data (its own files included).
        if mine["type"] == "EMPTY" && kind != "EMPTY" {
            mine = ours.get(format!("{name}.001")).unwrap_or_else(|| panic!("{label}: {name} is an empty with no data object"));
        }
        assert_eq!(mine["type"].as_str(), Some(kind), "{label}: {name}'s type");
        assert_eq!(theirs["worlds"].as_array().unwrap().len(), mine["worlds"].as_array().unwrap().len(), "{label}: {name} world sample count");
        for (frame, (a, b)) in theirs["worlds"].as_array().unwrap().iter().zip(mine["worlds"].as_array().unwrap()).enumerate() {
            let (a, b) = (matrix(a), matrix(b));
            assert!(a.abs_diff_eq(b, 2e-3), "{label}: {name}'s world at frame {} differs:\n{a}\n{b}", FRAMES[frame]);
        }
        match kind {
            "MESH" => {
                assert_eq!(theirs["vertices"], mine["vertices"], "{label}: {name}'s vertex count");
                assert_eq!(theirs["faces"], mine["faces"], "{label}: {name}'s face count");
                assert_eq!(
                    theirs["deformed"].as_array().unwrap().len(),
                    mine["deformed"].as_array().unwrap().len(),
                    "{label}: {name} deformation sample count"
                );
                for (frame, (a, b)) in theirs["deformed"].as_array().unwrap().iter().zip(mine["deformed"].as_array().unwrap()).enumerate() {
                    let (a, b) = (points(a), points(b));
                    assert_eq!(a.len(), b.len(), "{label}: {name}'s vertex count");
                    let worst = a.iter().zip(&b).map(|(a, b)| a.distance(*b)).fold(0.0f32, f32::max);
                    assert!(worst < 2e-3, "{label}: {name}'s vertices at frame {} are off by {worst}", FRAMES[frame]);
                }
                // Every group with weight in theirs has the same normalised weights in ours.
                for (group, weights) in theirs["groups"].as_object().into_iter().flatten() {
                    let weights = weights.as_array().unwrap();
                    if weights.iter().all(|weight| weight.as_f64() == Some(0.0)) {
                        continue;
                    }
                    let ours = mine["groups"][group].as_array().unwrap_or_else(|| panic!("{label}: {name} has no group {group}"));
                    assert_eq!(weights.len(), ours.len(), "{label}: {name} weight count in {group}");
                    for (a, b) in weights.iter().zip(ours) {
                        assert!((a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 2e-3, "{label}: {name}'s weights in {group} differ: {a} and {b}");
                    }
                }
                if let Some(material) = theirs["material"].as_array() {
                    let ours = mine["material"].as_array().unwrap_or_else(|| panic!("{label}: {name} has no material"));
                    assert_eq!(material.len(), ours.len(), "{label}: {name} material component count");
                    for (slot, (a, b)) in material.iter().zip(ours).enumerate() {
                        close(&format!("{label}: {name}'s material {slot}"), a, b);
                    }
                }
            }
            "LIGHT" => {
                assert_eq!(theirs["light"][0], mine["light"][0], "{label}: {name}'s kind");
                close(&format!("{label}: {name}'s power"), &theirs["light"][1], &mine["light"][1]);
                if theirs["light"][0] != "SUN" {
                    close(&format!("{label}: {name}'s size"), &theirs["light"][2], &mine["light"][2]);
                }
                if theirs["light"][0] == "SPOT" {
                    close(&format!("{label}: {name}'s spot size"), &theirs["light"][3], &mine["light"][3]);
                }
                for channel in 0..3 {
                    close(&format!("{label}: {name}'s colour"), &theirs["light"][4][channel], &mine["light"][4][channel]);
                }
            }
            "CAMERA" => {
                assert_eq!(theirs["camera"][0], mine["camera"][0], "{label}: {name}'s projection");
                close(&format!("{label}: {name}'s lens"), &theirs["camera"][1], &mine["camera"][1]);
                close(&format!("{label}: {name}'s sensor"), &theirs["camera"][2], &mine["camera"][2]);
            }
            _ => {}
        }
    }
}
