//! What each write rests on (task card C8-scene-read): for a command and its parameters, the
//! premises of every object the call reads to decide what it acts on, reads to compute what it
//! writes, or replaces whole, for kition-sdk's premise guard (`KitDocument::under`). The guard holds
//! a write's target and object parameters on every field and position; these premises replace that
//! per object, and add the objects a call reaches without a parameter: the selection or the active
//! object, the mode's session object, an object's material, the scene root (`"scene"`), the
//! ancestors an object's world place derives from.
//!
//! Each rule was read from the command's handler (`tests/declarations_independent.rs` records the
//! reading, path by path). The rules share a few kinds:
//! - **Who** the call acts on ([`Who`]): a parameter, else the selection, the active object or the
//!   mode's session object; the object itself only has to exist (`fields([])`) unless the call reads
//!   or replaces some of its fields.
//! - **Where** ([`At`]): nothing of where it is, its own place, or its world place (its own place
//!   and `constraints`, and every ancestor's: the parents and constraint targets it derives from).
//! - **The scene root's** fields the call reads or replaces (`camera`, `timeline`, `world`, ...).
//! - Per family, what the parameters add ([`dynamic`]): an item of a list by index
//!   (`modifiers.1`), the element indexes' `~topology`, an operand or a target that only has to
//!   exist, a material a colour key goes to, the picture's occluders.
//!
//! - A driven value the call reads ([`drivers`]): the driver's variable sources.
//! - A mode's session: a call in it holds the session's object as its handler reads it; the step
//!   that enters a session holds its object whole, since the kit is asked about every step of a
//!   run with the document as it was before the run (no session on).
//!
//! Not held (the reading's rules): what only refuses the call (validation, kind checks), what only
//! makes a new name unique, what only shapes the receipt.

use std::collections::{BTreeMap, BTreeSet};

use kition_kit::Premise;
use kition_scene_model::animation::Property;
use kition_scene_model::drivers::{Source, VariableSpace};
use kition_scene_model::{Id, ObjectData, Scene};
use serde_json::{Value, json};

use crate::params;

/// The editor state a call may act on when a parameter is left out: the selection and the
/// active object, and each mode's session object.
#[derive(Debug, Clone, Copy, Default)]
pub struct Editor<'a> {
    pub selected: &'a [Id],
    pub active: Option<Id>,
    /// The mesh in edit mode.
    pub edited: Option<Id>,
    pub sculpt: Option<Id>,
    /// A curve's, text's or lattice's edit session object.
    pub shape: Option<Id>,
    /// An armature in edit or pose mode.
    pub rig: Option<Id>,
    /// The armature in pose mode, and whether it has an active bone (a constraint command's
    /// bone when it names none).
    pub posed: Option<(Id, bool)>,
    /// A mesh in weight paint mode.
    pub paint: Option<Id>,
    /// The emitter and system Particle Edit grooms.
    pub groom: Option<(Id, usize)>,
    /// What each scope's undo and redo steps wrote, as the kit recorded it (`None`: not known).
    pub history: Option<&'a crate::History>,
}

/// What one undo step wrote (C8-scene-engine 5a): its objects, materials and node groups by id
/// (changed, created or deleted), and the scene's own fields it changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Touched {
    pub ids: BTreeSet<Id>,
    pub root: BTreeSet<&'static str>,
}

impl Touched {
    /// What a change from `before` to `after` wrote.
    pub fn between(before: &Scene, after: &Scene) -> Self {
        fn differ<T: PartialEq>(ids: &mut BTreeSet<Id>, before: &BTreeMap<Id, T>, after: &BTreeMap<Id, T>) {
            for id in before.keys().chain(after.keys()) {
                if before.get(id) != after.get(id) {
                    ids.insert(*id);
                }
            }
        }
        fn objects(scene: &Scene) -> BTreeMap<Id, &kition_scene_model::Object> {
            scene.objects.iter().map(|object| (object.id, object)).collect()
        }
        fn channels(scene: &Scene) -> BTreeMap<Id, Vec<&kition_scene_model::animation::Channel>> {
            let mut by: BTreeMap<Id, Vec<&kition_scene_model::animation::Channel>> = BTreeMap::new();
            for channel in &scene.animation.channels {
                by.entry(channel.target).or_default().push(channel);
            }
            by
        }
        fn materials(scene: &Scene) -> BTreeMap<Id, &kition_scene_model::Material> {
            scene.materials.iter().map(|material| (material.id, material)).collect()
        }
        fn groups(scene: &Scene) -> BTreeMap<Id, &kition_scene_model::node_tree::NodeGroup> {
            scene.node_groups.iter().map(|group| (group.id, group)).collect()
        }
        let mut ids = BTreeSet::new();
        differ(&mut ids, &objects(before), &objects(after));
        differ(&mut ids, &channels(before), &channels(after));
        differ(&mut ids, &materials(before), &materials(after));
        differ(&mut ids, &groups(before), &groups(after));
        let settings = |scene: &Scene| -> [(&'static str, Value); 8] {
            let animation = &scene.animation;
            let timeline = &animation.timeline;
            [
                ("world", serde_json::to_value(&scene.world).unwrap_or(Value::Null)),
                ("render", serde_json::to_value(scene.render).unwrap_or(Value::Null)),
                ("timeline", json!([timeline.start, timeline.end, timeline.fps])),
                ("camera", json!(scene.camera)),
                ("physics", serde_json::to_value(&scene.physics).unwrap_or(Value::Null)),
                ("actions", serde_json::to_value(&animation.actions).unwrap_or(Value::Null)),
                ("keyingSets", serde_json::to_value(&animation.keying_sets).unwrap_or(Value::Null)),
                ("objects", json!(scene.objects.iter().map(|object| object.id).collect::<Vec<_>>())),
            ]
        };
        let root = settings(before).into_iter().zip(settings(after)).filter(|((_, old), (_, new))| old != new).map(|((name, _), _)| name).collect();
        Self { ids, root }
    }
}

/// What one call needs of one object: the fields or paths it reads or replaces (`None`: every
/// one), and the positions it depends on (`None`: every one; empty: none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Need {
    pub fields: Option<BTreeSet<String>>,
    pub positions: Option<BTreeSet<String>>,
}

impl Need {
    /// Every field and every position.
    pub fn all() -> Self {
        Self::default()
    }

    /// These fields and no position.
    pub fn of(fields: &[&str]) -> Self {
        Self { fields: Some(fields.iter().map(|field| (*field).to_owned()).collect()), positions: Some(BTreeSet::new()) }
    }

    /// These fields and every position.
    fn placed(fields: &[&str]) -> Self {
        Self { positions: None, ..Self::of(fields) }
    }

    /// Every field, no position (a node: no object positions).
    fn whole() -> Self {
        Self { fields: None, positions: Some(BTreeSet::new()) }
    }

    /// What two needs of one object come to together.
    pub fn merge(&mut self, other: &Need) {
        fn union(into: &mut Option<BTreeSet<String>>, more: &Option<BTreeSet<String>>) {
            match (into.as_mut(), more) {
                (Some(known), Some(more)) => known.extend(more.iter().cloned()),
                _ => *into = None,
            }
        }
        union(&mut self.fields, &other.fields);
        union(&mut self.positions, &other.positions);
    }

    pub fn premise(&self, on: &On) -> Premise {
        let mut premise = Premise::on(on.id());
        premise.fields = self.fields.as_ref().map(|fields| fields.iter().cloned().collect());
        premise.positions = self.positions.as_ref().map(|positions| positions.iter().cloned().collect());
        premise
    }
}

/// What a premise is on: an object (or material, node group, node) by id, or the scene root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum On {
    Id(Id),
    Scene,
}

impl On {
    fn id(&self) -> Value {
        match self {
            Self::Id(id) => json!(id),
            Self::Scene => json!("scene"),
        }
    }
}

/// Who a call acts on.
#[derive(Clone, Copy, Debug)]
enum Who {
    /// Nobody: the call names no stored object.
    Nobody,
    /// `objects`, else the selection.
    Objects,
    /// In edit mode the edited mesh (`objects` ignored), else `objects`, else the selection.
    ObjectsOrEdited,
    /// `object`, else the active object.
    Object,
    /// In edit mode the edited mesh (`object` ignored), else `object`, else the active object.
    ObjectOrEdited,
    /// `object`, else the edited mesh, else the active object.
    Edited,
    /// `object`, else the sculpted mesh, else the active object.
    Sculpted,
    /// `object`, else the curve, text or lattice being edited, else the active object.
    Shaped,
    /// `object`, else the weight-painted mesh, else the active object.
    Painted,
    /// `object`, else the armature in pose mode, else the active object.
    Constrained,
    /// The armature in edit or pose mode only.
    Rig,
    /// The weight-painted mesh only.
    Paint,
    /// The curve, text or lattice being edited only.
    Shape,
    /// The object or material a graph channel names by id.
    Channel,
}

