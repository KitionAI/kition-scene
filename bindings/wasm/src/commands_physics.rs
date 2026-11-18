//! Physics (Physics panel; Scene ▸ Physics): the rigid body world, each object's rigid body,
//! cloth, soft body, and collision settings, and the bake (which also runs dynamic hair). Settings changes are one undo step each and are
//! stored in the replica; the bake is the session's (derived data, never saved: each replica
//! bakes its own, and the solver is deterministic). A change to anything a bake depends on makes
//! it out of date: it stops showing, and a new bake starts that the editor steps
//! (`physics.bakeStep`) or `physics.bake` finishes.

use std::sync::{Arc, Mutex};

use glam::Vec3;
use kition_kit::KitCommand;
use kition_scene_model::physics::{
    BodyKind, Cloth, Collision, CollisionShape, FieldKind, ForceField, JointKind, ObjectPhysics, PhysicsWorld, RigidBody, RigidConstraint, SoftBody,
};
use kition_scene_model::{Id, ObjectData, Scene};
use kition_scene_physics::Bake;
use serde_json::{Value, json};

use crate::SceneDocument;
use crate::params::{self, bad, schema as s};

/// The bake this session shows and the one under way.
#[derive(Clone, Default)]
pub struct PhysicsSession {
    /// A failed derived bake stops following edits until Bake is requested again.
    failure: Option<String>,
    /// The latest bake (shown while it matches the scene).
    pub baked: Option<Arc<kition_scene_model::physics::Baked>>,
    /// A bake under way.
    running: Option<Arc<Mutex<Bake>>>,
    /// Someone baked: out-of-date bakes start again by themselves until the bake is freed.
    pub follow: bool,
}

impl std::fmt::Debug for PhysicsSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PhysicsSession")
            .field("frames", &self.baked.as_ref().map(|baked| baked.frames))
            .field("running", &self.running.is_some())
            .field("follow", &self.follow)
            .finish()
    }
}

/// Frames one `physics.bakeStep` runs when it is not told.
const STEP_FRAMES: u32 = 4;

struct Spec {
    id: &'static str,
    title: &'static str,
    mutates: bool,
    schema: fn() -> Value,
}

fn objects() -> (&'static str, Value) {
    ("objects", s::objects("The objects; the selection when left out"))
}

fn world_schema() -> Value {
    s::of(vec![
        ("gravity", s::vec3("Gravity in m/s² (default 0, 0, -9.81)")),
        ("substeps", s::integer("Simulation steps per frame (1 to 100)")),
        ("iterations", s::integer("Solver iterations per step (1 to 100)")),
        ("timeScale", s::number("How fast simulated time runs (0.01 to 100)")),
        ("frameStart", s::integer("The first simulated frame")),
        ("frameEnd", s::integer("The last simulated frame")),
    ])
}

fn rigid_schema() -> Value {
    s::of(vec![
        objects(),
        ("type", s::choice("Active bodies move; passive ones only collide", &["active", "passive"])),
        ("enabled", s::boolean("Simulated (an active body that is not stays put)")),
        ("animated", s::boolean("A passive body moved by its animation, pushing active ones")),
        ("mass", s::number("Mass in kg")),
        ("friction", s::number("Friction, 0 to 10")),
        ("restitution", s::number("Bounciness, 0 to 1")),
        ("linearDamping", s::number("Linear damping, 0 to 1")),
        ("angularDamping", s::number("Angular damping, 0 to 1")),
        ("shape", s::choice("The collision shape (mesh: passive bodies only)", &["box", "sphere", "capsule", "cylinder", "convexHull", "mesh"])),
        ("margin", s::number("Collision margin in metres")),
        ("deactivation", s::boolean("The body may fall asleep when it barely moves (and wakes when something hits it)")),
        ("startDeactivated", s::boolean("The body starts asleep, hanging where it is until something hits it")),
        ("deactivateLinear", s::number("Below this speed (m/s) the body may fall asleep")),
        ("deactivateAngular", s::number("Below this spin (rad/s) the body may fall asleep")),
    ])
}

fn cloth_schema() -> Value {
    s::of(vec![
        objects(),
        ("effectorWeights", crate::commands_field_weights::schema()),
        ("quality", s::integer("Steps per frame (1 to 80)")),
        ("mass", s::number("Mass of each vertex in kg")),
        ("airDamping", s::number("Air damping")),
        ("tension", s::number("Stretch stiffness")),
        ("compression", s::number("Compression stiffness")),
        ("shear", s::number("Shear stiffness")),
        ("bending", s::number("Bending stiffness")),
        ("pinGroup", s::string("The vertex group that stays put (empty: none)")),
        ("collisionDistance", s::number("How close the cloth comes to collision objects, in metres")),
        ("selfCollision", s::boolean("The cloth collides with itself")),
        ("selfDistance", s::number("How far apart the cloth keeps from itself, in metres")),
    ])
}

