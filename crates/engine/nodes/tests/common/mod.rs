//! Building node groups in tests, and running them on a mesh.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;

use kition_scene_model::modifiers::NodesCall;
use kition_scene_model::{Id, Link, Mesh, Modifier, ModifierKind, Node, NodeGroup, Object, ObjectData, Scene};
use serde_json::Value;

pub const GROUP: Id = 50;
pub const INPUT: Id = 51;
pub const OUTPUT: Id = 52;

/// A node group under construction.
pub struct Tree {
    pub group: NodeGroup,
    next: Id,
}

impl Tree {
    /// A group with only its Group Input and Group Output, not linked.
    pub fn new() -> Self {
        let mut group = NodeGroup::new(GROUP, "Geometry Nodes", INPUT, OUTPUT);
        group.links.clear();
        Self { group, next: 100 }
    }

    /// Another group (for Group nodes): its id, its Group Input and Output at `id + 1` and
    /// `id + 2`, its nodes from `id * 10`.
    pub fn other(id: Id, name: &str) -> Self {
        let mut group = NodeGroup::new(id, name, id + 1, id + 2);
        group.links.clear();
        Self { group, next: id * 10 }
    }

    /// The group's Group Input and Group Output.
    pub fn ends(&self) -> (Id, Id) {
        (self.group.id + 1, self.group.id + 2)
    }

    /// Adds a node of `kind` with `values` (a JSON object of socket and setting values).
    pub fn add(&mut self, kind: &str, values: Value) -> Id {
        let id = self.next;
        self.next += 1;
        let values: BTreeMap<String, Value> = match values {
            Value::Object(map) => map.into_iter().collect(),
            _ => BTreeMap::new(),
        };
        self.group.nodes.push(Node { id, kind: kind.into(), location: [0.0, 0.0], values });
        id
    }

    pub fn link(&mut self, from: Id, from_socket: &str, to: Id, to_socket: &str) {
        self.group.links.push(Link { from, from_socket: from_socket.into(), to, to_socket: to_socket.into() });
    }

    /// Links `from`'s `socket` to the group's output.
    pub fn output(&mut self, from: Id, socket: &str) {
        self.link(from, socket, OUTPUT, "Geometry");
    }

    /// A scene holding the group and one object (with `mesh`) using it.
    pub fn scene(&self, mesh: Mesh) -> (Scene, Id) {
        let mut scene = Scene::empty();
        scene.node_groups.push(self.group.clone());
        let object = scene.add_object("Object", ObjectData::Mesh(mesh));
        (scene, object)
    }

    /// Runs the group on `mesh`.
    pub fn run(&self, mesh: Mesh) -> Result<Mesh, String> {
        self.run_with(mesh, &BTreeMap::new())
    }

    /// Runs the group on `mesh` with modifier inputs.
    pub fn run_with(&self, mesh: Mesh, inputs: &BTreeMap<String, Value>) -> Result<Mesh, String> {
        let (scene, id) = self.scene(mesh.clone());
        let object: &Object = scene.object(id).unwrap();
        kition_scene_nodes::evaluate(&NodesCall { mesh: &mesh, object, scene: &scene, group: GROUP, inputs, depth: 0 })
    }

    /// Runs the group on `mesh` with `others` in the scene too, at `frame` (the scene's frame
    /// when none), and returns its output geometry as it is.
    pub fn run_in(&self, mesh: Mesh, others: &[NodeGroup], frame: Option<i32>) -> Result<kition_scene_nodes::geometry::Geometry, String> {
        let (mut scene, id) = self.scene(mesh.clone());
        scene.node_groups.extend(others.iter().cloned());
        if let Some(frame) = frame {
            scene.animation.timeline.frame = frame;
        }
        let object: &Object = scene.object(id).unwrap();
        let inputs = BTreeMap::new();
        let call = NodesCall { mesh: &mesh, object, scene: &scene, group: GROUP, inputs: &inputs, depth: 0 };
        let input = kition_scene_nodes::geometry::Geometry::from_mesh(mesh.clone());
        kition_scene_nodes::eval::Evaluator::new(&call, &self.group, input).run()
    }

    /// Runs the group on `mesh` and returns its output geometry as it is (curves, points, and
    /// instances not turned into a mesh).
    pub fn run_geometry(&self, mesh: Mesh) -> Result<kition_scene_nodes::geometry::Geometry, String> {
        let (scene, id) = self.scene(mesh.clone());
        let object: &Object = scene.object(id).unwrap();
        let inputs = BTreeMap::new();
        let call = NodesCall { mesh: &mesh, object, scene: &scene, group: GROUP, inputs: &inputs, depth: 0 };
        let input = kition_scene_nodes::geometry::Geometry::from_mesh(mesh.clone());
        kition_scene_nodes::eval::Evaluator::new(&call, &self.group, input).run()
    }
}

/// A Geometry Nodes modifier using the test group.
pub fn modifier() -> Modifier {
    Modifier { name: "GeometryNodes".into(), enabled: true, kind: ModifierKind::GeometryNodes { group: Some(GROUP), inputs: BTreeMap::new() } }
}
