//! Binary export is checked offline against recorded Blender scenes. KITION_BLENDER also
//! opens each new binary export in Blender and checks its geometry, rig and animation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::Scene;
use kition_scene_usd::{usda, usdc};

fn placed(bytes: &[u8]) -> Scene {
    let mut scene = Scene::empty();
    kition_scene_exchange::place(&mut scene, kition_scene_usd::read(bytes).unwrap()).unwrap();
    scene
}

#[test]
fn binary_starter_scene_is_deterministic_and_round_trips() {
    let scene = Scene::starter();
    let bytes = kition_scene_usd::write_binary(&scene, None).unwrap();
    assert!(bytes.starts_with(b"PXR-USDC"));
    assert_eq!(bytes, kition_scene_usd::write_binary(&scene, None).unwrap());
    let text = kition_scene_usd::write(&scene, None).unwrap();
    let a = placed(&text);
    let b = placed(&bytes);
    same_numbers(&serde_json::to_value(a).unwrap(), &serde_json::to_value(b).unwrap(), "starter");
}

#[test]
fn binary_writer_rejects_unsupported_and_hostile_values() {
    for text in [
        "#usda 1.0\ndef Xform \"A\" { float x = inf }",
        "#usda 1.0\ndef Xform \"A\" { float x = 1e300 }",
        "#usda 1.0\ndef Xform \"A\" { half3 x = (1e10, 1, 1) }",
        "#usda 1.0\ndef Xform \"A\" { int x = 0.5 }",
        "#usda 1.0\ndef Xform \"A\" { uint x = -1 }",
        "#usda 1.0\ndef Xform \"A\" { float3 x = (1, 2) }",
        "#usda 1.0\ndef Xform \"A\" { unknown x = 1 }",
        "#usda 1.0\ndef Xform \"A\" ( documentation = \"unsupported\" ) {}",
        "#usda 1.0\ndef Xform \"A\" { token x = \"bad\\0token\" }",
        "#usda 1.0\ndef Xform \"A\" { rel x = <../relative> }",
        "#usda 1.0\ndef Xform \"A B\" {}",
        "#usda 1.0\ndef Xform \"A\" { rel x = </B.> }",
        "#usda 1.0\ndef Xform \"A\" { rel x = </B//C> }",
    ] {
        assert!(usdc::write(&usda::parse(text).unwrap()).is_err(), "{text}");
    }
    let mut layer = usda::parse("#usda 1.0\ndef Xform \"A\" {}").unwrap();
    for _ in 0..40 {
        let mut outer = layer.prims[0].clone();
        outer.children = std::mem::take(&mut layer.prims);
        layer.prims.push(outer);
    }
    assert!(usdc::write(&layer).is_err());
}

#[test]
fn binary_values_preserve_declared_types_and_qualifiers() {
    let text = r#"#usda 1.0
(metersPerUnit = 1 upAxis = "Z")
def Xform "A" {
    custom string display = "A name"
    uniform half3 scales = (1, 0.5, -2)
    quatf rotation = (0.5, 0.25, -0.75, 1)
    double precise = 0.1234567890123456
    float[] empty = []
    double3 xformOp:translate.timeSamples = {1: (0, 0, 0), 21: (3, 2, 1)}
    uniform token[] xformOpOrder = ["xformOp:translate"]
    rel binding = </B>
}
def Material "B" {}
"#;
    let layer = usda::parse(text).unwrap();
    let binary = usdc::parse(&usdc::write(&layer).unwrap()).unwrap();
    let a = &binary.prims[0];
    assert!(a.property("display").unwrap().custom);
    assert!(a.property("scales").unwrap().uniform);
    assert_eq!(a.value("scales").unwrap().numbers(), vec![1.0, 0.5, -2.0]);
    assert_eq!(a.value("rotation").unwrap().numbers(), vec![0.5, 0.25, -0.75, 1.0]);
    assert_eq!(a.value("precise").unwrap().as_f64(), Some(0.1234567890123456));
    assert_eq!(a.value("empty").unwrap().items().len(), 0);
    assert_eq!(a.property("xformOp:translate").unwrap().samples, layer.prims[0].property("xformOp:translate").unwrap().samples);
    assert_eq!(a.value("binding").unwrap().as_str(), Some("/B"));
}

#[path = "support/usdc_reference.rs"]
mod reference;