fn soft_schema() -> Value {
    s::of(vec![
        objects(),
        ("effectorWeights", crate::commands_field_weights::schema()),
        ("mass", s::number("Mass of each vertex in kg (1)")),
        ("friction", s::number("Air friction (0.5)")),
        ("speed", s::number("How fast simulated time runs (1)")),
        ("goal", s::boolean("Vertices are drawn to where the mesh puts them (on)")),
        ("goalGroup", s::string("The vertex group whose weights scale the goal (empty: every vertex alike)")),
        ("goalDefault", s::number("The goal weight, 0 to 1; 1 follows the mesh exactly (0.7)")),
        ("goalMin", s::number("The goal weight at group weight 0, 0 to 1 (0)")),
        ("goalMax", s::number("The goal weight at group weight 1, 0 to 1 (1)")),
        ("goalStiffness", s::number("Goal spring stiffness, 0 to 0.999 (0.5)")),
        ("goalDamping", s::number("Goal spring damping (0)")),
        ("edges", s::boolean("Springs along the mesh's edges (on)")),
        ("pull", s::number("Edge stiffness when stretched, 0 to 0.999 (0.5)")),
        ("push", s::number("Edge stiffness when squeezed, 0 to 0.999 (0.5)")),
        ("damping", s::number("Edge damping (0.5)")),
        ("bending", s::number("Springs between vertices two edges apart, 0 to 10 (0: none)")),
        ("selfCollision", s::boolean("Vertices keep apart from each other (off)")),
        ("ballSize", s::number("Self collision: each vertex's ball as a share of its edges' mean length (0.49)")),
        ("frameStart", s::integer("The first simulated frame (1); before it the mesh is at rest")),
        ("frameEnd", s::integer("The last simulated frame (250); after it the body holds")),
    ])
}

const FIELD_KINDS: &[&str] = &["force", "wind", "vortex", "turbulence", "drag"];
const JOINT_KINDS: &[&str] = &["fixed", "point", "hinge", "slider", "genericSpring", "motor"];

fn field_fields() -> Vec<(&'static str, Value)> {
    vec![
        (
            "type",
            s::choice(
                "force: away from the origin (towards it when negative); wind: along the object's Z; vortex: round its Z; turbulence: noise; drag: slows things",
                FIELD_KINDS,
            ),
        ),
        ("strength", s::number("How hard it pushes (m/s² on a particle at full strength; drag ignores it)")),
        ("flow", s::number("Things are carried along: they also slow by this share of their speed each second")),
        ("falloffPower", s::number("Strength falls off as (1 + distance)^-power (0: no falloff)")),
        ("maxDistance", s::number("Beyond this distance it does nothing (0 or less: no limit)")),
        ("minDistance", s::number("Within this distance it is at full strength (0 or less: none)")),
        ("size", s::number("Turbulence: the size of its swirls in metres")),
        ("seed", s::integer("Turbulence: different seeds give different noise")),
        ("linearDrag", s::number("Drag: slowing in proportion to speed")),
        ("quadraticDrag", s::number("Drag: slowing in proportion to speed squared")),
    ]
}

fn field_schema() -> Value {
    let mut fields = vec![objects()];
    fields.extend(field_fields());
    s::of(fields)
}

fn field_new_schema() -> Value {
    let mut fields = field_fields();
    fields.push(("location", s::vec3("Where to put it, in metres (the origin when left out)")));
    fields.push(("rotation", s::vec3("Rotation in degrees, XYZ Euler (wind and vortex act along its Z)")));
    fields.push(("name", s::string("The new empty's name")));
    s::of(fields)
}

fn constraint_fields() -> Vec<(&'static str, Value)> {
    vec![
        (
            "type",
            s::choice(
                "fixed: one piece; point: a ball joint; hinge: turns about Z; slider: slides along X; genericSpring: per axis limits and springs; motor: drives along/about X",
                JOINT_KINDS,
            ),
        ),
        ("object1", s::object("The first rigid body joined")),
        ("object2", s::object("The second rigid body joined")),
        ("enabled", s::boolean("The constraint acts")),
        ("disableCollisions", s::boolean("The joined bodies pass through each other (default true)")),
        ("breakable", s::boolean("The constraint breaks when it must push harder than the threshold")),
        ("breakingThreshold", s::number("The impulse (N·s) in one step that breaks it")),
        ("angleLimit", serde_json::json!({"type": "array", "description": "Hinge: [lower, upper] in degrees about Z, or null for none"})),
        ("distanceLimit", serde_json::json!({"type": "array", "description": "Slider: [lower, upper] in metres along X, or null for none"})),
        (
            "linearLimits",
            serde_json::json!({"type": "array", "description": "Generic spring: per axis X, Y, Z a [lower, upper] in metres, or null for free"}),
        ),
        (
            "angularLimits",
            serde_json::json!({"type": "array", "description": "Generic spring: per axis X, Y, Z a [lower, upper] in degrees, or null for free"}),
        ),
        (
            "linearSprings",
            serde_json::json!({"type": "array", "description": "Generic spring: per axis X, Y, Z a stiffness in N/m, or null for none"}),
        ),
        (
            "angularSprings",
            serde_json::json!({"type": "array", "description": "Generic spring: per axis X, Y, Z a stiffness in N·m/rad, or null for none"}),
        ),
        ("springDamping", s::number("Generic spring: the springs' damping")),
        ("linearMotor", s::boolean("Motor: enable translation along the constraint's X axis (default false)")),
        ("angularMotor", s::boolean("Motor: enable rotation about the constraint's X axis (default false)")),
        ("linearTargetVelocity", s::number("Motor: body 1 relative to body 2 in m/s, -100000 to 100000 (default 1)")),
        ("angularTargetVelocity", s::number("Motor: body 1 relative to body 2 in rad/s, -100000 to 100000 (default 1)")),
        ("linearMaxImpulse", s::number("Motor: maximum linear impulse control, 0 to 1000000000 (default 1); step impulse is value times seconds")),
        ("angularMaxImpulse", s::number("Motor: maximum angular impulse control, 0 to 1000000000 (default 1); step impulse is value times seconds")),
    ]
}

