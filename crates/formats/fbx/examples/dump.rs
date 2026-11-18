//! Prints an FBX file's node tree: `cargo run -p kition-scene-fbx --example dump -- <file> [depth]`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_fbx::node::{Node, Prop, parse};

fn show(node: &Node, depth: usize, max: usize) {
    let props: Vec<String> = node
        .props
        .iter()
        .map(|prop| match prop {
            Prop::F64Array(values) => format!("d[{}] {:?}", values.len(), &values[..values.len().min(if values.len() == 16 { 16 } else { 12 })]),
            Prop::F32Array(values) => format!("f[{}] {:?}", values.len(), &values[..values.len().min(12)]),
            Prop::I32Array(values) => format!("i[{}] {:?}", values.len(), &values[..values.len().min(24)]),
            Prop::I64Array(values) => format!("l[{}] {:?}", values.len(), &values[..values.len().min(12)]),
            Prop::Raw(data) => format!("raw[{}]", data.len()),
            other => format!("{other:?}"),
        })
        .collect();
    println!("{}{}: {}", "  ".repeat(depth), node.name, props.join(", "));
    if depth < max {
        for child in &node.children {
            show(child, depth + 1, max);
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let max: usize = std::env::args().nth(2).map_or(20, |text| text.parse().unwrap());
    let (nodes, version) = parse(&std::fs::read(path).unwrap()).unwrap();
    println!("version {version}");
    for node in &nodes {
        show(node, 0, max);
    }
}
