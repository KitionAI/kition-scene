//! Imports an FBX file into an empty scene and prints its objects:
//! `cargo run -p kition-scene-fbx --example summary -- <file>`.
#![allow(clippy::unwrap_used)]

use kition_scene_model::{ObjectData, Scene};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let imported = kition_scene_fbx::read(&std::fs::read(path).unwrap()).unwrap();
    let mut scene = Scene::empty();
    kition_scene_exchange::place(&mut scene, imported).unwrap();
    for object in &scene.objects {
        let world = scene.world_matrix(object.id).unwrap();
        let (scale, rotation, translation) = world.to_scale_rotation_translation();
        let detail = match &object.data {
            ObjectData::Curve(_) => "curve".to_string(),
            ObjectData::Lattice(_) => "lattice".to_string(),
            ObjectData::Text(_) => "text".to_string(),
            ObjectData::Surface(_) => "surface".to_string(),
            ObjectData::Mesh(mesh) => format!(
                "mesh v{} f{} uv{} smooth{} groups{:?} keys{:?}",
                mesh.positions.len(),
                mesh.faces.len(),
                mesh.has_uvs(),
                mesh.smooth,
                mesh.groups.iter().map(|group| (&group.name, group.weights.iter().filter(|weight| **weight > 0.0).count())).collect::<Vec<_>>(),
                mesh.shape_keys.iter().map(|key| (&key.name, key.value)).collect::<Vec<_>>()
            ),
            ObjectData::Armature(armature) => format!(
                "armature {:?}",
                armature.bones.iter().map(|bone| (&bone.name, bone.head, bone.tail, bone.roll, &bone.parent, bone.connected)).collect::<Vec<_>>()
            ),
            ObjectData::Camera(camera) => format!(
                "camera lens{} sensor{} clip{}..{} ortho{} {}",
                camera.lens, camera.sensor, camera.clip_start, camera.clip_end, camera.orthographic, camera.ortho_scale
            ),
            ObjectData::Light(light) => format!("light {:?} {}W spot{}", light.kind, light.power, light.spot_size.to_degrees()),
            ObjectData::Empty { .. } => "empty".into(),
        };
        let euler = kition_scene_model::scene::matrix_euler(kition_scene_model::glam::Mat3::from_quat(rotation));
        println!(
            "{} parent={:?} loc={:.4} rot={:.4} scale={:.4} {} material={:?}",
            object.name,
            object.parent,
            translation,
            euler,
            scale,
            detail,
            object.material.and_then(|id| scene.material(id).ok()).map(|material| (
                &material.name,
                material.base_color,
                material.base_color_texture.is_some()
            ))
        );
    }
    println!("channels {}", scene.animation.channels.len());
    for channel in scene.animation.channels.iter().take(12) {
        println!(
            "  {:?}[{}] {} keys {} first {:?} last {:?}",
            channel.property,
            channel.index,
            channel.path,
            channel.keys.len(),
            channel.keys.first().map(|key| (key.frame, key.value)),
            channel.keys.last().map(|key| (key.frame, key.value))
        );
    }
}