fn constraint_schema() -> Value {
    let mut fields = vec![objects()];
    fields.extend(constraint_fields());
    s::of(fields)
}

fn connect_schema() -> Value {
    s::of(vec![
        ("objects", s::objects("Rigid bodies to join one to the next, in order; the selection when left out")),
        ("type", s::choice("The kind of constraint (default fixed)", JOINT_KINDS)),
    ])
}

fn collision_schema() -> Value {
    s::of(vec![
        objects(),
        ("thickness", s::number("How far outside the surface cloth and soft bodies stay, in metres")),
        ("friction", s::number("Friction against cloth and soft bodies")),
        ("damping", s::number("How much of a soft body's bounce it takes, 0 to 1 (0.1)")),
    ])
}

fn targets_schema() -> Value {
    s::of(vec![objects()])
}

fn bake_schema() -> Value {
    s::of(vec![("wait", s::boolean("Bake every frame now (default); false starts a bake that physics.bakeStep runs"))])
}

fn step_schema() -> Value {
    s::of(vec![("frames", s::integer("Frames to bake now (default 4)"))])
}

fn none() -> Value {
    s::of(vec![])
}

const SPECS: &[Spec] = &[
    Spec { id: "physics.world.set", title: "Physics World", mutates: true, schema: world_schema },
    Spec { id: "physics.rigidBody.add", title: "Add Rigid Body", mutates: true, schema: rigid_schema },
    Spec { id: "physics.rigidBody.set", title: "Rigid Body Settings", mutates: true, schema: rigid_schema },
    Spec { id: "physics.rigidBody.remove", title: "Remove Rigid Body", mutates: true, schema: targets_schema },
    Spec { id: "physics.cloth.add", title: "Add Cloth", mutates: true, schema: cloth_schema },
    Spec { id: "physics.cloth.set", title: "Cloth Settings", mutates: true, schema: cloth_schema },
    Spec { id: "physics.cloth.remove", title: "Remove Cloth", mutates: true, schema: targets_schema },
    Spec { id: "physics.softBody.add", title: "Add Soft Body", mutates: true, schema: soft_schema },
    Spec { id: "physics.softBody.set", title: "Soft Body Settings", mutates: true, schema: soft_schema },
    Spec { id: "physics.softBody.remove", title: "Remove Soft Body", mutates: true, schema: targets_schema },
    Spec { id: "physics.collision.add", title: "Add Collision", mutates: true, schema: collision_schema },
    Spec { id: "physics.collision.remove", title: "Remove Collision", mutates: true, schema: targets_schema },
    Spec { id: "physics.field.new", title: "New Force Field", mutates: true, schema: field_new_schema },
    Spec { id: "physics.field.add", title: "Add Force Field", mutates: true, schema: field_schema },
    Spec { id: "physics.field.set", title: "Force Field Settings", mutates: true, schema: field_schema },
    Spec { id: "physics.field.remove", title: "Remove Force Field", mutates: true, schema: targets_schema },
    Spec { id: "physics.constraint.add", title: "Add Rigid Body Constraint", mutates: true, schema: constraint_schema },
    Spec { id: "physics.constraint.set", title: "Rigid Body Constraint Settings", mutates: true, schema: constraint_schema },
    Spec { id: "physics.constraint.remove", title: "Remove Rigid Body Constraint", mutates: true, schema: targets_schema },
    Spec { id: "physics.constraint.connect", title: "Connect Rigid Bodies", mutates: true, schema: connect_schema },
    Spec { id: "physics.bake", title: "Bake Physics", mutates: false, schema: bake_schema },
    Spec { id: "physics.bakeStep", title: "Bake Physics Step", mutates: false, schema: step_schema },
    Spec { id: "physics.free", title: "Free Physics Bake", mutates: false, schema: none },
];

fn targets(doc: &SceneDocument) -> Vec<Id> {
    doc.session.editor.selected.clone()
}

fn enabled(doc: &SceneDocument, id: &str) -> bool {
    match id {
        "physics.world.set" | "physics.field.new" => true,
        "physics.constraint.connect" => {
            targets(doc)
                .iter()
                .filter(|id| {
                    doc.scene.object(**id).ok().and_then(|object| object.physics.as_ref()).is_some_and(|physics| physics.rigid_body.is_some())
                })
                .count()
                >= 2
        }
        "physics.bake" => doc.scene.simulated_objects().next().is_some(),
        "physics.bakeStep" => doc.session.physics.running.is_some(),
        "physics.free" => doc.session.physics.baked.is_some() || doc.session.physics.running.is_some() || doc.session.physics.failure.is_some(),
        _ => !targets(doc).is_empty(),
    }
}