/// What of where the acted-on object is the call depends on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum At {
    /// Nothing.
    Here,
    /// Its own location, rotation, scale and parent.
    Own,
    /// Its world place: its own, its `constraints`, and every ancestor's.
    World,
}

/// A kind of rule: the commands it covers, who they act on, the fields of that object they read or
/// replace, where it is, and the scene root's fields.
struct Rule {
    commands: &'static [&'static str],
    who: Who,
    fields: &'static [&'static str],
    at: At,
    scene: &'static [&'static str],
}

const fn rule(commands: &'static [&'static str], who: Who, fields: &'static [&'static str], at: At, scene: &'static [&'static str]) -> Rule {
    Rule { commands, who, fields, at, scene }
}

/// The rules, family by family, each from its handler. A command's [`dynamic`] part adds what its
/// parameters decide.
const RULES: &[Rule] = &[
    // New objects from parameters: the new object is appended, its name only made unique. (The
    // primitives' edit-mode path joins the edited mesh: `dynamic`.)
    rule(
        &[
            "add.plane",
            "add.backdrop",
            "add.cube",
            "add.circle",
            "add.uvSphere",
            "add.icoSphere",
            "add.cylinder",
            "add.cone",
            "add.torus",
            "add.grid",
            "add.light",
            "add.empty",
            "add.armature",
            "add.curveBezier",
            "add.curveCircle",
            "add.curvePath",
            "add.text",
            "add.lattice",
            "physics.field.new",
            "action.new",
            "material.new",
            "nodes.newGroup",
            "compositor.new",
        ],
        Who::Nobody,
        &[],
        At::Here,
        &[],
    ),
    // add.camera: the new camera becomes the scene's when it has none.
    rule(&["add.camera"], Who::Nobody, &[], At::Here, &["camera"]),
    // Undo and redo invert or replay a scope's last transaction (`dynamic`: what its steps wrote).
    rule(&["edit.undo", "edit.redo"], Who::Nobody, &[], At::Here, &[]),
    // object.transform sets the given local values; object.scale multiplies the local scale.
    rule(&["object.transform", "object.scale"], Who::Objects, &[], At::Own, &[]),
    // object.translate and object.rotate turn a world offset or axis through the parent's frame
    // as it is when the step runs: the object's own place (parent and parent inverse), not its
    // constraints, not its ancestors'.
    rule(&["object.translate", "object.rotate"], Who::Objects, &[], At::Own, &[]),
    // A rename replaces the name (a lost update otherwise).
    rule(&["object.rename"], Who::Object, &["name"], At::Here, &[]),
    rule(&["object.hide"], Who::Objects, &["hidden"], At::Here, &[]),
    // object.order moves the object to an index of the scene's objects. The list is held by the
    // insertion window `place::under` gives (a premise on the root's whole `objects` would refuse
    // every insertion elsewhere in the list).
    rule(&["object.order"], Who::Object, &[], At::Here, &[]),
    // object.setCamera replaces the scene's camera (the object must be a camera: a kind check).
    rule(&["object.setCamera"], Who::Object, &[], At::Here, &["camera"]),
    // object.lookAt aims the object from its world place, rewriting it in the parent's space.
    rule(&["object.lookAt"], Who::Object, &[], At::World, &[]),
    // Shading and normals: the mesh blob (smooth, its windings); the edit mode's mesh wins.
    rule(
        &["object.shadeSmooth", "object.shadeSmoothByAngle", "object.shadeFlat", "mesh.recalculateNormals"],
        Who::ObjectsOrEdited,
        &["mesh"],
        At::Here,
        &[],
    ),
    // mesh.setGeometry replaces the mesh whole.
    rule(&["mesh.setGeometry"], Who::ObjectOrEdited, &["mesh"], At::Here, &[]),
    // modifier.move reorders the stack (`to` checked against its length).
    rule(&["modifier.move"], Who::Object, &["modifiers"], At::Here, &[]),
    // modifier.add appends a modifier (a unique name; operand and group stored by id: `dynamic`).
    rule(&["modifier.add"], Who::ObjectOrEdited, &[], At::Here, &[]),
    rule(&["material.clear"], Who::Objects, &["material"], At::Here, &[]),
    // A light's or a camera's settings merge into its data.
    rule(&["light.set", "camera.set"], Who::Object, &["data"], At::Here, &[]),
    rule(&["world.set", "world.setImage"], Who::Nobody, &[], At::Here, &["world"]),
    rule(&["render.set"], Who::Nobody, &[], At::Here, &["render"]),
    // file.import: keys land by the timeline's fps, a camera becomes the scene's when it has none.
    rule(&["file.import"], Who::Nobody, &[], At::Here, &["camera", "timeline"]),
    // UV projections read each mesh's local positions and faces; reset only each face's corner
    // count; clear nothing.
    rule(
        &["uv.smartProject", "uv.cubeProject", "uv.cylinderProject", "uv.sphereProject", "uv.planarProject"],
        Who::ObjectsOrEdited,
        &["mesh"],
        At::Here,
        &[],
    ),
    rule(&["uv.reset"], Who::ObjectsOrEdited, &["~topology"], At::Here, &[]),
    rule(&["uv.clear"], Who::ObjectsOrEdited, &[], At::Here, &[]),
    // material.setTexture sets one slot of the material (`dynamic`: the material exists).
    rule(&["material.setTexture"], Who::Nobody, &[], At::Here, &[]),
    // Sculpt session commands: the mask, the face sets and the hidden faces live in the mesh.
    rule(
        &["sculpt.maskInvert", "sculpt.maskSmooth", "sculpt.faceSetsFromMask", "sculpt.faceSetsFromVisible", "sculpt.faceSetsFromLooseParts"],
        Who::Sculpted,
        &["mesh"],
        At::Here,
        &[],
    ),
    rule(&["sculpt.maskClear", "sculpt.faceSetsClear", "sculpt.showAllFaces"], Who::Sculpted, &[], At::Here, &[]),
    rule(&["sculpt.maskFill"], Who::Sculpted, &["~topology"], At::Here, &[]),
    // detailFlood: the edge length from the view's pixel size at the world bounds' centre.
    rule(&["sculpt.detailFlood"], Who::Sculpted, &["mesh"], At::World, &[]),
    rule(&["sculpt.remesh"], Who::Sculpted, &["mesh"], At::Here, &[]),
    // Multires uses the first Multires modifier (appended by subdivide when there is none).
    rule(
        &["multires.subdivide", "multires.unsubdivide", "multires.rebuildSubdivisions", "multires.deleteHigher"],
        Who::Sculpted,
        &["mesh", "modifiers"],
        At::Here,
        &[],
    ),
    rule(&["multires.applyBase", "multires.reshape"], Who::Sculpted, &["mesh"], At::Here, &[]),
    rule(&["frame.range"], Who::Nobody, &[], At::Here, &["timeline"]),
    rule(&["keyingSet.new", "keyingSet.remove", "keyingSet.removePath", "keyingSet.setActive"], Who::Nobody, &[], At::Here, &["keyingSets"]),
    // Strips, tweaks and Action constraints follow an action's removal or rename: rewritten, not
    // read to decide.
    rule(&["action.remove", "action.rename"], Who::Nobody, &[], At::Here, &["actions"]),
    // nla.*: the object's tracks and strips by index (tweak checks only refuse).
    rule(
        &[
            "nla.addTrack",
            "nla.removeTrack",
            "nla.setTrack",
            "nla.setStrip",
            "nla.addTransition",
            "nla.makeMeta",
            "nla.dissolveMeta",
            "nla.removeStrip",
            "nla.setAction",
        ],
        Who::Object,
        &["nla"],
        At::Here,
        &[],
    ),
    // nla.addStrip: the action's keyed range sets the strip's frames.
    rule(&["nla.addStrip"], Who::Object, &["nla"], At::Here, &["actions"]),
    // nla.tweak / tweakExit: the strip's action's keys swap with the object's own (stashed in tweak).
    rule(&["nla.tweak", "nla.tweakExit"], Who::Object, &["animation", "nla", "tweak"], At::Here, &["actions"]),
    // nla.pushDown: the object's keyed channels become an action (its default name: `dynamic`).
    rule(&["nla.pushDown"], Who::Object, &["animation", "nla"], At::Here, &[]),
    // graph.*: the exact channel of the object or material the target id names, and its modifiers
    // (the re-evaluation at the current frame afterwards writes no stored value).
    rule(
        &["graph.addModifier", "graph.setModifier", "graph.removeModifier", "graph.moveKey", "graph.deleteKey", "graph.setInterpolation"],
        Who::Channel,
        &["animation"],
        At::Here,
        &[],
    ),
    // Armature edit and pose sessions: the session's armature's bones.
    rule(&["bone.add", "bone.extrude", "pose.set", "pose.clear"], Who::Rig, &["data"], At::Here, &[]),
    // Pose keys: the bones' pose values keyed into (or keys taken from) the armature's channels at
    // the tweak-mapped frame.
    rule(&["pose.insertKey"], Who::Rig, &["animation", "data", "nla", "tweak"], At::Here, &[]),
    rule(&["pose.deleteKey"], Who::Rig, &["animation", "nla", "tweak"], At::Here, &[]),
    // Vertex groups live in the mesh, by name: a new group is sized to the vertex count, an
    // assignment names vertices by index.
    rule(&["vertexGroup.add", "vertexGroup.assign", "vertexGroup.unassign"], Who::Edited, &["~topology"], At::Here, &[]),
    rule(&["vertexGroup.remove"], Who::Edited, &["mesh"], At::Here, &[]),
    // Shape keys: by name in the mesh; a removal prunes the key's channel, a key keys its value.
    rule(&["shapeKey.remove", "shapeKey.insertKey"], Who::Edited, &["animation", "mesh"], At::Here, &[]),
    rule(&["shapeKey.offset"], Who::Edited, &["mesh"], At::Here, &[]),
    rule(&["shapeKey.add", "shapeKey.set"], Who::Edited, &[], At::Here, &[]),
    // Object constraints: the stack (a removal or a move shifts it); apply makes the owner's world
    // with the stack up to the constraint its transform.
    rule(&["constraint.remove", "constraint.move"], Who::Constrained, &["constraints"], At::Here, &[]),
    rule(&["constraint.apply"], Who::Constrained, &[], At::World, &[]),
    rule(&["constraint.add", "constraint.set", "constraint.setInverse", "constraint.clearInverse"], Who::Constrained, &[], At::Here, &[]),
    // Weight paint session only: the session's mesh, its world, evaluated mesh and the view.
    rule(&["paint.gradient", "paint.stroke"], Who::Paint, &["mesh", "modifiers"], At::World, &[]),
    rule(&["paint.normalizeAll"], Who::Painted, &["mesh"], At::Here, &[]),
    rule(&["paint.vertices"], Who::Painted, &[], At::Here, &[]),
    // Curves, texts and lattices: settings, splines, letters and points merge into the data.
    rule(&["curve.set", "text.set", "curve.setSplines", "text.insert", "lattice.set", "lattice.movePoints"], Who::Shaped, &["data"], At::Here, &[]),
    // Curve and text edit sessions only: the session's selected points or caret.
    rule(
        &["curve.translate", "curve.extrude", "curve.subdivide", "curve.toggleCyclic", "curve.setHandleType", "curve.deletePoints", "text.delete"],
        Who::Shape,
        &["data"],
        At::Here,
        &[],
    ),
    // convertToMesh: each curve or text becomes its evaluated mesh and loses its modifiers.
    rule(&["object.convertToMesh"], Who::Objects, &["data", "modifiers"], At::Here, &[]),
    // Physics settings merge into the object's `physics` alone (kind checks only refuse).
    rule(
        &[
            "physics.rigidBody.add",
            "physics.rigidBody.set",
            "physics.rigidBody.remove",
            "physics.cloth.add",
            "physics.cloth.set",
            "physics.cloth.remove",
            "physics.softBody.add",
            "physics.softBody.set",
            "physics.softBody.remove",
            "physics.collision.add",
            "physics.collision.remove",
            "physics.field.add",
            "physics.field.set",
            "physics.field.remove",
            "physics.constraint.add",
            "physics.constraint.set",
            "physics.constraint.remove",
        ],
        Who::Objects,
        &["physics"],
        At::Here,
        &[],
    ),
    rule(&["physics.world.set"], Who::Nobody, &[], At::Here, &["physics"]),
    // connect: each new Empty sits between two bodies' world origins.
    rule(&["physics.constraint.connect"], Who::Objects, &["physics"], At::World, &[]),
    // drop and pop key from each object's local location and scale at `start`.
    rule(&["object.drop", "object.pop"], Who::Objects, &["animation", "nla", "tweak"], At::Own, &["timeline"]),
];

