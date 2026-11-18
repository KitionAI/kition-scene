//! The Multiresolution modifier's operators ([`kition_scene_model::multires`]): Subdivide adds a
//! level (and the modifier, when the mesh has none), Delete Higher drops the levels above the
//! sculpt level, Apply Base moves the base mesh towards the sculpted shape, and Reshape copies
//! the highest level's shape from another mesh with the same vertices. The levels shown and
//! sculpted are the modifier's settings (`modifier.set` with `levels` and `sculptLevels`). Each
//! operator is one undo step. Unsubdivide recovers one lower base; Rebuild Subdivisions recovers
//! every supported lower base, preserving detail through the mesh blob's existing levels.

use kition_kit::KitCommand;
use kition_scene_model::multires;
use kition_scene_model::{Id, Modifier, ModifierKind};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

struct Spec {
    id: &'static str,
    title: &'static str,
    schema: fn() -> Value,
}

fn object_field() -> (&'static str, Value) {
    ("object", s::object("The mesh; the sculpted or active one when left out"))
}

static SPECS: &[Spec] = &[
    Spec { id: "multires.subdivide", title: "Multires Subdivide", schema: || s::of(vec![object_field()]) },
    Spec { id: "multires.unsubdivide", title: "Multires Unsubdivide", schema: || s::of(vec![object_field()]) },
    Spec { id: "multires.rebuildSubdivisions", title: "Multires Rebuild Subdivisions", schema: || s::of(vec![object_field()]) },
    Spec { id: "multires.deleteHigher", title: "Multires Delete Higher", schema: || s::of(vec![object_field()]) },
    Spec { id: "multires.applyBase", title: "Multires Apply Base", schema: || s::of(vec![object_field()]) },
    Spec {
        id: "multires.reshape",
        title: "Multires Reshape",
        schema: || s::of(vec![object_field(), ("from", s::required(s::object("A mesh with the highest level's vertices, whose shape is copied")))]),
    },
];

/// The mesh the operators work on when none is named.
fn implied(doc: &SceneDocument) -> Option<Id> {
    doc.session
        .sculpt
        .as_ref()
        .map(|session| session.object)
        .or(doc.session.editor.active)
        .filter(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
}

fn enabled(doc: &SceneDocument) -> bool {
    doc.session.mode == Mode::Object && implied(doc).is_some()
}

/// Every operator, with whether it can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            schema: Some((spec.schema)()),
            mutates: true,
            enabled: enabled(doc),
            ..Default::default()
        })
        .collect()
}

/// The operators that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    if enabled(doc) { Vec::new() } else { SPECS.iter().map(|spec| spec.id.to_string()).collect() }
}

/// Runs an operator, or answers `None` when `command` is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = SPECS.iter().find(|spec| spec.id == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    let result = run(doc, command, params).and_then(|value| {
        doc.scene.validate().map_err(|error| format!("failed: the change would break the scene ({error})"))?;
        Ok(value)
    });
    Some(match result {
        Ok(value) => {
            doc.tidy_session();
            doc.committed(before).map(|()| value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    })
}

/// The index of the object's Multiresolution modifier.
fn modifier_index(doc: &SceneDocument, object: Id) -> Option<usize> {
    doc.scene.object(object).ok()?.modifiers.iter().position(|modifier| matches!(modifier.kind, ModifierKind::Multires { .. }))
}

/// Sets the modifier's levels shown and sculpted to at most `levels`, or to it with `all`.
fn set_levels(doc: &mut SceneDocument, object: Id, index: usize, levels: u32, all: bool) -> Result<(), String> {
    let found = doc.scene.object_mut(object).map_err(|error| error.to_string())?;
    if let Some(Modifier { kind: ModifierKind::Multires { levels: shown, sculpt_levels }, .. }) = found.modifiers.get_mut(index) {
        if all {
            (*shown, *sculpt_levels) = (levels, levels);
        } else {
            (*shown, *sculpt_levels) = ((*shown).min(levels), (*sculpt_levels).min(levels));
        }
    }
    Ok(())
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    if doc.session.mode != Mode::Object {
        return Err("not-allowed: the multiresolution operators work in object or sculpt mode".into());
    }
    let object = match params.get("object").filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value)?,
        None => implied(doc).ok_or("not-allowed: select a mesh")?,
    };
    if doc.scene.object(object).map_err(|error| error.to_string())?.mesh().is_none() {
        return Err(bad("only a mesh has multiresolution levels"));
    }
    let index = match (command, modifier_index(doc, object)) {
        (_, Some(index)) => index,
        ("multires.subdivide", None) => {
            let found = doc.scene.object_mut(object).map_err(|error| error.to_string())?;
            found.modifiers.push(Modifier::new(ModifierKind::Multires { levels: 0, sculpt_levels: 0 }));
            found.modifiers.len() - 1
        }
        _ => return Err("not-allowed: the mesh has no Multires modifier".into()),
    };
    let dyntopo = doc.session.sculpt.as_ref().is_some_and(|session| session.object == object && session.dyntopo.enabled);
    let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
    let failed = |error: kition_scene_model::SceneError| format!("not-allowed: {}", error.to_string().replace("invalid-params: ", ""));
    match command {
        "multires.subdivide" => {
            if dyntopo {
                return Err("not-allowed: turn dynamic topology off first".into());
            }
            let levels = multires::subdivide(mesh).map_err(failed)?;
            set_levels(doc, object, index, levels, true)?;
        }
        "multires.unsubdivide" | "multires.rebuildSubdivisions" => {
            if dyntopo {
                return Err("not-allowed: turn dynamic topology off first".into());
            }
            let levels =
                if command == "multires.unsubdivide" { multires::unsubdivide(mesh) } else { multires::rebuild_subdivisions(mesh) }.map_err(failed)?;
            // Rebuild chooses the finest viewport and sculpt levels. Unsubdivide keeps both
            // numeric settings, as the reference operator does; the old base is now level 1.
            if command == "multires.rebuildSubdivisions" {
                set_levels(doc, object, index, levels, true)?;
            }
        }
        "multires.deleteHigher" => {
            let keep = match doc.scene.object(object).ok().and_then(|found| found.modifiers.get(index)).map(|modifier| &modifier.kind) {
                Some(ModifierKind::Multires { sculpt_levels, .. }) => *sculpt_levels,
                _ => 0,
            };
            let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
            multires::delete_higher(mesh, keep);
            set_levels(doc, object, index, keep, false)?;
        }
        "multires.applyBase" => multires::apply_base(mesh).map_err(failed)?,
        "multires.reshape" => {
            let from = params.get("from").filter(|value| !value.is_null()).ok_or_else(|| bad("`from` is required"))?;
            let from = params::resolve(&doc.scene, from)?;
            if from == object {
                return Err(bad("reshape copies from another mesh"));
            }
            let positions =
                doc.scene.object(from).map_err(|error| error.to_string())?.mesh().ok_or_else(|| bad("`from` is not a mesh"))?.positions.clone();
            let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
            multires::reshape(mesh, &positions).map_err(failed)?;
        }
        other => return Err(format!("unknown-command: {other}")),
    }
    let levels = doc.scene.object(object).ok().and_then(|found| found.mesh()).map_or(0, multires::levels);
    Ok(json!({ "object": object, "levels": levels, "modifier": index }))
}
