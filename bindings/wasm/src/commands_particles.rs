//! Particle systems and Particle Edit mode, as the reference application's Particles panel and
//! Particle Edit mode ([`kition_scene_model::particles`]).
//!
//! A mesh object holds particle systems: emitters, whose particles are born over a frame range
//! and fly under gravity, and hair. The settings are the document's (they merge per object, like
//! modifiers); the particles are derived from them and the seed at each frame, never saved, so
//! any frame shows at once and every replica sees the same ones. Grooming hair in Particle Edit
//! mode is the person's work and is saved (a blob per object). The viewport, renders, the path
//! tracer, and exports see particles through [`realized`]: halos as small octahedra, instances as
//! copies of their object, and hair as tapering ribbons (facing the eye in pictures; across the
//! root's normal in exports, which bake hair to ribbon meshes). Clicking a particle selects its
//! emitter.

use std::collections::BTreeMap;

use glam::{Vec3, Vec4Swizzles};
use kition_kit::KitCommand;
use kition_kit::tools::{PointerAction, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::particles::{self, Cache, GroomBrush, GroomDab, HairDynamics, Kind, ParticleSystem, View, hair};
use kition_scene_model::{Id, ObjectData, Scene};
use kition_scene_raster::Projector;
use serde::Serialize;
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument, Step};

/// Particles worked out for pictures: recent frames, and which emitter each shown object is.
#[derive(Default)]
pub struct Derived {
    cache: Cache,
    owners: BTreeMap<Id, Id>,
}

/// A grooming brush's settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Groom {
    #[serde(serialize_with = "brush_id")]
    pub kind: GroomBrush,
    /// Radius in viewport pixels.
    pub radius: f32,
    pub strength: f32,
    pub subtract: bool,
}

fn brush_id<S: serde::Serializer>(brush: &GroomBrush, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(brush.id())
}

impl Default for Groom {
    fn default() -> Self {
        Self { kind: GroomBrush::Comb, radius: 50.0, strength: 0.5, subtract: false }
    }
}

/// A hair system in Particle Edit mode.
#[derive(Debug, Clone)]
pub struct ParticleEditSession {
    pub object: Id,
    /// The system's index on the object.
    pub system: usize,
    pub brush: Groom,
    stroke: Option<Live>,
}

impl PartialEq for ParticleEditSession {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object && self.system == other.system && self.brush == other.brush
    }
}

/// A grooming stroke under way.
#[derive(Debug, Clone)]
struct Live {
    before: Box<Step>,
    projector: Projector,
    last: (f64, f64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// A mesh is active (or named).
    Mesh,
    /// The active mesh has a particle system.
    System,
    /// A hair system can be groomed (or Particle Edit is on).
    Groomable,
    Edit,
    /// Object mode, with no other mode's session (no command's own gate: what a named mesh needs).
    ObjectMode,
}

struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    mutates: bool,
    needs: Needs,
    schema: fn() -> Value,
}

const fn spec(id: &'static str, title: &'static str, menu: &'static [&'static str], mutates: bool, needs: Needs, schema: fn() -> Value) -> Spec {
    Spec { id, title, menu, mutates, needs, schema }
}

const KINDS: &[&str] = &["emitter", "hair"];
const GROOM_BRUSHES: &[&str] = &["comb", "cut", "length", "puff", "smooth"];
const PARTICLES: &[&str] = &["Object", "Particles"];

/// The settings `particle.add` and `particle.set` take (the system's fields, camelCase).
const SETTINGS: &[&str] = &[
    "kind",
    "physics",
    "seed",
    "count",
    "frameStart",
    "frameEnd",
    "lifetime",
    "lifetimeRandom",
    "emitFrom",
    "distribution",
    "normalVelocity",
    "objectVelocity",
    "randomVelocity",
    "gravity",
    "drag",
    "timestep",
    "bounce",
    "dieOnCollision",
    "renderAs",
    "size",
    "sizeRandom",
    "rotationRandom",
    "hairLength",
    "segments",
    "children",
    "childCount",
    "childRadius",
    "strandWidth",
];

/// Settings that change where hair grows: changing them drops the groom.
const REGROWS: &[&str] = &["kind", "seed", "count", "emitFrom", "distribution", "segments"];