/// The commands this file adds.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: Some(vec!["Object".into(), "Physics".into()]),
            shortcut: None,
            params: None,
            schema: Some((spec.schema)()),
            mutates: spec.mutates,
            enabled: enabled(doc, spec.id),
            ..Default::default()
        })
        .collect()
}

/// The ones that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.id)).map(|spec| spec.id.to_string()).collect()
}

/// Runs `command` if it is one of these; `None` for every other command.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = SPECS.iter().find(|spec| spec.id == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    if !spec.mutates {
        let result = match command {
            "physics.bake" => bake(doc, params),
            "physics.bakeStep" => bake_step(doc, params),
            _ => {
                doc.session.physics = PhysicsSession::default();
                tidy(doc);
                Ok(json!({ "state": "none" }))
            }
        };
        doc.touched();
        return Some(result);
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    let result = change(doc, command, params).and_then(|value| {
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

fn chosen(doc: &SceneDocument, params: &Value) -> Result<Vec<Id>, String> {
    let ids = params::objects(&doc.scene, params, "objects")?.unwrap_or_else(|| targets(doc));
    if ids.is_empty() {
        return Err(bad("select an object or name `objects`"));
    }
    Ok(ids)
}

fn physics_of(scene: &mut Scene, id: Id) -> Result<&mut ObjectPhysics, String> {
    let object = scene.object_mut(id).map_err(|error| format!("not-found: {error}"))?;
    Ok(object.physics.get_or_insert_with(ObjectPhysics::default))
}

/// Drops empty physics (an object with none has none).
fn prune(scene: &mut Scene) {
    for object in &mut scene.objects {
        if object.physics.as_ref().is_some_and(ObjectPhysics::is_empty) {
            object.physics = None;
        }
    }
}

fn change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    if command == "physics.world.set" {
        set_world(&mut doc.scene.physics, params)?;
        return Ok(world_json(&doc.scene.physics));
    }
    if command == "physics.field.new" {
        return new_field(doc, params);
    }
    if command == "physics.constraint.connect" {
        return connect(doc, params);
    }
    let ids = chosen(doc, params)?;
    let ends = constraint_ends(&doc.scene, params)?;
    for id in &ids {
        let has_geometry = doc.scene.object(*id).map_err(|error| format!("not-found: {error}"))?.mesh().is_some();
        let physics = physics_of(&mut doc.scene, *id)?;
        match command {
            "physics.rigidBody.add" | "physics.rigidBody.set" => {
                if !has_geometry {
                    return Err(bad("rigid bodies are meshes"));
                }
                let body = match (&mut physics.rigid_body, command) {
                    (Some(body), _) => body,
                    (None, "physics.rigidBody.add") => physics.rigid_body.insert(RigidBody::default()),
                    (None, _) => return Err(format!("not-found: object {id} has no rigid body")),
                };
                set_rigid(body, params)?;
            }
            "physics.cloth.add" | "physics.cloth.set" => {
                if !has_geometry {
                    return Err(bad("cloth is a mesh"));
                }
                if physics.soft_body.is_some() {
                    return Err(bad("the object is a soft body: remove it first"));
                }
                let cloth = match (&mut physics.cloth, command) {
                    (Some(cloth), _) => cloth,
                    (None, "physics.cloth.add") => physics.cloth.insert(Cloth::default()),
                    (None, _) => return Err(format!("not-found: object {id} has no cloth")),
                };
                set_cloth(cloth, params)?;
            }
            "physics.softBody.add" | "physics.softBody.set" => {
                if !has_geometry {
                    return Err(bad("a soft body is a mesh"));
                }
                if physics.cloth.is_some() {
                    return Err(bad("the object is cloth: remove it first"));
                }
                let soft = match (&mut physics.soft_body, command) {
                    (Some(soft), _) => soft,
                    (None, "physics.softBody.add") => physics.soft_body.insert(SoftBody::default()),
                    (None, _) => return Err(format!("not-found: object {id} has no soft body")),
                };
                set_soft(soft, params)?;
            }
            "physics.collision.add" => {
                if !has_geometry {
                    return Err(bad("collision objects are meshes"));
                }
                let collision = physics.collision.get_or_insert_with(Collision::default);
                if let Some(thickness) = params::number(params, "thickness")? {
                    collision.thickness = thickness;
                }
                if let Some(friction) = params::number(params, "friction")? {
                    collision.friction = friction;
                }
                if let Some(damping) = params::number(params, "damping")? {
                    collision.damping = damping;
                }
            }
            "physics.field.add" | "physics.field.set" => {
                let field = match (&mut physics.field, command) {
                    (Some(field), _) => field,
                    (None, "physics.field.add") => physics.field.insert(ForceField::new(field_kind(params)?.unwrap_or_default())),
                    (None, _) => return Err(format!("not-found: object {id} has no force field")),
                };
                set_field(field, params)?;
            }
            "physics.constraint.add" | "physics.constraint.set" => {
                let constraint = match (&mut physics.rigid_constraint, command) {
                    (Some(constraint), _) => constraint,
                    (None, "physics.constraint.add") => physics.rigid_constraint.insert(RigidConstraint::default()),
                    (None, _) => return Err(format!("not-found: object {id} has no rigid body constraint")),
                };
                set_constraint(constraint, params, ends, *id)?;
            }
            "physics.field.remove" => physics.field = None,
            "physics.constraint.remove" => physics.rigid_constraint = None,
            "physics.rigidBody.remove" => physics.rigid_body = None,
            "physics.cloth.remove" => physics.cloth = None,
            "physics.softBody.remove" => physics.soft_body = None,
            "physics.collision.remove" => physics.collision = None,
            _ => return Err(format!("unknown-command: {command}")),
        }
        physics.check().map_err(|error| bad(error.to_string()))?;
    }
    prune(&mut doc.scene);
    let objects: Vec<Value> = ids.iter().map(|id| object_json(&doc.scene, *id)).collect();
    Ok(json!({ "objects": objects }))
}

fn set_world(world: &mut PhysicsWorld, params: &Value) -> Result<(), String> {
    if let Some(gravity) = params::vec3(params, "gravity")? {
        world.gravity = gravity;
    }
    if let Some(substeps) = params::integer(params, "substeps")? {
        world.substeps = substeps;
    }
    if let Some(iterations) = params::integer(params, "iterations")? {
        world.iterations = iterations;
    }
    if let Some(scale) = params::number(params, "timeScale")? {
        world.time_scale = scale;
    }
    let frame = |key: &str| -> Result<Option<i32>, String> {
        match params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => {
                value.as_i64().and_then(|frame| i32::try_from(frame).ok()).map(Some).ok_or_else(|| bad(format!("`{key}` is a whole number")))
            }
        }
    };
    if let Some(start) = frame("frameStart")? {
        world.frame_start = start;
    }
    if let Some(end) = frame("frameEnd")? {
        world.frame_end = end;
    }
    world.check().map_err(|error| bad(error.to_string()))
}