/// The fields an object's world bounds at a frame are evaluated from: its animation and NLA, its
/// constraints, whether it is hidden, and its geometry (with modifiers).
fn bounds_fields(scene: &Scene, id: Id, animated: bool) -> Vec<&'static str> {
    let mut fields = vec!["constraints", "hidden"];
    if animated {
        fields.extend(["animation", "nla"]);
    }
    match scene.object(id).map(|object| &object.data) {
        Ok(ObjectData::Mesh(_)) => fields.extend(["mesh", "modifiers"]),
        Ok(ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_)) => fields.extend(["data", "modifiers"]),
        _ => {}
    }
    fields
}

/// What a call rests on, gathered object by object.
struct Rests<'a> {
    scene: &'a Scene,
    editor: Editor<'a>,
    params: &'a Value,
    found: BTreeMap<On, Need>,
}

impl<'a> Rests<'a> {
    fn add(&mut self, on: On, need: Need) {
        match self.found.get_mut(&on) {
            Some(known) => known.merge(&need),
            None => {
                self.found.insert(on, need);
            }
        }
    }

    fn of(&mut self, id: Id, fields: &[&str]) {
        self.add(On::Id(id), Need::of(fields));
    }

    fn placed(&mut self, id: Id, fields: &[&str]) {
        self.add(On::Id(id), Need::placed(fields));
    }

    fn scene(&mut self, fields: &[&str]) {
        if !fields.is_empty() {
            self.add(On::Scene, Need::of(fields));
        }
    }

    /// Its world place: its own place and constraints, and every ancestor's (the parents and the
    /// constraint targets it derives from, through theirs).
    fn world(&mut self, id: Id) {
        self.placed(id, &["constraints"]);
        let mut seen = BTreeSet::from([id]);
        let mut queue = self.from(id);
        while let Some(at) = queue.pop() {
            if !seen.insert(at) || seen.len() > self.scene.objects.len().saturating_add(1) {
                continue;
            }
            self.placed(at, &["constraints"]);
            queue.extend(self.from(at));
        }
    }

    /// The frame an object's own fields are in, when it is parented: its own place (the parent
    /// inverse) and the parent's world place. Not its own constraints.
    fn parent_frame(&mut self, id: Id) {
        if let Some(parent) = self.scene.object(id).ok().and_then(|object| object.parent) {
            self.placed(id, &[]);
            self.world(parent);
        }
    }

    /// The objects `id`'s world place derives from directly: its parent and constraint targets.
    fn from(&self, id: Id) -> Vec<Id> {
        let Ok(object) = self.scene.object(id) else { return Vec::new() };
        object
            .parent
            .into_iter()
            .chain(object.constraints.iter().flat_map(|constraint| constraint.kind.targets()).map(|target| target.object))
            .collect()
    }

    /// An object's world bounds at a frame (`animated`) or as it stands.
    fn bounds(&mut self, id: Id, animated: bool) {
        let fields = bounds_fields(self.scene, id, animated);
        self.world(id);
        self.placed(id, &fields);
    }

    /// The viewport picture besides `subject`: every visible object with a surface is drawn and
    /// can occlude (where it is in the world too), a hidden one by whether it is hidden.
    fn picture(&mut self, subject: Option<Id>) {
        let scene = self.scene;
        for object in scene.objects.iter().filter(|object| Some(object.id) != subject && object.has_geometry()) {
            if object.hidden {
                self.of(object.id, &["hidden"]);
            } else {
                self.world(object.id);
                self.add(On::Id(object.id), Need::all());
            }
        }
    }

    /// The ids a parameter names (one or a list): a deleted numeric id kept, so the guard refuses
    /// its absence. `None` when the parameter is left out.
    fn named(&self, key: &str) -> Option<Vec<Id>> {
        let value = self.params.get(key).filter(|value| !value.is_null())?;
        let items: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        Some(items.into_iter().filter_map(|item| id_of(self.scene, item)).collect())
    }

    fn objects_or_selection(&self) -> Vec<Id> {
        self.named("objects").unwrap_or_else(|| self.editor.selected.to_vec())
    }

    fn object_or(&self, fallback: Option<Id>) -> Vec<Id> {
        self.named("object").unwrap_or_else(|| fallback.into_iter().collect())
    }