fn settings_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("kind", s::choice("Emitter (particles that fly) or hair (strands)", KINDS)),
        ("physics", s::choice("Emitter physics: Newtonian or airborne boids with averaged rules", &["newtonian", "boids"])),
        ("boids", crate::commands_boids::schema()),
        ("seed", s::integer("Different seeds give different, each repeatable, particles (0)")),
        ("count", s::integer("Particles or parent strands (1000, at most 100000)")),
        ("frameStart", s::number("Emitter: the first frame particles are born (1)")),
        ("frameEnd", s::number("Emitter: the last frame particles are born (200)")),
        ("lifetime", s::number("Emitter: frames a particle lives (50)")),
        ("lifetimeRandom", s::number("Emitter: how much of the lifetime may randomly be cut, 0 to 1")),
        ("emitFrom", s::choice("Where particles start", &["faces", "verts", "volume"])),
        ("distribution", s::choice("How start points spread", &["jittered", "random"])),
        ("normalVelocity", s::number("Start speed along the surface normal, m/s (1)")),
        ("objectVelocity", s::vec3("Start velocity along the emitter's axes, m/s")),
        ("randomVelocity", s::number("Start speed in a random direction, m/s")),
        ("gravity", s::number("How strongly the scene's gravity pulls (1)")),
        ("drag", s::number("Air drag: the share of speed lost per second")),
        ("timestep", s::number("Seconds simulated per frame (0.04)")),
        ("colliders", s::objects("Objects particles bounce off (ids or names; an empty list for none)")),
        ("bounce", s::number("The share of speed kept on a bounce, 0 to 1 (0.5)")),
        ("dieOnCollision", s::boolean("Particles die where they hit a collider")),
        ("renderAs", s::choice("How particles show", &["halo", "object", "none"])),
        ("instanceObject", s::object("With renderAs object: the object copied at each particle (null for none)")),
        ("size", s::number("Particle size (halo diameter, instance scale), metres (0.05)")),
        ("sizeRandom", s::number("How much of the size may randomly be taken off, 0 to 1")),
        ("rotationRandom", s::number("How far instances turn at random, 0 to 1")),
        ("hairLength", s::number("Hair: strand length, metres (4)")),
        ("segments", s::integer("Hair: segments per strand (5)")),
        ("children", s::choice("Hair: child strands", &["none", "simple", "interpolated"])),
        ("childCount", s::integer("Hair: children per parent strand (10)")),
        ("childRadius", s::number("Hair: how far simple children spread round their parent, metres (0.2)")),
        ("strandWidth", s::number("Hair: strand width at the root, metres (0.01)")),
        ("densityGroup", s::string("The vertex group whose weights set how densely particles and strands start (empty: everywhere alike)")),
        ("dynamics", dynamics_schema()),
    ]
}

/// Hair dynamics' settings, as `dynamics` takes them.
const DYNAMICS: &[&str] = &["quality", "mass", "stiffness", "damping", "airDamping", "pinStiffness", "collision", "collisionDistance"];

fn dynamics_schema() -> Value {
    let properties = json!({
        "quality": s::integer("Steps per frame, 1 to 80 (5)"),
        "mass": s::number("Mass of each point in kg (0.3)"),
        "stiffness": s::number("How firmly strands bend back to their rest shape (0.5)"),
        "damping": s::number("How fast their bending settles (0.5)"),
        "airDamping": s::number("Air damping, 0 to 10 (1)"),
        "pinStiffness": s::number("How strongly points are drawn to their rest place (0)"),
        "collision": s::boolean("Strands collide with collision objects (on)"),
        "collisionDistance": s::number("How close strands come to collision objects, metres (0.015)"),
    });
    json!({
        "oneOf": [{ "type": "boolean" }, { "type": "object", "properties": properties, "additionalProperties": false }],
        "description": "Hair: dynamics simulated in the physics bake (true: on with the defaults, false: off; an object: its settings, turning it on)"
    })
}

/// Puts `dynamics` from `params` on a hair system.
fn configure_dynamics(system: &mut ParticleSystem, params: &Value) -> Result<(), String> {
    match params.get("dynamics") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(false)) => system.dynamics = None,
        Some(Value::Bool(true)) => {
            system.dynamics.get_or_insert_with(HairDynamics::default);
        }
        Some(Value::Object(given)) => {
            if let Some(unknown) = given.keys().find(|key| !DYNAMICS.contains(&key.as_str())) {
                return Err(bad(format!("`dynamics` has no setting {unknown}")));
            }
            let mut fields = serde_json::to_value(system.dynamics.clone().unwrap_or_default()).map_err(|error| error.to_string())?;
            if let Some(current) = fields.as_object_mut() {
                current.extend(given.iter().filter(|(_, value)| !value.is_null()).map(|(key, value)| (key.clone(), value.clone())));
            }
            system.dynamics = Some(serde_json::from_value(fields).map_err(|error| bad(format!("a dynamics setting is out of place: {error}")))?);
        }
        Some(_) => return Err(bad("`dynamics` is true, false, or an object of its settings")),
    }
    if system.kind != Kind::Hair && system.dynamics.is_some() {
        if params.get("dynamics").is_some_and(|value| value.as_bool() != Some(false) && !value.is_null()) {
            return Err(bad("hair dynamics are for hair systems"));
        }
        // An emitter has no strands to simulate.
        system.dynamics = None;
    }
    Ok(())
}

fn which_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("object", s::object("The mesh; the active one when left out")),
        (
            "system",
            json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "The system's index or name; the first when left out" }),
        ),
    ]
}