#[test]
fn binary_exports_match_recorded_blender_scenes() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usdc6");
    let dir = std::env::temp_dir().join(format!("kition-usdc6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["default", "things", "skinned"] {
        let expected = serde_json::from_slice(&std::fs::read(fixtures.join(format!("{name}.json"))).unwrap()).unwrap();
        let scene = placed(&std::fs::read(fixtures.join(format!("{name}.usdc"))).unwrap());
        let before = reference::report(&scene);
        reference::compare(name, &expected, &before);
        let binary = kition_scene_usd::write_binary(&scene, None).unwrap();
        let after = reference::report(&placed(&binary));
        reference::compare(name, &expected, &after);
        let ascii = kition_scene_usd::write(&scene, None).unwrap();
        let text_scene = placed(&ascii);
        let binary_scene = placed(&binary);
        // Identical KDM-relevant scene data after importing each representation (float
        // precision is the declared USD type in both readers).
        same_numbers(&serde_json::to_value(text_scene).unwrap(), &serde_json::to_value(binary_scene).unwrap(), name);
        if let Ok(blender) = std::env::var("KITION_BLENDER") {
            let file = dir.join(format!("{name}.usdc"));
            let report = dir.join(format!("{name}.json"));
            std::fs::write(&file, binary).unwrap();
            let text_file = dir.join(format!("{name}.usda"));
            std::fs::write(&text_file, ascii).unwrap();
            let text_stage = openusd_report(&blender, &text_file, &dir.join(format!("{name}-text-stage.json")));
            let binary_stage = openusd_report(&blender, &file, &dir.join(format!("{name}-binary-stage.json")));
            assert_eq!(text_stage, binary_stage, "OpenUSD authored fields: {name}");
            let status = std::process::Command::new(blender)
                .args(["-b", "--factory-startup", "--python"])
                .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/usdc.py"))
                .arg("--")
                .arg("--report")
                .arg(file)
                .arg(&report)
                .status()
                .unwrap();
            assert!(status.success());
            let read = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
            reference::compare(name, &before, &read);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

// USDA's text parser retains decimal doubles until import; USDC rounds a declared float to
// binary32. Quaternion/Euler conversion can then differ by a few float ULPs. Structure and
// integer values stay exact; the numerical limit is one part per million.
fn same_numbers(a: &serde_json::Value, b: &serde_json::Value, path: &str) {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((a - b).abs() <= 1e-6 * a.abs().max(1.0), "{path}: {a} != {b}");
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                same_numbers(a, b, &format!("{path}/{i}"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>(), "{path}");
            for (key, a) in a {
                same_numbers(a, &b[key], &format!("{path}/{key}"));
            }
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}

#[test]
fn binary_writer_bounds_target_paths_and_rejects_duplicate_specs() {
    for suffix in ["rel link", "token link.connect"] {
        let path = format!("/{}Leaf", "Child/".repeat(3000));
        let text = format!("#usda 1.0\ndef Xform \"A\" {{ {suffix} = <{path}> }}");
        assert!(usdc::write(&usda::parse(&text).unwrap()).is_err());
    }
    let duplicate = usda::parse("#usda 1.0\ndef Xform \"A\" {} def Xform \"A\" {}").unwrap();
    assert!(usdc::write(&duplicate).is_err());
}

#[test]
fn binary_values_match_blenders_independent_openusd_reader() {
    let input = include_str!("fixtures/usdc6/typed.usda");
    let expected: serde_json::Value = serde_json::from_str(include_str!("fixtures/usdc6/typed.stage.json")).unwrap();
    let binary = usdc::write(&usda::parse(input).unwrap()).unwrap();
    let parsed = usdc::parse(&binary).unwrap();
    let ours = layer_report(&parsed);
    assert_eq!(canonical_numbers(ours), canonical_numbers(expected.clone()));
    if let Ok(blender) = std::env::var("KITION_BLENDER") {
        let dir = std::env::temp_dir().join(format!("kition-usdc6-types-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("typed.usdc");
        std::fs::write(&file, binary).unwrap();
        let actual = openusd_report(&blender, &file, &dir.join("typed.json"));
        assert_eq!(actual, expected);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn openusd_report(blender: &str, file: &std::path::Path, output: &std::path::Path) -> serde_json::Value {
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/usdc_stage.py"))
        .arg("--")
        .arg(file)
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success());
    serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap()
}

fn layer_report(layer: &usda::Layer) -> serde_json::Value {
    use serde_json::{Value, json};
    fn value(value: &usda::Value) -> Value {
        match value {
            usda::Value::Bool(v) => json!(v),
            usda::Value::Number(v) => json!(v),
            usda::Value::Dict(items) => json!(items.iter().map(|(key, entry)| (key.clone(), value_of(entry))).collect::<serde_json::Map<_, _>>()),
            usda::Value::List(items) | usda::Value::Tuple(items) => json!(items.iter().map(value_of).collect::<Vec<_>>()),
            other => json!(other.as_str().unwrap()),
        }
    }
    fn value_of(v: &usda::Value) -> Value {
        value(v)
    }
    fn targets(v: Option<&usda::Value>) -> Vec<String> {
        match v {
            None => Vec::new(),
            Some(usda::Value::List(items)) => items.iter().map(|v| v.as_str().unwrap().to_string()).collect(),
            Some(v) => vec![v.as_str().unwrap().to_string()],
        }
    }
    fn visit(prim: &usda::Prim, owner: &str, out: &mut serde_json::Map<String, Value>) {
        let path = format!("{owner}/{}", prim.name);
        let mut props = serde_json::Map::new();
        for p in &prim.properties {
            let entry = if p.relationship {
                json!({"targets": targets(p.value.as_ref())})
            } else {
                json!({"type":p.type_name,"uniform":p.uniform,"custom":p.custom,"default":p.value.as_ref().map(value),
                    "samples":p.samples.iter().map(|(time,v)|json!([time,value(v)])).collect::<Vec<_>>(),"connections":targets(p.connect.as_ref())})
            };
            props.insert(p.name.clone(), entry);
        }
        out.insert(
            path.clone(),
            json!({"type":prim.type_name,"properties":props,
            "customData":prim.meta("customData").map(value).unwrap_or_else(||json!({})),
            "apiSchemas":prim.meta("apiSchemas").map(value).unwrap_or_else(||json!([]))}),
        );
        for child in &prim.children {
            visit(child, &path, out);
        }
    }
    let mut out = serde_json::Map::new();
    for prim in &layer.prims {
        visit(prim, "", &mut out);
    }
    Value::Object(out)
}

fn canonical_numbers(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n) => serde_json::json!(n.as_f64().unwrap()),
        Value::Array(items) => Value::Array(items.into_iter().map(canonical_numbers).collect()),
        Value::Object(items) => Value::Object(items.into_iter().map(|(key, value)| (key, canonical_numbers(value))).collect()),
        other => other,
    }
}