    fn who(&self, who: Who) -> Vec<Id> {
        let editor = self.editor;
        match who {
            Who::Nobody => Vec::new(),
            Who::Objects => self.objects_or_selection(),
            Who::ObjectsOrEdited => editor.edited.map_or_else(|| self.objects_or_selection(), |id| vec![id]),
            Who::Object => self.object_or(editor.active),
            Who::ObjectOrEdited => editor.edited.map_or_else(|| self.object_or(editor.active), |id| vec![id]),
            Who::Edited => self.object_or(editor.edited.or(editor.active)),
            Who::Sculpted => self.object_or(editor.sculpt.or(editor.active)),
            Who::Shaped => self.object_or(editor.shape.or(editor.active)),
            Who::Painted => self.object_or(editor.paint.or(editor.active)),
            Who::Constrained => self.object_or(editor.posed.map(|(id, _)| id).or(editor.active)),
            Who::Rig => editor.rig.into_iter().collect(),
            Who::Paint => editor.paint.into_iter().collect(),
            Who::Shape => editor.shape.into_iter().collect(),
            Who::Channel => self.params.get("target").and_then(Value::as_u64).and_then(|id| Id::try_from(id).ok()).into_iter().collect(),
        }
    }

    fn has(&self, key: &str) -> bool {
        self.params.get(key).is_some_and(|value| !value.is_null())
    }

    fn text(&self, key: &str) -> Option<&'a str> {
        self.params.get(key).and_then(Value::as_str)
    }

    fn index(&self, key: &str) -> Option<u32> {
        params::integer(self.params, key).ok().flatten()
    }

    /// Each of `keys` that names an object, a material or a node group: it only has to exist.
    fn exist(&mut self, keys: &[&str]) {
        for key in keys {
            for value in params::object_values(self.params, key) {
                if let Some(id) = any_id(self.scene, key, value) {
                    self.of(id, &[]);
                }
            }
        }
    }

    /// Each node id `keys` name: the node whole (a node has no position of the object model).
    fn nodes(&mut self, keys: &[&str]) {
        for key in keys {
            for value in params::object_values(self.params, key) {
                if let Some(id) = value.as_u64().and_then(|id| Id::try_from(id).ok()) {
                    self.add(On::Id(id), Need::whole());
                }
            }
        }
    }

    /// The material `key` names (an id or a name).
    fn material(&self, key: &str) -> Option<Id> {
        self.params.get(key).and_then(|value| crate::commands::material_ref_value(self.scene, value).ok())
    }

    /// The material an object's slot names.
    fn slot(&self, id: Id) -> Option<Id> {
        self.scene.object(id).ok().and_then(|object| object.material)
    }

    /// The node group `group` names (an id or a name), else the only one.
    fn group(&self) -> Option<Id> {
        match self.params.get("group").filter(|value| !value.is_null()) {
            Some(value) => any_id(self.scene, "group", value),
            None => match self.scene.node_groups.as_slice() {
                [only] => Some(only.id),
                _ => None,
            },
        }
    }

    /// The scene's camera.
    fn camera(&self) -> Option<Id> {
        self.scene.camera
    }
}

/// An object's id from a parameter's value: an id or its digits (kept when the scene has no such
/// object, so the guard refuses its absence), or a name.
fn id_of(scene: &Scene, value: &Value) -> Option<Id> {
    params::resolve(scene, value).ok().or_else(|| value.as_u64().and_then(|id| Id::try_from(id).ok()))
}

/// The id `value` names for parameter `key`: a material, a node group, else an object.
fn any_id(scene: &Scene, key: &str, value: &Value) -> Option<Id> {
    if let Some(id) = value.as_u64().and_then(|id| Id::try_from(id).ok()) {
        return Some(id);
    }
    let name = value.as_str()?;
    if let Some(id) = name.parse::<Id>().ok().filter(|_| name.bytes().all(|byte| byte.is_ascii_digit())) {
        return Some(id);
    }
    match key.rsplit('.').next().unwrap_or(key) {
        "material" => scene.materials.iter().find(|material| material.name == name).map(|material| material.id),
        "group" | "shader" | "volume" => scene.node_groups.iter().find(|group| group.name == name).map(|group| group.id),
        _ => scene.object_named(name).map(|object| object.id),
    }
}

/// The properties a key or channel command names (`properties`: a list or one name), if given.
fn property_list(params: &Value) -> Option<Vec<&str>> {
    match params.get("properties").filter(|value| !value.is_null())? {
        Value::Array(items) => Some(items.iter().filter_map(Value::as_str).collect()),
        Value::String(name) => Some(vec![name.as_str()]),
        _ => Some(Vec::new()),
    }
}

/// What keying `property` of an object reads of it: its keys (`animation`) and where they go in
/// time (`nla`, `tweak`), and the keyed value (its place, `hidden`, or a light's or camera's data).
/// `baseColor` keys the object's material instead: [`Rests::color_key`].
fn key_fields(property: &str) -> (&'static [&'static str], bool) {
    match property {
        "location" | "rotation" | "scale" => (&["animation", "nla", "tweak"], true),
        "hidden" => (&["animation", "hidden", "nla", "tweak"], false),
        _ => (&["animation", "data", "nla", "tweak"], false),
    }
}

impl Rests<'_> {
    /// A key, a channel or a driver of `baseColor` goes to the material the object's slot names:
    /// the slot, and the material's keys (`values`: and its settings, the keyed value).
    fn color_key(&mut self, id: Id, values: bool) {
        self.of(id, &["material"]);
        if let Some(material) = self.slot(id) {
            self.of(material, if values { &["animation", "data"] } else { &["animation"] });
        }
    }
}