fn set_rigid(body: &mut RigidBody, params: &Value) -> Result<(), String> {
    match params::string(params, "type")? {
        Some("active") => body.kind = BodyKind::Active,
        Some("passive") => body.kind = BodyKind::Passive,
        Some(other) => return Err(bad(format!("`type` is active or passive, not {other}"))),
        None => {}
    }
    if let Some(shape) = params::string(params, "shape")? {
        body.shape = serde_json::from_value::<CollisionShape>(json!(shape)).map_err(|_| bad(format!("no collision shape {shape}")))?;
    }
    if let Some(enabled) = params::boolean(params, "enabled")? {
        body.enabled = enabled;
    }
    if let Some(animated) = params::boolean(params, "animated")? {
        body.animated = animated;
    }
    if let Some(deactivation) = params::boolean(params, "deactivation")? {
        body.deactivation = deactivation;
    }
    if let Some(asleep) = params::boolean(params, "startDeactivated")? {
        body.start_deactivated = asleep;
    }
    for (key, slot) in [
        ("mass", &mut body.mass),
        ("friction", &mut body.friction),
        ("restitution", &mut body.restitution),
        ("linearDamping", &mut body.linear_damping),
        ("angularDamping", &mut body.angular_damping),
        ("margin", &mut body.margin),
        ("deactivateLinear", &mut body.deactivate_linear),
        ("deactivateAngular", &mut body.deactivate_angular),
    ] {
        if let Some(value) = params::number(params, key)? {
            *slot = value;
        }
    }
    Ok(())
}

fn set_cloth(cloth: &mut Cloth, params: &Value) -> Result<(), String> {
    crate::commands_field_weights::configure(&mut cloth.effector_weights, params)?;
    if let Some(quality) = params::integer(params, "quality")? {
        cloth.quality = quality;
    }
    if let Some(group) = params::string(params, "pinGroup")? {
        cloth.pin_group = (!group.is_empty()).then(|| group.to_string());
    }
    if let Some(on) = params::boolean(params, "selfCollision")? {
        cloth.self_collision = on;
    }
    for (key, slot) in [
        ("mass", &mut cloth.mass),
        ("airDamping", &mut cloth.air_damping),
        ("tension", &mut cloth.tension),
        ("compression", &mut cloth.compression),
        ("shear", &mut cloth.shear),
        ("bending", &mut cloth.bending),
        ("collisionDistance", &mut cloth.collision_distance),
        ("selfDistance", &mut cloth.self_distance),
    ] {
        if let Some(value) = params::number(params, key)? {
            *slot = value;
        }
    }
    Ok(())
}