fn groom_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("kind", s::choice("The grooming brush", GROOM_BRUSHES)),
        ("radius", s::number("Radius in viewport pixels (50)")),
        ("strength", s::number("Strength, 0 to 1 (0.5)")),
        ("subtract", s::boolean("Length and puff the other way: shorten, flatten")),
    ]
}

static SPECS: &[Spec] = &[
    spec("particle.add", "Add Particle System", PARTICLES, true, Needs::Mesh, || {
        let mut fields = settings_fields();
        fields.push(("object", s::object("The mesh; the active one when left out")));
        fields.push(("name", s::string("The system's name (ParticleSystem)")));
        s::of(fields)
    }),
    spec("particle.set", "Particle Settings", &[], true, Needs::System, || {
        let mut fields = settings_fields();
        fields.extend(which_fields());
        fields.push(("name", s::string("A new name for the system")));
        s::of(fields)
    }),
    spec("particle.remove", "Remove Particle System", PARTICLES, true, Needs::System, || s::of(which_fields())),
    spec("particle.convert", "Convert Particles to Mesh", PARTICLES, true, Needs::System, || s::of(which_fields())),
    spec("particle.stats", "Particle Statistics", &[], false, Needs::System, || {
        let mut fields = which_fields();
        fields.push(("frame", s::number("The frame (the current one when left out)")));
        s::of(fields)
    }),
    spec("mode.particleEdit", "Particle Edit Mode", &["Object"], false, Needs::Groomable, || {
        let mut fields = which_fields();
        fields.push(("on", s::boolean("Particle Edit on or off; toggles when left out")));
        s::of(fields)
    }),
    spec("particleEdit.brush", "Grooming Brush", &[], false, Needs::Edit, || s::of(groom_fields())),
    spec("particleEdit.stroke", "Groom Stroke", &[], true, Needs::Edit, || {
        let mut fields = groom_fields();
        fields.push((
            "points",
            s::required(json!({ "type": "array", "items": { "type": "json" }, "description": "The stroke's path: [x, y] viewport pixels" })),
        ));
        s::of(fields)
    }),
    spec("particleEdit.reset", "Delete Edit", PARTICLES, true, Needs::Edit, || s::of(vec![])),
];

fn spec_of(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

fn active_mesh(doc: &SceneDocument) -> Option<Id> {
    doc.session.editor.active.filter(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let object_mode = doc.session.mode == Mode::Object
        && doc.session.rig.is_none()
        && doc.session.paint.is_none()
        && doc.session.shape.is_none()
        && doc.session.sculpt.is_none();
    let systems = || active_mesh(doc).and_then(|id| doc.scene.object(id).ok()).map(|object| object.particles.as_slice()).unwrap_or_default();
    match needs {
        Needs::Mesh => object_mode && active_mesh(doc).is_some(),
        Needs::System => object_mode && !systems().is_empty(),
        Needs::Groomable => doc.session.particle_edit.is_some() || (object_mode && systems().iter().any(|system| system.kind == Kind::Hair)),
        Needs::Edit => doc.session.particle_edit.is_some(),
        Needs::ObjectMode => object_mode,
    }
}

/// Every particle command, with whether it can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (!spec.menu.is_empty()).then(|| spec.menu.iter().map(|part| (*part).to_string()).collect()),
            params: None,
            schema: Some((spec.schema)()),
            mutates: spec.mutates,
            enabled: enabled(doc, spec.needs),
            ..Default::default()
        })
        .collect()
}

/// The particle commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Mode changes elsewhere end Particle Edit.
const ENDS_EDIT: &[&str] = &["mode.set", "mode.toggle", "mode.armature", "mode.pose", "mode.weightPaint", "mode.sculpt", "mode.shape"];