/// What the parameters of `command` add to its rule, per family.
fn dynamic(rests: &mut Rests, command: &str, subjects: &[Id]) {
    let scene = rests.scene;
    let params = rests.params;
    match command {
        // Undo and redo (C8-scene-engine 5a): the objects and the scene's fields the steps they may
        // take back (or replay) wrote, every field and position; an object such a step created
        // and that is gone now (or deleted and not back) is not there to hold. Not known (the
        // replica keeps steps the kit recorded nothing of): the whole scene.
        "edit.undo" | "edit.redo" => {
            let scope = if rests.text("scope") == Some("person") { kition_kit::kdm::UndoScope::Person } else { kition_kit::kdm::UndoScope::Agent };
            match rests.editor.history.and_then(|history| history.steps(scope, command == "edit.undo")) {
                None => rests.add(On::Scene, Need::all()),
                Some(steps) => {
                    for touched in steps {
                        for id in &touched.ids {
                            let there = scene.object(*id).is_ok() || scene.material(*id).is_ok() || scene.node_group(*id).is_some();
                            if there {
                                rests.add(On::Id(*id), Need::all());
                            }
                        }
                        let root: Vec<&str> = touched.root.iter().copied().collect();
                        rests.scene(&root);
                    }
                }
            }
        }
        // The primitives in edit mode join the edited mesh, placed through its world transform.
        "add.plane" | "add.cube" | "add.circle" | "add.uvSphere" | "add.icoSphere" | "add.cylinder" | "add.cone" | "add.torus" | "add.grid" => {
            if let Some(edited) = rests.editor.edited {
                rests.world(edited);
                rests.of(edited, &["mesh"]);
            }
        }
        // applyTransform and originToGeometry rewrite a mesh and its place from both; any other
        // kind is skipped (it only has to exist).
        "object.applyTransform" | "object.originToGeometry" => {
            for id in subjects {
                if scene.object(*id).is_ok_and(|object| object.mesh().is_some()) {
                    rests.placed(*id, &["mesh"]);
                } else {
                    rests.of(*id, &[]);
                }
            }
        }
        // parent: `parent` else the active object, whose world gives the children's parent
        // inverse; `objects` else the selection less the parent, their parent fields replaced.
        "object.parent" => {
            let parent = rests.named("parent").map_or(rests.editor.active, |ids| ids.first().copied());
            let children: Vec<Id> = rests.objects_or_selection().into_iter().filter(|id| Some(*id) != parent).collect();
            for child in children {
                // Keep Transform: an old parent's frame goes into the fields.
                rests.placed(child, &[]);
                rests.parent_frame(child);
            }
            if let Some(parent) = parent {
                rests.world(parent);
            }
        }
        // clearParent: Keep Transform puts the parent's frame (the parent's world place and the
        // parent inverse) into the own fields; the own constraints are not baked.
        "object.clearParent" => {
            for id in subjects {
                rests.placed(*id, &[]);
                rests.parent_frame(*id);
            }
        }
        // reveal: `objects` (the hidden ones among them) else every hidden object: each one's
        // `hidden` read.
        "object.reveal" => {
            let ids = rests.named("objects").unwrap_or_else(|| scene.objects.iter().map(|object| object.id).collect());
            for id in ids {
                rests.of(id, &["hidden"]);
            }
        }
        // join: the active mesh takes the other selected meshes, through their world transforms.
        "object.join" => {
            for id in rests.editor.selected.iter().copied().chain(rests.editor.active) {
                rests.world(id);
                rests.of(id, &["mesh"]);
            }
        }
        // duplicate: in edit mode the selected elements of the edited mesh (their indexes, as the
        // element edits), in an armature session the selected bones, else each object whole.
        "edit.duplicate" => match (rests.editor.edited, rests.editor.rig) {
            (Some(edited), _) => rests.of(edited, &["~topology"]),
            (None, Some(rig)) => rests.of(rig, &["data"]),
            (None, None) => {
                for id in subjects {
                    rests.add(On::Id(*id), Need::all());
                }
            }
        },
        // delete in a curve's or a text's edit session: its selected points or the letter after
        // the caret (a lattice session deletes nothing); in an armature session its selected
        // bones, and their channels pruned.
        "edit.delete" if rests.editor.shape.is_some() || (rests.editor.rig.is_some() && rests.editor.edited.is_none()) => {
            match (rests.editor.shape, rests.editor.rig) {
                (Some(shape), _) => {
                    if !scene.object(shape).is_ok_and(|object| matches!(object.data, ObjectData::Lattice(_))) {
                        rests.of(shape, &["data"]);
                    }
                }
                (None, Some(rig)) => rests.of(rig, &["animation", "data"]),
                (None, None) => {}
            }
        }
        // delete: each object only has to exist; a deleted parent's children keep their world
        // place (both worlds read); a deleted scene camera is replaced by the first camera left.
        // (In edit mode, or given `elements`, it deletes the edited mesh's elements: [`elements`].)
        "edit.delete" if !rests.has("elements") && rests.editor.edited.is_none() => {
            for id in subjects {
                rests.of(*id, &[]);
                let children: Vec<Id> = scene.objects.iter().filter(|object| object.parent == Some(*id)).map(|object| object.id).collect();
                if !children.is_empty() {
                    rests.world(*id);
                }
                for child in children {
                    rests.world(child);
                }
                if scene.camera == Some(*id) {
                    rests.scene(&["camera", "objects"]);
                }
            }
        }
        // modifier.remove / setImage / set: one modifier by index, else the stack (by name).
        "modifier.remove" | "modifier.setImage" | "modifier.set" => {
            let path = rests.index("index").map_or_else(|| "modifiers".to_owned(), |index| format!("modifiers.{index}"));
            for id in subjects {
                rests.of(*id, &[path.as_str()]);
            }
            rests.exist(&["operand", "group"]);
        }
        // apply: the raw mesh through every modifier up to the one applied.
        "modifier.apply" => {
            let paths: Vec<String> = match rests.index("index") {
                Some(index) => (0..=index).map(|at| format!("modifiers.{at}")).collect(),
                None => vec!["modifiers".to_owned()],
            };
            for id in subjects {
                rests.of(*id, &["mesh"]);
                for path in &paths {
                    rests.of(*id, &[path.as_str()]);
                }
            }
        }
        "modifier.add" => rests.exist(&["operand", "group"]),
        // material.new assigns the new material (by default) to the edited mesh, else to the
        // selection's objects with geometry.
        "material.new" if params::boolean(params, "assign").ok().flatten().unwrap_or(true) => {
            let targets = rests.editor.edited.map_or_else(|| rests.editor.selected.to_vec(), |id| vec![id]);
            for id in targets {
                if scene.object(id).is_ok_and(|object| object.has_geometry()) {
                    rests.of(id, &["material"]);
                }
            }
        }
        // material.set: the named settings into its data, `name` for a rename; a shader exists.
        "material.set" => {
            if let Some(material) = rests.material("material") {
                let settings = params.as_object().is_some_and(|map| map.keys().any(|key| !matches!(key.as_str(), "material" | "name")));
                let mut fields = Vec::new();
                if settings {
                    fields.push("data");
                }
                if rests.has("name") {
                    fields.push("name");
                }
                rests.of(material, &fields);
            }
            rests.exist(&["shader"]);
        }
        "material.assign" => {
            for id in subjects {
                rests.of(*id, &["material"]);
            }
            rests.exist(&["material"]);
        }
        // delete: the users are every object whose slot names the material.
        "material.delete" => {
            rests.exist(&["material"]);
            for object in &scene.objects {
                rests.of(object.id, &["material"]);
            }
        }
        "material.setTexture" => rests.exist(&["material"]),
        // A mode's session entered (C8-scene-engine 4c): the steps after it in the run act on the
        // session's object, which no parameter of theirs names, and the kit is asked what each
        // step rests on with the document as it was before the run (no session on): the step that
        // enters the session holds its object, every field and position.
        "mode.set" if rests.text("mode") == Some("object") => {}
        "mode.set" | "mode.toggle" | "mode.armature" | "mode.pose" | "mode.weightPaint" | "mode.sculpt" | "mode.particleEdit" | "mode.shape" => {
            for id in subjects {
                rests.add(On::Id(*id), Need::all());
            }
        }
        "camera.set" => rests.exist(&["focusObject"]),
        "world.set" => rests.exist(&["volume"]),
        _ => {}
    }
    match command {
        "sculpt.stroke" | "sculpt.gesture" | "sculpt.faceSetVisibility" | "sculpt.hideFaceSet" | "sculpt.isolateFaceSet" => {
            sculpt(rests, command, subjects);
        }
        "sculpt.dyntopo" => {
            let enabled = params::boolean(params, "enabled").ok().flatten() != Some(false);
            for id in subjects {
                rests.of(*id, if enabled { &["mesh"] } else { &[] });
            }
        }
        // reshape: the other mesh's local base positions become the highest level.
        "multires.reshape" => {
            for id in rests.named("from").unwrap_or_default() {
                rests.of(id, &["mesh"]);
            }
        }
        "anim.insertKey" => insert_key(rests, subjects),
        // deleteKey: the keys of the properties (every one by default, baseColor's on the
        // material) on a tweak-mapped frame; setInterpolation: on a tweak-mapped frame when it
        // names one; clear: every key, no frame map.
        "anim.deleteKey" | "anim.clear" | "anim.setInterpolation" => {
            let mapped =
                command == "anim.deleteKey" || (command == "anim.setInterpolation" && params.get("frame").is_some_and(|frame| !frame.is_null()));
            let properties = property_list(params);
            let color = properties.as_ref().is_none_or(|list| list.contains(&"baseColor"));
            let others = properties.as_ref().is_none_or(|list| list.iter().any(|property| *property != "baseColor"));
            for id in subjects {
                if others {
                    rests.of(*id, if mapped { &["animation", "nla", "tweak"] } else { &["animation"] });
                }
                if color {
                    rests.color_key(*id, false);
                }
            }
        }
        // Drivers: the property's channel on the object (its material's for baseColor; a bone's
        // or a shape key's must exist); variable sources only referenced.
        "driver.add" | "driver.set" | "driver.remove" => {
            let property = rests.text("property").unwrap_or_default();
            for id in subjects {
                match property {
                    "baseColor" => rests.color_key(*id, false),
                    property if property.starts_with("bone") => rests.of(*id, &["animation", "data"]),
                    "shapeKey" => rests.of(*id, &["animation", "mesh"]),
                    _ => rests.of(*id, &["animation"]),
                }
            }
            rests.exist(&["variables[].object", "variables[].a", "variables[].b"]);
        }
        // addPaths: the objects only referenced; baseColor stores the object's material.
        "keyingSet.addPaths" => {
            rests.scene(&["keyingSets"]);
            let color = property_list(params).is_some_and(|list| list.contains(&"baseColor"));
            for id in rests.objects_or_selection() {
                rests.of(id, &[]);
                if color {
                    rests.of(id, &["material"]);
                    if let Some(material) = rests.slot(id) {
                        rests.of(material, &[]);
                    }
                }
            }
        }
        // action.new copies the keyed channels of `object` (never the active one).
        "action.new" => {
            for id in rests.named("object").unwrap_or_default() {
                rests.of(id, &["animation"]);
            }
        }
        // assign: the object's own keys replaced (an armature's bone channels need its bones).
        "action.assign" => {
            rests.scene(&["actions"]);
            for id in rests.objects_or_selection() {
                let armature = scene.object(id).is_ok_and(|object| matches!(object.data, ObjectData::Armature(_)));
                rests.of(id, if armature { &["animation", "data"] } else { &["animation"] });
            }
        }
        // pushDown names the action after the object when no name is given.
        "nla.pushDown" if !rests.has("name") => {
            for id in subjects {
                rests.of(*id, &["name"]);
            }
        }
        // A pose key with no properties keys what the active keying set names.
        "pose.insertKey" if property_list(params).is_none() && !rests.has("keyingSet") => rests.scene(&["keyingSets"]),
        // bone.set: the session's armature wins over `object`; a rename renames the bone's
        // channels; a custom shape only has to exist.
        "bone.set" => {
            let owners = rests.editor.rig.map_or_else(|| rests.object_or(rests.editor.active), |id| vec![id]);
            for id in owners {
                rests.of(id, if rests.has("name") { &["animation", "data"] } else { &["data"] });
            }
            rests.exist(&["shape"]);
        }
        "object.parentArmature" => parent_armature(rests),
        "shapeKey.add" => {
            let from = rests.named("from").unwrap_or_default();
            for id in subjects {
                rests.of(*id, if from.is_empty() { &["~topology"] } else { &["mesh"] });
            }
            for id in from {
                rests.of(id, &["mesh"]);
            }
        }
        // set: a value, or a rename (which renames the key's channels).
        "shapeKey.set" => {
            for id in subjects {
                rests.of(*id, if rests.has("rename") { &["animation", "mesh"] } else { &["mesh"] });
            }
        }
        "constraint.add" | "constraint.set" | "constraint.setInverse" | "constraint.clearInverse" => constraint(rests, command, subjects),
        // paint.vertices: draw replaces weights by vertex index, add and subtract read them.
        "paint.vertices" => {
            let reads = matches!(rests.text("mode"), Some("add" | "subtract"));
            for id in subjects {
                rests.of(*id, if reads { &["mesh"] } else { &["~topology"] });
            }
        }
        "particles.add" => {
            for id in subjects {
                rests.of(*id, &[]);
            }
            rests.exist(&["instanceObject", "colliders", "boids.rules[].target"]);
        }
        // set: system by index (0 by default) or name, its settings merged; a change of how the
        // particles are made drops the groom.
        "particles.set" | "particles.remove" => {
            let path = system_path(params);
            let regroom = command == "particles.set"
                && ["kind", "seed", "count", "emitFrom", "distribution", "segments", "densityGroup"].iter().any(|key| rests.has(key));
            for id in subjects {
                rests.of(*id, &[path.as_str()]);
                if regroom {
                    rests.of(*id, &["hair"]);
                }
            }
            rests.exist(&["instanceObject", "colliders", "boids.rules[].target"]);
        }
        // convert bakes the system at the current frame into a world-space mesh named after the
        // emitter: its evaluated mesh, world, material, the system and groom; every other visible
        // object with collision or a force field acts on the particles.
        "particles.convert" => {
            let path = system_path(params);
            for id in subjects {
                rests.world(*id);
                rests.of(*id, &["hair", "hidden", "material", "mesh", "modifiers", "name", path.as_str()]);
            }
            for object in scene.objects.iter().filter(|object| !subjects.contains(&object.id)) {
                rests.of(object.id, &["hidden", "physics"]);
            }
        }
        // Particle Edit: the session's emitter, its world and evaluated mesh, and the groom.
        "particleEdit.stroke" | "particleEdit.reset" => {
            if let Some((id, system)) = rests.editor.groom {
                let path = format!("particles.{system}");
                if command == "particleEdit.stroke" {
                    rests.world(id);
                    rests.of(id, &["hair", "mesh", "modifiers", path.as_str()]);
                } else {
                    rests.of(id, &["hair"]);
                }
            }
        }
        "physics.constraint.add" | "physics.constraint.set" => rests.exist(&["object1", "object2"]),
        "nodes.newGroup"
        | "nodes.removeGroup"
        | "nodes.renameGroup"
        | "nodes.addNode"
        | "nodes.removeNodes"
        | "nodes.moveNodes"
        | "nodes.setValue"
        | "nodes.link"
        | "nodes.unlink"
        | "nodes.addInput"
        | "nodes.addOutput"
        | "nodes.removeInput"
        | "nodes.removeOutput"
        | "nodes.addZoneItem"
        | "nodes.removeZoneItem" => nodes(rests, command),
        "compositor.new" | "compositor.set" | "compositor.setImage" | "compositor.pickMatte" => compositor(rests, command),
        "camera.move" | "camera.shake" | "camera.handheld" => camera(rests, command),
        // light.studio aims from the scene camera's world place at a point or an object's world
        // bounds as the scene stands; it sets the world's strength, AO and colour management.
        "light.studio" => {
            rests.scene(&["camera", "render", "world"]);
            if let Some(camera) = rests.camera() {
                rests.world(camera);
            }
            for id in point_object(rests, "target") {
                rests.bounds(id, false);
            }
        }
        // hop: the objects' world bounds at `start`; `to` an object's at the landing frame.
        "object.hop" => {
            rests.scene(&["timeline"]);
            for id in subjects {
                rests.bounds(*id, true);
                rests.placed(*id, &["tweak"]);
            }
            for id in point_object(rests, "to") {
                rests.bounds(id, true);
            }
        }
        // gather: each object's local place at `start`; `radius` 0 packs by world bounds; `center`
        // an object's world bounds' centre.
        "object.gather" => {
            rests.scene(&["timeline"]);
            let packs = params::number(params, "radius").ok().flatten() == Some(0.0);
            for id in subjects {
                if packs {
                    rests.bounds(*id, true);
                }
                rests.placed(*id, &["animation", "nla", "tweak"]);
                // Its world place: its fields through the parent's frame.
                rests.parent_frame(*id);
            }
            for id in point_object(rests, "center") {
                rests.bounds(id, true);
            }
        }
        // wave: each object's world bounds' centre from `origin` (a point or an object's); colour
        // keys go to the object's material (its base colour read).
        "object.wave" => {
            rests.scene(&["timeline"]);
            let color = rests.text("property") == Some("color");
            for id in subjects {
                rests.bounds(*id, true);
                if color {
                    rests.of(*id, &["material"]);
                    if let Some(material) = rests.slot(*id) {
                        rests.of(material, &["animation", "data"]);
                    }
                } else {
                    rests.of(*id, &["tweak"]);
                }
            }
            for id in point_object(rests, "origin") {
                rests.bounds(id, true);
            }
        }
        _ => {}
    }
    elements(rests, command);
    for value in crate::commands_nodes::value_objects(scene, command, params) {
        rests.of(value, &[]);
    }
}