fn set_soft(soft: &mut SoftBody, params: &Value) -> Result<(), String> {
    crate::commands_field_weights::configure(&mut soft.effector_weights, params)?;
    if let Some(group) = params::string(params, "goalGroup")? {
        soft.goal_group = (!group.is_empty()).then(|| group.to_string());
    }
    for (key, slot) in [("goal", &mut soft.goal), ("edges", &mut soft.edges), ("selfCollision", &mut soft.self_collision)] {
        if let Some(on) = params::boolean(params, key)? {
            *slot = on;
        }
    }
    for (key, slot) in [
        ("mass", &mut soft.mass),
        ("friction", &mut soft.friction),
        ("speed", &mut soft.speed),
        ("goalDefault", &mut soft.goal_default),
        ("goalMin", &mut soft.goal_min),
        ("goalMax", &mut soft.goal_max),
        ("goalStiffness", &mut soft.goal_stiffness),
        ("goalDamping", &mut soft.goal_damping),
        ("pull", &mut soft.pull),
        ("push", &mut soft.push),
        ("damping", &mut soft.damping),
        ("bending", &mut soft.bending),
        ("ballSize", &mut soft.ball_size),
    ] {
        if let Some(value) = params::number(params, key)? {
            *slot = value;
        }
    }
    for (key, slot) in [("frameStart", &mut soft.frame_start), ("frameEnd", &mut soft.frame_end)] {
        match params.get(key) {
            None | Some(Value::Null) => {}
            Some(value) => {
                *slot = value.as_i64().and_then(|frame| i32::try_from(frame).ok()).ok_or_else(|| bad(format!("`{key}` is a whole number")))?;
            }
        }
    }
    Ok(())
}

fn field_kind(params: &Value) -> Result<Option<FieldKind>, String> {
    match params::string(params, "type")? {
        None => Ok(None),
        Some(name) => FieldKind::ALL.into_iter().find(|kind| kind.name() == name).map(Some).ok_or_else(|| bad(format!("no force field type {name}"))),
    }
}

fn set_field(field: &mut ForceField, params: &Value) -> Result<(), String> {
    if let Some(kind) = field_kind(params)? {
        field.kind = kind;
    }
    for (key, slot) in [
        ("strength", &mut field.strength),
        ("flow", &mut field.flow),
        ("falloffPower", &mut field.falloff_power),
        ("size", &mut field.size),
        ("linearDrag", &mut field.linear_drag),
        ("quadraticDrag", &mut field.quadratic_drag),
    ] {
        if let Some(value) = params::number(params, key)? {
            *slot = value;
        }
    }
    for (key, slot) in [("maxDistance", &mut field.max_distance), ("minDistance", &mut field.min_distance)] {
        if params.get(key).is_some() {
            *slot = params::number(params, key)?.filter(|distance| *distance > 0.0);
        }
    }
    if let Some(seed) = params::integer(params, "seed")? {
        field.seed = seed;
    }
    field.check().map_err(bad)
}

/// `object1` and `object2` as given: absent, cleared (null), or naming an object.
type Ends = [Option<Option<Id>>; 2];

fn constraint_ends(scene: &Scene, params: &Value) -> Result<Ends, String> {
    let end = |key: &str| -> Result<Option<Option<Id>>, String> {
        match params.get(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(value) => params::resolve(scene, value).map(|id| Some(Some(id))),
        }
    };
    Ok([end("object1")?, end("object2")?])
}

/// `[lower, upper]` (or null) as given under `key`, scaled by `unit`.
fn range(value: &Value, key: &str, unit: f32) -> Result<Option<[f32; 2]>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Array(ends) if ends.len() == 2 => {
            let end = |slot: usize| ends.get(slot).and_then(Value::as_f64).filter(|end| end.is_finite()).map(|end| end as f32 * unit);
            match (end(0), end(1)) {
                (Some(low), Some(high)) if low <= high => Ok(Some([low, high])),
                _ => Err(bad(format!("`{key}` must be [lower, upper] with lower at most upper"))),
            }
        }
        _ => Err(bad(format!("`{key}` must be [lower, upper] or null"))),
    }
}

/// Three entries (X, Y, Z) under `key`, each read by `read`.
fn per_axis<T: Copy>(params: &Value, key: &str, read: impl Fn(&Value) -> Result<Option<T>, String>) -> Result<Option<[Option<T>; 3]>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(Value::Array(items)) if items.len() == 3 => {
            let mut out = [None; 3];
            for (slot, item) in out.iter_mut().zip(items) {
                *slot = read(item)?;
            }
            Ok(Some(out))
        }
        Some(_) => Err(bad(format!("`{key}` must be three entries, X, Y, and Z"))),
    }
}