/// Runs a particle command, or answers `None` when it is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if doc.session.particle_edit.is_some() && ENDS_EDIT.contains(&command) {
        doc.session.particle_edit = None;
        doc.touched();
    }
    let spec = spec_of(command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    // A named mesh stands in for the active one, in the modes where the active one would do.
    let named = params.get("object").is_some_and(|value| !value.is_null());
    let stands_in = match spec.needs {
        Needs::Mesh | Needs::System => enabled(doc, Needs::ObjectMode),
        Needs::Groomable => true,
        Needs::Edit | Needs::ObjectMode => false,
    };
    if !(enabled(doc, spec.needs) || (named && stands_in)) {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    if !spec.mutates {
        let result = run_view(doc, command, params);
        if result.is_ok() {
            doc.touched();
        }
        return Some(result);
    }
    Some(change(doc, spec.title, |doc| run_change(doc, command, params)))
}

/// Runs a change as one undo step, rolling everything back when it fails.
fn change(doc: &mut SceneDocument, label: &str, work: impl FnOnce(&mut SceneDocument) -> Result<Value, String>) -> Result<Value, String> {
    let before = doc.step(label);
    let session = doc.session.clone();
    match work(doc) {
        Ok(value) => {
            doc.tidy_session();
            if let Err(error) = doc.scene.validate() {
                doc.restore(before);
                doc.session = session;
                return Err(format!("not-allowed: {}", error.to_string().replace("invalid-file: ", "")));
            }
            doc.committed(before)?;
            Ok(value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    }
}

/// Ends Particle Edit when its hair system is gone.
pub fn tidy(doc: &mut SceneDocument) {
    if let Some(session) = &doc.session.particle_edit
        && !doc
            .scene
            .object(session.object)
            .ok()
            .and_then(|object| object.particles.get(session.system))
            .is_some_and(|system| system.kind == Kind::Hair)
    {
        doc.session.particle_edit = None;
    }
}

/// The object a command names, or the active mesh.
fn object_of(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    let id = match params.get("object").filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value)?,
        None => active_mesh(doc).ok_or("not-allowed: select a mesh")?,
    };
    if doc.scene.object(id).ok().and_then(|object| object.mesh()).is_none() {
        return Err(bad("only a mesh has particle systems"));
    }
    Ok(id)
}

/// The object and system index a command names (the first system when it names none).
fn system_of(doc: &SceneDocument, params: &Value) -> Result<(Id, usize), String> {
    let object = object_of(doc, params)?;
    let systems = &doc.scene.object(object).map_err(|error| error.to_string())?.particles;
    let index = match params.get("system") {
        None | Some(Value::Null) => 0,
        Some(Value::Number(number)) => {
            number.as_u64().and_then(|index| usize::try_from(index).ok()).ok_or_else(|| bad("`system` is an index or a name"))?
        }
        Some(Value::String(name)) => {
            systems.iter().position(|system| system.name == *name).ok_or_else(|| format!("not-found: no particle system named {name}"))?
        }
        Some(_) => return Err(bad("`system` is an index or a name")),
    };
    if index >= systems.len() {
        return Err(format!("not-found: the object has no particle system {index}"));
    }
    Ok((object, index))
}

/// Puts the settings `params` gives on `system`; answers whether hair regrows (the groom goes).
fn configure(scene: &Scene, system: &mut ParticleSystem, params: &Value) -> Result<bool, String> {
    let mut value = serde_json::to_value(&*system).map_err(|error| error.to_string())?;
    let mut regrows = false;
    if let Some(fields) = value.as_object_mut() {
        for key in SETTINGS {
            if let Some(given) = params.get(*key).filter(|given| !given.is_null()) {
                regrows |= REGROWS.contains(key) && fields.get(*key) != Some(given);
                fields.insert((*key).to_string(), given.clone());
            }
        }
        if let Some(name) = params::string(params, "name")? {
            fields.insert("name".into(), json!(name));
        }
    }
    let groomed = std::mem::take(&mut system.groomed);
    let mut parsed: ParticleSystem = serde_json::from_value(value).map_err(|error| bad(format!("a setting is out of place: {error}")))?;
    match params.get("colliders") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) if items.is_empty() => parsed.colliders.clear(),
        Some(_) => parsed.colliders = params::objects(scene, params, "colliders")?.unwrap_or_default(),
    }
    match params.get("instanceObject") {
        None => {}
        Some(Value::Null) => parsed.instance_object = None,
        Some(value) => parsed.instance_object = Some(params::resolve(scene, value)?),
    }
    if let Some(group) = params::string(params, "densityGroup")? {
        let group = (!group.is_empty()).then(|| group.to_string());
        regrows |= parsed.density_group != group;
        parsed.density_group = group;
    }
    configure_dynamics(&mut parsed, params)?;
    crate::commands_boids::configure(scene, &mut parsed, params)?;
    let had_groom = !groomed.is_empty();
    parsed.groomed = if regrows { Vec::new() } else { groomed };
    parsed.check().map_err(bad)?;
    *system = parsed;
    Ok(regrows && had_groom)
}

fn system_json(scene: &Scene, object: Id, index: usize, system: &ParticleSystem) -> Value {
    let mut value = serde_json::to_value(system).unwrap_or(Value::Null);
    if let Some(fields) = value.as_object_mut() {
        fields.insert("object".into(), json!(object));
        fields.insert("index".into(), json!(index));
        fields.insert("groomed".into(), json!(!system.groomed.is_empty()));
        let names = |ids: &[Id]| ids.iter().filter_map(|id| scene.object(*id).ok()).map(|found| found.name.clone()).collect::<Vec<_>>();
        fields.insert("colliderNames".into(), json!(names(&system.colliders)));
    }
    value
}