/// An edit-mode command's `elements` (or, without them, the edited mesh): the vertex and face
/// indexes stay valid while the mesh's topology does. The knife projects through the object's world
/// place; separate copies its place, material and modifiers (and its name when no name is given).
fn elements(rests: &mut Rests, command: &str) {
    if !ELEMENT_ONLY.contains(&command) && command != "edit.delete" {
        return;
    }
    let id = match rests.params.get("elements").filter(|value| !value.is_null()) {
        Some(elements) => elements.get("object").and_then(|value| id_of(rests.scene, value)),
        // Without elements, the edited mesh and its selected elements (`mode`).
        None => rests.editor.edited,
    };
    let Some(id) = id else { return };
    rests.of(id, &["~topology"]);
    match command {
        "mesh.knife" => rests.world(id),
        "mesh.separate" => {
            rests.placed(id, &["material", "modifiers"]);
            if !rests.has("name") {
                rests.of(id, &["name"]);
            }
        }
        _ => {}
    }
}

/// The path of the particle system `system` names: by index (0 by default), the list by name.
fn system_path(params: &Value) -> String {
    match params.get("system") {
        None | Some(Value::Null) => "particles.0".to_owned(),
        Some(Value::Number(number)) => number.as_u64().map_or_else(|| "particles".to_owned(), |index| format!("particles.{index}")),
        Some(_) => "particles".to_owned(),
    }
}

/// The object a point-or-object parameter names (a number, its digits, or a name), if it names
/// one rather than a point.
fn point_object(rests: &Rests, key: &str) -> Vec<Id> {
    rests
        .params
        .get(key)
        .filter(|value| !value.is_array() && !value.is_null() && !value.is_object())
        .and_then(|value| id_of(rests.scene, value))
        .into_iter()
        .collect()
}

/// anim.insertKey: `keyingSet`, else (with no `properties`) the scene's active keying set, else the
/// properties (location, rotation and scale by default), each keyed at its current value into the
/// object's channels at the tweak-mapped frame; baseColor on the object's material.
fn insert_key(rests: &mut Rests, subjects: &[Id]) {
    let properties = property_list(rests.params);
    let set = rests.text("keyingSet");
    let (fields, placed): (&[&str], bool) = match set {
        Some("location" | "rotation" | "scale" | "locRot" | "locScale" | "locRotScale" | "rotScale") => (&["animation", "nla", "tweak"], true),
        Some("wholeCharacter") => (&["animation", "data", "nla", "tweak"], false),
        Some(_) => {
            rests.scene(&["keyingSets"]);
            (&["animation", "data", "hidden", "nla", "tweak"], true)
        }
        None if properties.is_none() => {
            rests.scene(&["keyingSets"]);
            (&["animation", "nla", "tweak"], true)
        }
        None => (&[], false),
    };
    for id in subjects {
        if set.is_some() || properties.is_none() {
            if placed {
                rests.placed(*id, fields);
            } else {
                rests.of(*id, fields);
            }
            continue;
        }
        for property in properties.iter().flatten() {
            if *property == "baseColor" {
                rests.color_key(*id, true);
                continue;
            }
            let (fields, placed) = key_fields(property);
            if placed {
                rests.placed(*id, fields);
            } else {
                rests.of(*id, fields);
            }
        }
    }
}