fn set_constraint(constraint: &mut RigidConstraint, params: &Value, ends: Ends, owner: Id) -> Result<(), String> {
    for key in ["linearMotor", "angularMotor", "linearTargetVelocity", "angularTargetVelocity", "linearMaxImpulse", "angularMaxImpulse"] {
        if params.get(key).is_some_and(Value::is_null) {
            return Err(bad(format!("`{key}` cannot be null")));
        }
    }
    if let Some(name) = params::string(params, "type")? {
        constraint.kind = JointKind::ALL.into_iter().find(|kind| kind.name() == name).ok_or_else(|| bad(format!("no constraint type {name}")))?;
    }
    let [first, second] = ends;
    if let Some(first) = first {
        constraint.object1 = first;
    }
    if let Some(second) = second {
        constraint.object2 = second;
    }
    if constraint.objects().any(|id| id == owner) {
        return Err(bad("a constraint joins two other objects, not its own"));
    }
    if constraint.object1.is_some() && constraint.object1 == constraint.object2 {
        return Err(bad("a constraint joins two different objects"));
    }
    for (key, slot) in [
        ("enabled", &mut constraint.enabled),
        ("disableCollisions", &mut constraint.disable_collisions),
        ("breakable", &mut constraint.breakable),
        ("linearMotor", &mut constraint.linear_motor),
        ("angularMotor", &mut constraint.angular_motor),
    ] {
        if let Some(value) = params::boolean(params, key)? {
            *slot = value;
        }
    }
    for (key, slot) in [
        ("breakingThreshold", &mut constraint.breaking_threshold),
        ("springDamping", &mut constraint.spring_damping),
        ("linearTargetVelocity", &mut constraint.linear_target_velocity),
        ("angularTargetVelocity", &mut constraint.angular_target_velocity),
        ("linearMaxImpulse", &mut constraint.linear_max_impulse),
        ("angularMaxImpulse", &mut constraint.angular_max_impulse),
    ] {
        if let Some(value) = params::number(params, key)? {
            *slot = value;
        }
    }
    let degrees = 1f32.to_radians();
    if let Some(value) = params.get("angleLimit") {
        constraint.angle_limit = range(value, "angleLimit", degrees)?;
    }
    if let Some(value) = params.get("distanceLimit") {
        constraint.distance_limit = range(value, "distanceLimit", 1.0)?;
    }
    if let Some(limits) = per_axis(params, "linearLimits", |item| range(item, "linearLimits", 1.0))? {
        constraint.linear_limits = limits;
    }
    if let Some(limits) = per_axis(params, "angularLimits", |item| range(item, "angularLimits", degrees))? {
        constraint.angular_limits = limits;
    }
    let stiffness = |item: &Value| -> Result<Option<f32>, String> {
        match item {
            Value::Null => Ok(None),
            value => value
                .as_f64()
                .filter(|number| number.is_finite() && *number >= 0.0)
                .map(|number| Some(number as f32))
                .ok_or_else(|| bad("a spring's stiffness is a number, 0 or more, or null")),
        }
    };
    if let Some(springs) = per_axis(params, "linearSprings", stiffness)? {
        constraint.linear_springs = springs;
    }
    if let Some(springs) = per_axis(params, "angularSprings", stiffness)? {
        constraint.angular_springs = springs;
    }
    constraint.check().map_err(|error| bad(error.to_string()))
}

/// A new empty with a force field, selected.
fn new_field(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let kind = field_kind(params)?.unwrap_or_default();
    let mut field = ForceField::new(kind);
    set_field(&mut field, params)?;
    let default_name = match kind {
        FieldKind::Force => "Field",
        FieldKind::Wind => "Wind",
        FieldKind::Vortex => "Vortex",
        FieldKind::Turbulence => "Turbulence",
        FieldKind::Drag => "Drag",
    };
    let name = params::string(params, "name")?.unwrap_or(default_name);
    let location = params::vec3(params, "location")?;
    let rotation = params::vec3(params, "rotation")?;
    let id = doc.scene.add_object(name, ObjectData::Empty { size: 1.0 });
    let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
    if let Some(location) = location {
        object.location = location;
    }
    if let Some(rotation) = rotation {
        object.rotation = Vec3::new(rotation.x.to_radians(), rotation.y.to_radians(), rotation.z.to_radians());
    }
    object.physics = Some(ObjectPhysics { field: Some(field), ..ObjectPhysics::default() });
    doc.session.editor.selected = vec![id];
    doc.session.editor.active = Some(id);
    Ok(json!({ "object": id, "objects": [object_json(&doc.scene, id)] }))
}

/// Joins rigid bodies one to the next with constraints on new empties halfway between them.
fn connect(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let ids = chosen(doc, params)?;
    let kind = match params::string(params, "type")? {
        None => JointKind::Fixed,
        Some(name) => JointKind::ALL.into_iter().find(|kind| kind.name() == name).ok_or_else(|| bad(format!("no constraint type {name}")))?,
    };
    let bodies: Vec<Id> = ids
        .into_iter()
        .filter(|id| doc.scene.object(*id).ok().and_then(|object| object.physics.as_ref()).is_some_and(|physics| physics.rigid_body.is_some()))
        .collect();
    if bodies.len() < 2 {
        return Err(bad("connect joins two or more rigid bodies"));
    }
    let mut made = Vec::new();
    for (first, second) in bodies.iter().copied().zip(bodies.iter().copied().skip(1)) {
        let at = |id: Id| doc.scene.world_matrix(id).map(|matrix| matrix.w_axis.truncate()).map_err(|error| error.to_string());
        let middle = (at(first)? + at(second)?) * 0.5;
        let id = doc.scene.add_object("Constraint", ObjectData::Empty { size: 0.5 });
        let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
        object.location = middle;
        let constraint = RigidConstraint { kind, object1: Some(first), object2: Some(second), ..RigidConstraint::default() };
        object.physics = Some(ObjectPhysics { rigid_constraint: Some(constraint), ..ObjectPhysics::default() });
        made.push(id);
    }
    doc.session.editor.selected = made.clone();
    doc.session.editor.active = made.last().copied();
    let objects: Vec<Value> = made.iter().map(|id| object_json(&doc.scene, *id)).collect();
    Ok(json!({ "objects": objects }))
}

