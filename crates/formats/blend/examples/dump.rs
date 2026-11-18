//! Prints what a .blend file holds: `cargo run -p kition-scene-blend --example dump -- <file.blend>`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

fn main() {
    let path = std::env::args().nth(1).expect("a .blend path");
    let file = kition_scene_blend::read(&std::fs::read(path).unwrap()).unwrap();
    for (index, object) in file.imported.objects.iter().enumerate() {
        let detail = match &object.data {
            kition_scene_model::ObjectData::Mesh(mesh) => {
                format!("mesh {}v {}f {}loose smooth={}", mesh.positions.len(), mesh.faces.len(), mesh.loose_edges.len(), mesh.smooth)
            }
            other => format!("{other:?}"),
        };
        println!(
            "{index} {} loc={:?} rot={:?} scale={:?} parent={:?} material={:?} {detail}",
            object.name, object.location, object.rotation, object.scale, object.parent, object.material
        );
        for modifier in file.modifiers.get(index).into_iter().flatten() {
            println!("    {} {:?} enabled={}", modifier.name, modifier.kind, modifier.enabled);
        }
    }
    println!("materials {:?}", file.imported.materials);
    println!("booleans {:?} hidden {:?} camera {:?} world {:?} render {:?}", file.booleans, file.hidden, file.camera, file.world, file.render);
}