/// Sculpt strokes, gestures and face-set visibility: by screen pixels, the surface under the
/// pointer in the viewport picture (every visible object drawn at the current frame can occlude;
/// the multires level's faces through `modifiers`); by world points, the world matrix only.
fn sculpt(rests: &mut Rests, command: &str, subjects: &[Id]) {
    let params = rests.params;
    let pixels = |key: &str| {
        params.get(key).and_then(Value::as_array).is_some_and(|points| points.iter().any(|point| point.as_array().is_some_and(|xy| xy.len() == 2)))
    };
    let (picture, fields): (bool, &[&str]) = match command {
        "sculpt.stroke" => (pixels("points"), &["mesh", "modifiers"]),
        "sculpt.gesture" => (params::boolean(params, "frontFacesOnly").ok().flatten() == Some(true), &["mesh"]),
        "sculpt.faceSetVisibility" => match rests.text("action") {
            Some("showAll") => {
                for id in subjects {
                    rests.of(*id, &[]);
                }
                return;
            }
            _ if rests.has("faceSet") => {
                for id in subjects {
                    rests.of(*id, &["mesh"]);
                }
                return;
            }
            _ => (true, &["mesh", "modifiers"]),
        },
        _ => (true, &["mesh", "modifiers"]),
    };
    for id in subjects {
        rests.world(*id);
        rests.of(*id, fields);
    }
    if picture {
        rests.picture(subjects.first().copied());
    }
}