fn world_json(world: &PhysicsWorld) -> Value {
    serde_json::to_value(world).unwrap_or(Value::Null)
}

fn object_json(scene: &Scene, id: Id) -> Value {
    let physics = scene.object(id).ok().and_then(|object| object.physics.clone()).unwrap_or_default();
    let mut value = serde_json::to_value(&physics).unwrap_or_else(|_| json!({}));
    if let Some(map) = value.as_object_mut() {
        map.insert("id".into(), json!(id));
    }
    value
}

/// Starts a bake of the scene as it is now (replacing one under way).
fn start(doc: &mut SceneDocument) -> Result<(), String> {
    let mut scene = doc.scene.clone();
    scene.physics_cache = Default::default();
    let run = match Bake::new(&scene) {
        Ok(run) => run,
        Err(error) => {
            fail(doc, error.clone());
            return Err(error);
        }
    };
    doc.session.physics.failure = None;
    doc.session.physics.baked = Some(Arc::new(run.result()));
    doc.session.physics.running = Some(Arc::new(Mutex::new(run)));
    Ok(())
}

fn fail(doc: &mut SceneDocument, error: String) {
    doc.session.physics.running = None;
    doc.session.physics.baked = None;
    doc.session.physics.follow = false;
    doc.session.physics.failure = Some(error);
    install(doc);
}

/// Runs `frames` more frames of the bake under way; shows what it has.
fn advance(doc: &mut SceneDocument, frames: usize) -> Result<(), String> {
    let Some(running) = doc.session.physics.running.clone() else { return Ok(()) };
    let mut run = running.lock().map_err(|_| "failed: the bake stopped".to_string())?;
    let done = run.step(frames);
    if let Some(error) = run.error().map(str::to_string) {
        drop(run);
        fail(doc, error.clone());
        return Err(error);
    }
    doc.session.physics.baked = Some(Arc::new(run.result()));
    drop(run);
    if done {
        doc.session.physics.running = None;
    }
    install(doc);
    Ok(())
}

fn bake(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    if doc.scene.simulated_objects().next().is_none() {
        return Err(bad("nothing has physics to bake"));
    }
    doc.session.physics.follow = true;
    start(doc)?;
    if params::boolean(params, "wait")?.unwrap_or(true) {
        advance(doc, usize::MAX)?;
    } else {
        install(doc);
    }
    Ok(state_json(doc))
}

fn bake_step(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let frames = params::integer(params, "frames")?.unwrap_or(STEP_FRAMES).clamp(1, 10_000);
    advance(doc, frames as usize)?;
    Ok(state_json(doc))
}

/// Puts the session's bake into the scene and checks it against the scene.
fn install(doc: &mut SceneDocument) {
    doc.scene.physics_cache.baked = doc.session.physics.baked.clone();
    doc.scene.refresh_physics();
}

/// After any change (and when the view is rebuilt from the replica): shows the bake if it still
/// matches; when it does not and someone baked, a new bake starts.
pub fn tidy(doc: &mut SceneDocument) {
    install(doc);
    if doc.scene.physics_cache.current || !doc.session.physics.follow {
        return;
    }
    let key = doc.scene.physics_key();
    let running = doc.session.physics.running.as_ref().and_then(|run| run.lock().ok().map(|run| run.key()));
    if running == Some(key) {
        return;
    }
    if doc.scene.simulated_objects().next().is_none() {
        doc.session.physics.running = None;
        return;
    }
    if start(doc).is_err() {
        doc.session.physics.running = None;
    }
    install(doc);
}

fn state_json(doc: &SceneDocument) -> Value {
    let physics = &doc.session.physics;
    let (done, total) = match &physics.running {
        Some(run) => run.lock().map(|run| run.progress()).unwrap_or((0, 0)),
        None => {
            let frames = physics.baked.as_ref().map_or(0, |baked| baked.frames);
            (frames, frames)
        }
    };
    let state = if physics.failure.is_some() {
        "failed"
    } else if physics.running.is_some() {
        "baking"
    } else if physics.baked.is_none() {
        "none"
    } else if doc.scene.physics_cache.current {
        "baked"
    } else {
        "outdated"
    };
    let mut state = json!({ "state": state, "frames": done, "total": total });
    if let (Some(error), Some(map)) = (&physics.failure, state.as_object_mut()) {
        map.insert("error".into(), json!(error));
    }
    state
}

/// `physics` in inspect: the world, the bake's state, each object's settings, and whether
/// anything can be baked (physics or dynamic hair).
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let Some(map) = inspected.as_object_mut() else { return };
    let objects: Vec<Value> = doc.scene.physics_objects().map(|object| object_json(&doc.scene, object.id)).collect();
    let bakeable = doc.scene.simulated_objects().next().is_some();
    let mut physics = json!({ "world": world_json(&doc.scene.physics), "objects": objects, "bakeable": bakeable });
    if let (Some(fields), Value::Object(state)) = (physics.as_object_mut(), state_json(doc)) {
        fields.extend(state);
    }
    map.insert("physics".into(), physics);
}