/// What a system shows at `frame`: an emitter's live particles, or hair's strands.
fn stats(doc: &SceneDocument, object: Id, index: usize, frame: Option<f32>) -> Result<Value, String> {
    let scene = match frame {
        Some(frame) if frame.is_finite() => doc.scene.at_frame(frame),
        Some(_) => return Err(bad("the frame is not a number")),
        None => doc.scene.clone(),
    };
    let system = scene.object(object).ok().and_then(|found| found.particles.get(index)).ok_or("not-found: no such particle system")?;
    let round = |value: f32| (f64::from(value) * 10_000.0).round() / 10_000.0;
    let vector = |value: Vec3| json!([round(value.x), round(value.y), round(value.z)]);
    match system.kind {
        Kind::Emitter => {
            let live = particles::particles(&scene, object, index, &mut doc.particles.borrow_mut().cache);
            let count = live.len().max(1) as f32;
            let mean = live.iter().fold(Vec3::ZERO, |sum, particle| sum + particle.position) / count;
            let speed = live.iter().map(|particle| particle.velocity.length()).sum::<f32>() / count;
            Ok(json!({
                "kind": "emitter",
                "frame": particles::frame(&scene),
                "alive": live.len(),
                "meanPosition": vector(mean),
                "meanSpeed": round(speed),
            }))
        }
        Kind::Hair => {
            let local = particles::system_surface(&scene, object, system, false).ok_or("unsupported: the emitter has no surface")?;
            let parents = hair::parents(system, &local);
            let all = hair::strands(system, &local);
            let world = particles::strands(&scene, object, index, true);
            let mean = |strands: &[Vec<Vec3>]| strands.iter().map(|strand| hair::length(strand)).sum::<f32>() / strands.len().max(1) as f32;
            Ok(json!({
                "kind": "hair",
                "parents": parents.len(),
                "strands": all.len(),
                "meanLength": round(mean(&parents)),
                "meanWorldLength": round(mean(&world)),
                "groomed": !system.groomed.is_empty(),
            }))
        }
    }
}

fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "particle.stats" => {
            let (object, index) = system_of(doc, params)?;
            stats(doc, object, index, params::number(params, "frame")?)
        }
        "mode.particleEdit" => {
            let on = params::boolean(params, "on")?.unwrap_or(doc.session.particle_edit.is_none());
            if !on {
                doc.session.particle_edit = None;
                return Ok(json!({ "mode": "object" }));
            }
            let object = object_of(doc, params)?;
            let systems = &doc.scene.object(object).map_err(|error| error.to_string())?.particles;
            let index = match params.get("system").filter(|value| !value.is_null()) {
                Some(_) => system_of(doc, params)?.1,
                None => systems.iter().position(|system| system.kind == Kind::Hair).ok_or_else(|| bad("the object has no hair system"))?,
            };
            if systems.get(index).is_none_or(|system| system.kind != Kind::Hair) {
                return Err(bad("only hair can be groomed"));
            }
            doc.session.mode = Mode::Object;
            doc.session.paint = None;
            doc.session.rig = None;
            doc.session.shape = None;
            doc.session.sculpt = None;
            doc.session.editor.selected = vec![object];
            doc.session.editor.active = Some(object);
            let brush = doc.session.particle_edit.as_ref().map(|session| session.brush).unwrap_or_default();
            doc.session.particle_edit = Some(ParticleEditSession { object, system: index, brush, stroke: None });
            Ok(json!({ "mode": "particleEdit", "object": object, "system": index }))
        }
        "particleEdit.brush" => {
            let session = doc.session.particle_edit.as_mut().ok_or("not-allowed: not in Particle Edit mode")?;
            let mut brush = session.brush;
            configure_brush(&mut brush, params)?;
            session.brush = brush;
            Ok(json!({ "brush": brush }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn configure_brush(brush: &mut Groom, params: &Value) -> Result<(), String> {
    if let Some(kind) = params::string(params, "kind")? {
        brush.kind = GroomBrush::from_id(kind).ok_or_else(|| bad(format!("unknown grooming brush `{kind}`")))?;
    }
    if let Some(radius) = params::number(params, "radius")? {
        if !(1.0..=2000.0).contains(&radius) {
            return Err(bad("`radius` must be 1 to 2000 pixels"));
        }
        brush.radius = radius;
    }
    if let Some(strength) = params::number(params, "strength")? {
        if !(0.0..=1.0).contains(&strength) {
            return Err(bad("`strength` must be 0 to 1"));
        }
        brush.strength = strength;
    }
    if let Some(subtract) = params::boolean(params, "subtract")? {
        brush.subtract = subtract;
    }
    Ok(())
}

fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "particle.add" => {
            let object = object_of(doc, params)?;
            let kind = match params::string(params, "kind")? {
                Some("hair") => Kind::Hair,
                Some("emitter") | None => Kind::Emitter,
                Some(other) => return Err(bad(format!("unknown kind `{other}`"))),
            };
            let found = doc.scene.object(object).map_err(|error| error.to_string())?;
            let name = particles::unique_name(found, params::string(params, "name")?.unwrap_or("ParticleSystem"));
            let mut system = ParticleSystem::new(name.clone(), kind);
            let mut settings = params.clone();
            if let Some(fields) = settings.as_object_mut() {
                fields.remove("name");
            }
            configure(&doc.scene, &mut system, &settings)?;
            let target = doc.scene.object_mut(object).map_err(|error| error.to_string())?;
            target.particles.push(system);
            let index = target.particles.len() - 1;
            doc.session.editor.selected = vec![object];
            doc.session.editor.active = Some(object);
            Ok(json!({ "object": object, "system": index, "name": name }))
        }
        "particle.set" => {
            let (object, index) = system_of(doc, params)?;
            if let Some(name) = params::string(params, "name")? {
                let found = doc.scene.object(object).map_err(|error| error.to_string())?;
                if found.particles.iter().enumerate().any(|(other, system)| other != index && system.name == name) {
                    return Err(bad(format!("the object already has a particle system named {name}")));
                }
            }
            let scene = doc.scene.clone();
            let system =
                doc.scene.object_mut(object).ok().and_then(|found| found.particles.get_mut(index)).ok_or("not-found: no such particle system")?;
            let dropped = configure(&scene, system, params)?;
            let value = system_json(&scene, object, index, system);
            Ok(json!({ "system": value, "groomDropped": dropped }))
        }
        "particle.remove" => {
            let (object, index) = system_of(doc, params)?;
            let target = doc.scene.object_mut(object).map_err(|error| error.to_string())?;
            let removed = target.particles.remove(index);
            // Particle Edit keeps grooming the system it grooms (now one place lower when it came
            // after the removed one), and ends when that system is the one removed.
            if let Some(session) = doc.session.particle_edit.as_mut().filter(|session| session.object == object) {
                match session.system.cmp(&index) {
                    std::cmp::Ordering::Greater => session.system -= 1,
                    std::cmp::Ordering::Equal => doc.session.particle_edit = None,
                    std::cmp::Ordering::Less => {}
                }
            }
            Ok(json!({ "object": object, "removed": removed.name }))
        }
        "particle.convert" => {
            let (object, index) = system_of(doc, params)?;
            let name =
                doc.scene.object(object).ok().and_then(|found| found.particles.get(index)).map(|system| system.name.clone()).unwrap_or_default();
            // Baked as exports bake it: in world space, hair across its roots' normals.
            let mut only = doc.scene.clone();
            for other in &mut only.objects {
                if other.id != object {
                    other.particles.clear();
                } else {
                    let keep = other.particles.get(index).cloned();
                    other.particles = keep.into_iter().collect();
                }
            }
            let realized =
                particles::realize(&only, View::default(), &mut doc.particles.borrow_mut().cache).ok_or("not-found: no particle system")?;
            let made = realized
                .owners
                .keys()
                .next()
                .and_then(|id| realized.scene.object(*id).ok())
                .ok_or("not-allowed: the system shows nothing at this frame")?;
            let (data, material) = (made.data.clone(), made.material);
            let label = format!("{} {name}", doc.scene.object(object).map(|found| found.name.clone()).unwrap_or_default());
            let id = doc.scene.add_object(&label, data);
            let faces = match &doc.scene.object(id).map_err(|error| error.to_string())?.data {
                ObjectData::Mesh(mesh) => mesh.faces.len(),
                _ => 0,
            };
            if let Ok(target) = doc.scene.object_mut(id) {
                target.material = material;
            }
            doc.session.editor.selected = vec![id];
            doc.session.editor.active = Some(id);
            Ok(json!({ "object": id, "faces": faces }))
        }
        "particleEdit.stroke" => stroke_command(doc, params),
        "particleEdit.reset" => {
            let session = doc.session.particle_edit.as_ref().ok_or("not-allowed: not in Particle Edit mode")?;
            let (object, index) = (session.object, session.system);
            let system =
                doc.scene.object_mut(object).ok().and_then(|found| found.particles.get_mut(index)).ok_or("not-found: no such particle system")?;
            system.groomed.clear();
            Ok(json!({ "object": object, "system": index }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

/// One grooming dab at pixel `at` (combing by the move from `last`); answers how many strands it
/// changed.
fn dab(doc: &mut SceneDocument, projector: &Projector, brush: Groom, at: (f64, f64), last: Option<(f64, f64)>) -> Result<usize, String> {
    let session = doc.session.particle_edit.as_ref().ok_or("not-allowed: not in Particle Edit mode")?;
    let (object, index) = (session.object, session.system);
    let world = doc.scene.world_matrix(object).map_err(|error| error.to_string())?;
    let system = doc.scene.object(object).ok().and_then(|found| found.particles.get(index)).ok_or("not-found: no such particle system")?;
    let local = particles::system_surface(&doc.scene, object, system, false).ok_or("unsupported: the emitter has no surface")?;
    let mut strands = hair::parents(system, &local);
    let normals: Vec<Vec3> = hair::roots(system, &local).into_iter().map(|start| start.normal).collect();
    let view_projection = projector.view_projection();
    let to_screen = |point: Vec3| {
        let clip = view_projection * world.transform_point3(point).extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let screen = projector.to_screen(clip);
        Some((screen.x, screen.y))
    };
    // The pointer's move as a world move at the object's depth, then in its own space.
    let drag = match last {
        Some(last) => {
            let inverse_view = projector.view.inverse();
            let pixel = projector.pixel_size_at(world.transform_point3(Vec3::ZERO));
            let (dx, dy) = ((at.0 - last.0) as f32, (at.1 - last.1) as f32);
            let moved = (inverse_view.x_axis.xyz() * dx - inverse_view.y_axis.xyz() * dy) * pixel;
            world.inverse().transform_vector3(moved)
        }
        None => Vec3::ZERO,
    };
    if brush.kind == GroomBrush::Comb && drag == Vec3::ZERO {
        return Ok(0);
    }
    let before = strands.clone();
    let touched = hair::groom(
        &mut strands,
        &GroomDab {
            brush: brush.kind,
            strength: brush.strength,
            radius: brush.radius,
            center: (at.0 as f32, at.1 as f32),
            drag,
            subtract: brush.subtract,
            to_screen: &to_screen,
            normals: &normals,
        },
    );
    if strands != before
        && let Some(system) = doc.scene.object_mut(object).ok().and_then(|found| found.particles.get_mut(index))
    {
        system.groomed = strands;
    }
    Ok(touched)
}

fn projector(doc: &SceneDocument) -> Projector {
    doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height)
}

/// `particleEdit.stroke`: a whole stroke through viewport pixels, as one undo step.
fn stroke_command(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let mut brush = doc.session.particle_edit.as_ref().map(|session| session.brush).ok_or("not-allowed: not in Particle Edit mode")?;
    configure_brush(&mut brush, params)?;
    let points = params.get("points").and_then(Value::as_array).ok_or_else(|| bad("`points` is a list of [x, y]"))?;
    if points.is_empty() || points.len() > 10_000 {
        return Err(bad("a stroke has 1 to 10000 points"));
    }
    let projector = projector(doc);
    let mut last = None;
    let mut touched = 0;
    for point in points {
        let numbers: Vec<f64> = point.as_array().map(|items| items.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        let [x, y] = numbers.as_slice() else { return Err(bad("each point is [x, y] pixels")) };
        if !(x.is_finite() && y.is_finite()) {
            return Err(bad("each point is [x, y] pixels"));
        }
        touched += dab(doc, &projector, brush, (*x, *y), last)?;
        last = Some((*x, *y));
    }
    let session = doc.session.particle_edit.as_ref().ok_or("not-allowed: not in Particle Edit mode")?;
    Ok(json!({ "object": session.object, "system": session.system, "touched": touched }))
}

/// The pointer in Particle Edit mode: every tool grooms. Down starts a stroke (one undo step),
/// Drag grooms along the path, Up commits, Move shows the brush.
pub fn pointer(doc: &mut SceneDocument, event: &ToolPointer) -> Option<ToolResponse> {
    let session = doc.session.particle_edit.clone()?;
    let at = (event.x, event.y);
    let subtract = event.mods.command;
    let circle = |busy: bool| {
        let radius = f64::from(session.brush.radius);
        let d = format!("M {} {} a {radius} {radius} 0 1 0 {} 0 a {radius} {radius} 0 1 0 {} 0", at.0 - radius, at.1, radius * 2.0, -radius * 2.0);
        ToolResponse {
            overlays: vec![ToolOverlay::Path { d, color: [255, 255, 255, 230], width: 1.5, dashed: subtract }],
            cursor: "crosshair".into(),
            busy,
            ..Default::default()
        }
    };
    let mut brush = session.brush;
    // Cmd held flips length and puff for this stroke.
    brush.subtract ^= subtract;
    match event.action {
        PointerAction::Down => {
            let before = Box::new(doc.step("Groom"));
            let projector = projector(doc);
            let _ = dab(doc, &projector, brush, at, None);
            doc.touched();
            if let Some(current) = doc.session.particle_edit.as_mut() {
                current.stroke = Some(Live { before, projector, last: at });
            }
            Some(circle(true))
        }
        PointerAction::Drag => {
            let mut live = doc.session.particle_edit.as_mut().and_then(|current| current.stroke.take())?;
            if (at.0 - live.last.0).hypot(at.1 - live.last.1) >= 2.0 {
                if dab(doc, &live.projector, brush, at, Some(live.last)).unwrap_or(0) > 0 {
                    doc.touched();
                }
                live.last = at;
            }
            if let Some(current) = doc.session.particle_edit.as_mut() {
                current.stroke = Some(live);
            }
            Some(circle(true))
        }
        PointerAction::Up => {
            let live = doc.session.particle_edit.as_mut().and_then(|current| current.stroke.take())?;
            if doc.scene != live.before.scene {
                if doc.scene.validate().is_ok() {
                    // One transaction per stroke.
                    let _ = doc.committed(*live.before);
                } else {
                    let keep = doc.session.particle_edit.clone();
                    doc.restore(*live.before);
                    doc.session.particle_edit = keep;
                    doc.touched();
                }
            }
            Some(circle(false))
        }
        PointerAction::Move | PointerAction::DoubleClick => Some(circle(false)),
    }
}

/// Escape during a stroke takes it back.
pub fn cancel(doc: &mut SceneDocument) -> bool {
    let Some(live) = doc.session.particle_edit.as_mut().and_then(|current| current.stroke.take()) else { return false };
    let keep = doc.session.particle_edit.clone();
    doc.restore(*live.before);
    doc.session.particle_edit = keep;
    doc.touched();
    true
}

/// `scene` with its particles as meshes for a picture from `view`, or `None` when it has no
/// particle systems; remembers which emitter each added object is, for picking.
pub fn realized(doc: &SceneDocument, scene: &Scene, view: View) -> Option<Scene> {
    if !particles::any(scene) {
        return None;
    }
    let mut derived = doc.particles.borrow_mut();
    let realized = particles::realize(scene, view, &mut derived.cache)?;
    derived.owners = realized.owners;
    Some(realized.scene)
}

/// The view a viewport picture is drawn from.
pub fn viewport_view(projector: &Projector) -> View {
    let eye = projector.eye();
    let forward = projector.view.inverse().transform_vector3(-Vec3::Z).normalize_or_zero();
    if projector.perspective {
        View { eye: Some(eye), pixel: projector.pixel_size_at(eye + forward), orthographic: None }
    } else {
        View { eye: Some(eye), pixel: projector.pixel_size_at(eye + forward), orthographic: Some(-forward) }
    }
}

/// The view a camera render is drawn from: the camera's place, true sizes.
pub fn camera_view(scene: &Scene) -> View {
    let eye = scene.camera.and_then(|camera| scene.world_matrix(camera).ok()).map(|matrix| matrix.transform_point3(Vec3::ZERO));
    View { eye, ..View::default() }
}

/// The scene the viewport draws: with particles for the current view.
pub fn viewport_scene(doc: &SceneDocument) -> Option<Scene> {
    realized(doc, &doc.scene, viewport_view(&projector(doc)))
}

/// `scene` (the document's at some frame) as a camera render sees it.
pub fn render_scene(doc: &SceneDocument, scene: Scene) -> Scene {
    let view = camera_view(&scene);
    realized(doc, &scene, view).unwrap_or(scene)
}

/// `doc`'s scene as exports write it (hair baked to ribbons across their roots' normals), with
/// `only` widened to the particles of the objects it names.
pub fn export_scene(doc: &SceneDocument, only: Option<Vec<Id>>) -> (Scene, Option<Vec<Id>>) {
    match realized(doc, &doc.scene, View::default()) {
        Some(scene) => {
            let owners = doc.particles.borrow().owners.clone();
            let only = only.map(|mut ids| {
                ids.extend(owners.iter().filter(|(_, owner)| ids.contains(owner)).map(|(id, _)| *id).collect::<Vec<_>>());
                ids
            });
            (scene, only)
        }
        None => (doc.scene.clone(), only),
    }
}

/// The objects picked in a picture, each particle object as its emitter.
pub fn owners(doc: &SceneDocument, picked: Vec<Id>) -> Vec<Id> {
    let derived = doc.particles.borrow();
    let mut out: Vec<Id> = Vec::with_capacity(picked.len());
    for id in picked {
        let id = derived.owners.get(&id).copied().unwrap_or(id);
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Adds particle systems and Particle Edit mode to `inspect()`.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let systems: Vec<Value> = doc
        .scene
        .objects
        .iter()
        .flat_map(|object| object.particles.iter().enumerate().map(|(index, system)| system_json(&doc.scene, object.id, index, system)))
        .collect();
    if let Some(fields) = inspected.as_object_mut() {
        fields.insert("particles".into(), Value::Array(systems));
    }
    let (Some(session), Some(mode)) = (&doc.session.particle_edit, inspected.get_mut("mode").and_then(Value::as_object_mut)) else { return };
    mode.insert("mode".into(), json!("particleEdit"));
    mode.insert("object".into(), json!(session.object));
    mode.insert("particleEdit".into(), json!({ "system": session.system, "brush": session.brush, "stroking": session.stroke.is_some() }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_well_formed() {
        for spec in SPECS {
            kition_kit::schema::validate(&(spec.schema)()).unwrap_or_else(|error| panic!("{}: {error}", spec.id));
        }
    }
}