/// object.parentArmature: `armature` else the active object, whose world gives the parent inverse;
/// `objects` else the selected meshes, each given an Armature modifier and (by `weights`) groups:
/// automatic weights from both worlds and the bones, empty ones from the bone names and vertex
/// count, none without a bind.
fn parent_armature(rests: &mut Rests) {
    let scene = rests.scene;
    let armature = rests.named("armature").map_or(rests.editor.active, |ids| ids.first().copied());
    let children: Vec<Id> = rests.named("objects").unwrap_or_else(|| {
        rests
            .editor
            .selected
            .iter()
            .copied()
            .filter(|id| Some(*id) != armature && scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
            .collect()
    });
    let weights = rests.text("weights").unwrap_or("automatic");
    if let Some(armature) = armature {
        rests.world(armature);
        if weights != "none" {
            rests.of(armature, &["data"]);
        }
    }
    for child in children {
        // Keep Transform: an old parent's frame goes into the fields.
        rests.parent_frame(child);
        match weights {
            "none" => rests.of(child, &["modifiers"]),
            "empty" => rests.of(child, &["mesh", "modifiers"]),
            _ => {
                rests.world(child);
                rests.of(child, &["mesh", "modifiers"]);
            }
        }
    }
}

/// Constraints: an object's own stack (one by index, else the stack by name), or a bone's (in the
/// armature's data); a target only has to exist, but a Child Of's inverse reads its target's world,
/// a Stretch To's rest length both worlds, and an Action constraint names a scene action.
fn constraint(rests: &mut Rests, command: &str, subjects: &[Id]) {
    let scene = rests.scene;
    // A bone named, or the pose session's active bone when the owner is its armature.
    let bone = rests.has("bone") || rests.editor.posed.is_some_and(|(id, active)| active && subjects == [id]);
    let path = match rests.index("index") {
        Some(index) if command != "constraint.add" => format!("constraints.{index}"),
        _ => "constraints".to_owned(),
    };
    let owned: [&str; 1] = [if bone { "data" } else { path.as_str() }];
    for id in subjects {
        rests.of(*id, &owned);
    }
    rests.exist(&["target", "pole", "targets[].object"]);
    match command {
        "constraint.add" if !bone => match rests.text("type") {
            Some("childOf") => {
                for target in rests.named("target").unwrap_or_default() {
                    rests.world(target);
                }
            }
            Some("stretchTo") => {
                for id in subjects.iter().copied().chain(rests.named("target").unwrap_or_default()) {
                    rests.world(id);
                }
            }
            Some("action") => rests.scene(&["actions"]),
            _ => {}
        },
        // setInverse: the Child Of's inverse from its target's world.
        "constraint.setInverse" if !bone => {
            let index = rests.index("index").and_then(|index| usize::try_from(index).ok());
            for id in subjects {
                let targets: Vec<Id> = scene
                    .object(*id)
                    .ok()
                    .and_then(|object| object.constraints.get(index.unwrap_or(0)))
                    .map(|constraint| constraint.kind.targets().into_iter().map(|target| target.object).collect())
                    .unwrap_or_default();
                for target in targets {
                    rests.world(target);
                }
            }
        }
        _ => {}
    }
}

/// Node groups: the group exists (or its interface, links or nodes are read), the nodes named by id
/// whole; newGroup may give the group to an object's modifier, a material's shader or the world.
fn nodes(rests: &mut Rests, command: &str) {
    if command == "nodes.newGroup" {
        for id in rests.named("object").unwrap_or_default() {
            match rests.index("modifier") {
                Some(index) => rests.of(id, &[format!("modifiers.{index}").as_str()]),
                None => rests.of(id, &[]),
            }
        }
        if let Some(material) = rests.material("material") {
            rests.of(material, &["data"]);
        }
        if params::boolean(rests.params, "world").ok().flatten() == Some(true) {
            rests.scene(&["world"]);
        }
        return;
    }
    let fields: &[&str] = match command {
        "nodes.renameGroup" => &["name"],
        "nodes.setValue" => &["inputs"],
        "nodes.link" => &["inputs", "links", "outputs"],
        "nodes.unlink" => &["links"],
        "nodes.removeInput" => &["inputs"],
        "nodes.removeOutput" => &["outputs"],
        "nodes.addZoneItem" | "nodes.removeZoneItem" => &["nodes"],
        _ => &[],
    };
    if let Some(group) = rests.group() {
        rests.of(group, fields);
    }
    rests.nodes(&["node", "nodes", "from", "to"]);
    if command == "nodes.addInput" && matches!(rests.text("type"), Some("object" | "material")) {
        rests.exist(&["default"]);
    }
}

/// Compositing: `render.compositor` and the compositing flag; setImage's group defaults to the one
/// renders use; pickMatte reads the name under a point of a render of the whole scene through its
/// camera at the current frame.
fn compositor(rests: &mut Rests, command: &str) {
    let named = rests.has("group");
    match command {
        "compositor.new" if params::boolean(rests.params, "use").ok().flatten() != Some(false) => rests.scene(&["render"]),
        "compositor.set" => rests.scene(&["render"]),
        "compositor.setImage" if !named => {
            rests.scene(&["render"]);
            if let Some(group) = rests.scene.render.compositor {
                rests.of(group, &[]);
            }
        }
        // pickMatte: the scene's camera and the render's size the ray is cast through; what is
        // under the point is a place (`commands_compositor::matte_under`, C8-scene-engine 5b).
        "compositor.pickMatte" => rests.scene(&["camera", "render"]),
        _ => {}
    }
    if named && let Some(group) = rests.params.get("group").and_then(|value| any_id(rests.scene, "group", value)) {
        rests.of(group, &[]);
    }
    rests.nodes(&["node"]);
}

/// Camera moves: `camera` else the scene's camera, at `start` (else the current frame): its lens,
/// clip and place evaluated there, keys of its place written; `target`/`to` a point or an object's
/// world bounds at `start`; the timeline's end widened; a whip turns on motion blur. shake and
/// handheld overlay the camera's local place between its nearest keys.
fn camera(rests: &mut Rests, command: &str) {
    rests.scene(&["timeline"]);
    let camera = match rests.named("camera") {
        Some(ids) => ids,
        None => {
            rests.scene(&["camera"]);
            rests.camera().into_iter().collect()
        }
    };
    if command != "camera.move" {
        for id in camera {
            rests.placed(id, &["animation", "nla", "tweak"]);
        }
        return;
    }
    for id in camera {
        rests.world(id);
        rests.placed(id, &["animation", "data", "nla", "tweak"]);
    }
    if rests.text("preset") == Some("whip") {
        rests.scene(&["render"]);
    }
    for key in ["target", "to"] {
        for id in point_object(rests, key) {
            rests.bounds(id, true);
        }
    }
}

/// What a call of `command` with `params` (a world point already resolved into `object` or
/// `objects`) rests on, object by object (`None`: no rule; the guard keeps its default).
pub fn rests(scene: &Scene, editor: Editor, command: &str, params: &Value) -> Option<BTreeMap<On, Need>> {
    let rule = RULES.iter().find(|rule| rule.commands.contains(&command));
    if rule.is_none() && !DYNAMIC_WHO.iter().any(|(commands, _)| commands.contains(&command)) {
        return None;
    }
    let mut rests = Rests { scene, editor, params, found: BTreeMap::new() };
    let subjects = match rule {
        Some(rule) => {
            let subjects = rests.who(rule.who);
            for id in &subjects {
                match rule.at {
                    At::Here => rests.of(*id, rule.fields),
                    At::Own => rests.placed(*id, rule.fields),
                    At::World => {
                        rests.world(*id);
                        rests.of(*id, rule.fields);
                    }
                }
            }
            rests.scene(rule.scene);
            subjects
        }
        None => DYNAMIC_WHO.iter().find(|(commands, _)| commands.contains(&command)).map(|(_, who)| rests.who(*who)).unwrap_or_default(),
    };
    dynamic(&mut rests, command, &subjects);
    drivers(&mut rests);
    Some(rests.found)
}

/// Whether `need` reads the value of a driven `property`: a place (location, rotation, scale) when
/// it depends on where the object is, else the field holding the value (`hidden`; `data` for a
/// light's, a camera's or a bone's, a material's base colour; `mesh` for a shape key's).
fn reads_driven(need: &Need, property: Property) -> bool {
    let field = match property {
        Property::Location | Property::Rotation | Property::Scale => return need.positions.as_ref().is_none_or(|positions| !positions.is_empty()),
        Property::Hidden => "hidden",
        Property::ShapeKey => "mesh",
        Property::Power
        | Property::Color
        | Property::Lens
        | Property::BaseColor
        | Property::BoneLocation
        | Property::BoneRotation
        | Property::BoneScale => "data",
    };
    need.fields.as_ref().is_none_or(|fields| fields.iter().any(|held| held == field || held.starts_with(&format!("{field}."))))
}

/// Driven values (C8-scene-engine 4b): every commit, and every evaluation at another frame,
/// re-evaluates the drivers, so a call that reads a driven value rests on what the driver's
/// variables read: each source object's value (its place, its world place, or the field holding
/// the property), and, when the call evaluates the driven object over time (it reads its
/// `animation`), the source's keys too. A source's own drivers are followed the same way.
fn drivers(rests: &mut Rests) {
    let scene = rests.scene;
    if !scene.animation.channels.iter().any(|channel| channel.driver.is_some()) {
        return;
    }
    let mut done: BTreeSet<(usize, bool)> = BTreeSet::new();
    // A few rounds: each may make a source read whose own value is driven.
    for _ in 0..=scene.animation.channels.len().min(64) {
        let mut found = Vec::new();
        for (slot, channel) in scene.animation.channels.iter().enumerate() {
            let Some(driver) = &channel.driver else { continue };
            let Some(need) = rests.found.get(&On::Id(channel.target)) else { continue };
            if !reads_driven(need, channel.property) {
                continue;
            }
            let over_time = need.fields.as_ref().is_none_or(|fields| fields.iter().any(|field| field == "animation"));
            if done.insert((slot, over_time)) {
                found.push((driver, over_time));
            }
        }
        if found.is_empty() {
            break;
        }
        for (driver, over_time) in found {
            for variable in &driver.variables {
                source(rests, &variable.source, over_time);
            }
        }
    }
}

/// What a driver's variable reads of its sources.
fn source(rests: &mut Rests, source: &Source, over_time: bool) {
    let mut targets: Vec<(Id, bool, bool)> = Vec::new();
    match source {
        Source::SingleProperty { target: Some(id), property, .. } => {
            match property {
                Property::Location | Property::Rotation | Property::Scale => rests.placed(*id, &[]),
                Property::Hidden => rests.of(*id, &["hidden"]),
                Property::ShapeKey => rests.of(*id, &["mesh"]),
                _ => rests.of(*id, &["data"]),
            }
            if over_time {
                rests.of(*id, &["animation", "nla"]);
            }
        }
        Source::SingleProperty { target: None, .. } => {}
        // A transform in its own values reads the place (a bone's pose in the armature's data);
        // in world or local space (constraints included) the world place.
        Source::Transform { target: Some(target), space, .. } => {
            targets.push((target.object, target.bone.is_some(), *space != VariableSpace::Transform))
        }
        Source::Transform { target: None, .. } => {}
        Source::RotationDifference { a, b } | Source::Distance { a, b } => {
            targets.extend(a.iter().chain(b.iter()).map(|target| (target.object, target.bone.is_some(), true)));
        }
    }
    for (id, bone, world) in targets {
        if world {
            rests.world(id);
        } else {
            rests.placed(id, &[]);
        }
        if bone {
            rests.of(id, &["data"]);
        }
        if over_time {
            rests.of(id, &["animation", "nla"]);
        }
    }
}

/// Commands whose whole rule is their parameters' ([`dynamic`]), with who they act on.
const DYNAMIC_WHO: &[(&[&str], Who)] = &[
    (
        &[
            "object.applyTransform",
            "object.originToGeometry",
            "edit.duplicate",
            "material.assign",
            "anim.insertKey",
            "anim.deleteKey",
            "anim.clear",
            "anim.setInterpolation",
            "object.clearParent",
            "object.hop",
            "object.gather",
            "object.wave",
        ],
        Who::Objects,
    ),
    (&["object.parent", "object.reveal", "object.join", "material.delete", "material.set", "nodes.removeGroup"], Who::Nobody),
    (&["mode.set", "mode.toggle", "mode.armature", "mode.pose", "mode.weightPaint", "mode.sculpt", "mode.particleEdit", "mode.shape"], Who::Object),
    (&["edit.delete"], Who::Objects),
    (
        &[
            "modifier.remove",
            "modifier.setImage",
            "modifier.set",
            "modifier.apply",
            "driver.add",
            "driver.set",
            "driver.remove",
            "particles.add",
            "particles.set",
            "particles.remove",
            "particles.convert",
        ],
        Who::Object,
    ),
    (
        &["sculpt.stroke", "sculpt.gesture", "sculpt.faceSetVisibility", "sculpt.hideFaceSet", "sculpt.isolateFaceSet", "sculpt.dyntopo"],
        Who::Sculpted,
    ),
    (
        &[
            "bone.set",
            "object.parentArmature",
            "keyingSet.addPaths",
            "action.assign",
            "particleEdit.stroke",
            "particleEdit.reset",
            "camera.move",
            "camera.shake",
            "camera.handheld",
            "light.studio",
        ],
        Who::Nobody,
    ),
    (
        &[
            "nodes.renameGroup",
            "nodes.addNode",
            "nodes.removeNodes",
            "nodes.moveNodes",
            "nodes.setValue",
            "nodes.link",
            "nodes.unlink",
            "nodes.addInput",
            "nodes.addOutput",
            "nodes.removeInput",
            "nodes.removeOutput",
            "nodes.addZoneItem",
            "nodes.removeZoneItem",
            "compositor.set",
            "compositor.setImage",
            "compositor.pickMatte",
        ],
        Who::Nobody,
    ),
    (&ELEMENT_ONLY, Who::Nobody),
];

/// The edit-mode commands whose object is the elements' (or the edited mesh).
const ELEMENT_ONLY: [&str; 34] = [
    "mesh.extrude",
    "mesh.inset",
    "mesh.bevel",
    "mesh.subdivide",
    "mesh.loopCut",
    "mesh.merge",
    "mesh.deleteFaces",
    "mesh.deleteVertices",
    "mesh.fill",
    "mesh.flip",
    "mesh.translate",
    "mesh.scale",
    "mesh.extrudeIndividual",
    "mesh.dissolveVertices",
    "mesh.dissolveEdges",
    "mesh.dissolveFaces",
    "mesh.bridge",
    "mesh.bisect",
    "mesh.separate",
    "mesh.symmetrize",
    "mesh.spin",
    "mesh.bevelVertices",
    "mesh.rotate",
    "mesh.knife",
    "mesh.edgeSlide",
    "mesh.markSeam",
    "mesh.clearSeam",
    "uv.unwrap",
    "uv.stitch",
    "uv.pin",
    "uv.unpin",
    "uv.translate",
    "uv.rotate",
    "uv.scale",
];
