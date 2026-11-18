//! Group nodes and zones. A Group node runs another group of the scene with the node's inputs
//! as that group's Group Input and gives what reaches its Group Output. A repeat zone runs the
//! nodes between its input and its output a number of times, each time on what the last time
//! gave; a simulation zone runs them once a frame from the scene's start frame, each frame on what
//! the frame before gave (the zone's inputs on the start frame), and passes its inputs through
//! before the start frame.

use std::collections::BTreeSet;
use std::sync::Arc;

use kition_scene_model::node_tree::Tree;
use kition_scene_model::node_tree::{self, Node, ZoneItem};
use kition_scene_model::{Id, SocketType};
use serde_json::Value;

use crate::eval::{Data, Evaluator, Outputs};
use crate::field::{Field, Scalar};
use crate::geometry::{Geometry, MAX_ELEMENTS};

/// How many Group nodes deep groups may run each other.
pub const MAX_GROUP_DEPTH: u32 = 32;
/// The most times a repeat zone runs.
pub const MAX_ITERATIONS: i32 = 100_000;
/// The most frames a simulation zone runs from the start frame.
pub const MAX_FRAMES: i32 = 10_000;

fn too_big(outputs: &Outputs) -> bool {
    outputs.values().any(|data| matches!(data, Data::Geometry(geometry) if geometry.size() > MAX_ELEMENTS))
}

impl<'a> Evaluator<'a> {
    /// A Group node: the used group run on the node's inputs.
    pub(crate) fn group_node(&mut self, node: &Node) -> Result<Outputs, String> {
        let Some(id) = node_tree::used_group(node) else { return Ok(Outputs::new()) };
        if self.depth >= MAX_GROUP_DEPTH {
            return Err("node groups use each other too deeply".into());
        }
        let scene = self.call.scene;
        let inner = scene.node_group(id).ok_or_else(|| format!("the node group {id} a Group node uses is missing"))?;
        if inner.tree != Tree::Geometry {
            return Err(format!("{} is not a geometry node group", inner.name));
        }
        if node_tree::uses_group(&scene.node_groups, inner.id, inner.id) || inner.id == self.group.id {
            return Err(format!("{} uses itself", inner.name));
        }
        let mut interface = Outputs::new();
        for (socket, kind) in node_tree::group_input_sockets(inner) {
            let default = inner.inputs.iter().find(|input| input.name == socket).map_or(Value::Null, |input| input.default.clone());
            let data = self.socket(node, &socket, kind, &default)?;
            interface.insert(socket, data);
        }
        let input = match interface.get("Geometry") {
            Some(Data::Geometry(geometry)) => geometry.clone(),
            _ => Arc::new(Geometry::default()),
        };
        let prefix = format!("{}{}/", self.prefix, node.id);
        let mut nested = self.nested(inner, input, Some(Arc::new(interface)), prefix, self.depth + 1, self.frame);
        nested.run_outputs()
    }

    /// The nodes of the zone from `input` to `output`: those reached from the input that lead
    /// to the output. A link from them to a node outside the zone is refused.
    fn zone_body(&self, input: Id, output: Id) -> Result<BTreeSet<Id>, String> {
        let links = &self.group.links;
        let mut after = BTreeSet::new();
        let mut pending = vec![input];
        while let Some(at) = pending.pop() {
            for link in links.iter().filter(|link| link.from == at) {
                if link.to != output && after.insert(link.to) {
                    pending.push(link.to);
                }
            }
        }
        let mut before = BTreeSet::new();
        let mut pending = vec![output];
        while let Some(at) = pending.pop() {
            for link in links.iter().filter(|link| link.to == at) {
                if link.from != input && before.insert(link.from) {
                    pending.push(link.from);
                }
            }
        }
        let body: BTreeSet<Id> = after.intersection(&before).copied().collect();
        for link in links {
            let inside = link.from == input || body.contains(&link.from);
            if inside && link.to != output && !body.contains(&link.to) {
                return Err("a link leaves a zone other than through its output".into());
            }
        }
        Ok(body)
    }

    /// Refuses a simulation zone inside another zone (directly, or in a group a Group node in the
    /// zone runs): it would need a state per run of the outer zone.
    fn no_simulation_in(&self, body: &BTreeSet<Id>) -> Result<(), String> {
        let groups = &self.call.scene.node_groups;
        let simulates = |group: &kition_scene_model::NodeGroup| group.nodes.iter().any(|node| node.kind == "simulationOutput");
        for node in self.group.nodes.iter().filter(|node| body.contains(&node.id)) {
            let nested = node_tree::used_group(node)
                .is_some_and(|id| groups.iter().any(|group| (group.id == id || node_tree::uses_group(groups, id, group.id)) && simulates(group)));
            if matches!(node.kind.as_str(), "simulationInput" | "simulationOutput") || nested {
                return Err("a simulation zone cannot be inside another zone".into());
            }
        }
        Ok(())
    }

    /// The values a zone input's items get from its own inputs.
    fn zone_inputs(&mut self, input: &Node, items: &[ZoneItem]) -> Result<Outputs, String> {
        let mut state = Outputs::new();
        for item in items {
            let data = self.socket(input, &item.name, item.kind, &Value::Null)?;
            state.insert(item.name.clone(), data);
        }
        Ok(state)
    }

    /// Runs a zone's body once: the input node gives `given`, and what reaches the output's
    /// items comes back.
    fn zone_pass(&mut self, input: Id, output: &Node, body: &BTreeSet<Id>, given: Outputs, items: &[ZoneItem]) -> Result<Outputs, String> {
        for id in body {
            self.done.remove(id);
        }
        self.done.insert(input, Arc::new(given));
        let mut next = Outputs::new();
        for item in items {
            let data = self.socket(output, &item.name, item.kind, &Value::Null)?;
            next.insert(item.name.clone(), data);
        }
        for id in body {
            self.done.remove(id);
        }
        self.done.remove(&input);
        if too_big(&next) {
            return Err(format!("the zone would make more than {MAX_ELEMENTS} elements"));
        }
        Ok(next)
    }

    /// A zone input node met outside its zone's run (its zone runs no time, or it has no
    /// output): its own inputs, at the first iteration.
    pub(crate) fn zone_input(&mut self, node: &Node) -> Result<Outputs, String> {
        let items = self.group.zone_items(node);
        let mut out = self.zone_inputs(node, &items)?;
        let first = if node.kind == "repeatInput" { ("Iteration", Scalar::Int(0)) } else { ("Delta Time", Scalar::Float(0.0)) };
        out.insert(first.0.into(), Data::Field(Field::Constant(first.1)));
        Ok(out)
    }

    /// A repeat zone's output: the body run `Iterations` times.
    pub(crate) fn repeat_output(&mut self, node: &Node) -> Result<Outputs, String> {
        let input = self.group.zone_input_of(node).cloned().ok_or("the repeat zone has no Repeat Input")?;
        let items = node_tree::items_of(node);
        let iterations = match self.socket(&input, "Iterations", SocketType::Int, &Value::from(1))? {
            Data::Field(field) => crate::field::single(&field).int(),
            _ => 1,
        };
        if iterations > MAX_ITERATIONS {
            return Err(format!("a repeat zone runs at most {MAX_ITERATIONS} times"));
        }
        let body = self.zone_body(input.id, node.id)?;
        self.no_simulation_in(&body)?;
        let mut state = self.zone_inputs(&input, &items)?;
        for iteration in 0..iterations.max(0) {
            let mut given = state.clone();
            given.insert("Iteration".into(), Data::Field(Field::Constant(Scalar::Int(iteration))));
            state = self.zone_pass(input.id, node, &body, given, &items)?;
        }
        Ok(state)
    }

    /// A simulation zone's output at this evaluator's frame.
    pub(crate) fn simulation_output(&mut self, node: &Node) -> Result<Outputs, String> {
        let input = self.group.zone_input_of(node).cloned().ok_or("the simulation zone has no Simulation Input")?;
        let items = node_tree::items_of(node);
        let timeline = &self.call.scene.animation.timeline;
        let (start, fps) = (timeline.start, timeline.fps);
        let frame = if self.frame.is_finite() { self.frame.floor() as i32 } else { start };
        if frame < start {
            return self.zone_inputs(&input, &items);
        }
        if frame - start >= MAX_FRAMES {
            return Err(format!("a simulation zone runs at most {MAX_FRAMES} frames from the start frame"));
        }
        let prefix = self.prefix.clone();
        let key = |frame: i32| (prefix.clone(), node.id, frame);
        // The last frame worked out in this run, else the start.
        let mut at = start;
        let mut state: Option<Arc<Outputs>> = None;
        {
            let states = self.states.borrow();
            for earlier in (start..=frame).rev() {
                if let Some(found) = states.get(&key(earlier)) {
                    at = earlier + 1;
                    state = Some(found.clone());
                    break;
                }
            }
        }
        let body = self.zone_body(input.id, node.id)?;
        self.no_simulation_in(&body)?;
        let delta = if fps > 0.0 { 1.0 / fps } else { 0.0 };
        while at <= frame {
            let made = if at == frame {
                self.simulation_step(&input, node, &body, &items, state.as_deref(), if at == start { 0.0 } else { delta })?
            } else {
                let mut other = self.nested(self.group, self.input.clone(), self.interface.clone(), self.prefix.clone(), self.depth, at as f32);
                other.simulation_step(&input, node, &body, &items, state.as_deref(), if at == start { 0.0 } else { delta })?
            };
            let made = Arc::new(made);
            self.states.borrow_mut().insert(key(at), made.clone());
            state = Some(made);
            at += 1;
        }
        Ok(state.map(Arc::unwrap_or_clone).unwrap_or_default())
    }

    /// One frame of a simulation zone: the body run on the state before (the zone's inputs on
    /// the start frame), or that state when Skip is on.
    fn simulation_step(
        &mut self,
        input: &Node,
        output: &Node,
        body: &BTreeSet<Id>,
        items: &[ZoneItem],
        before: Option<&Outputs>,
        delta: f32,
    ) -> Result<Outputs, String> {
        let state = match before {
            Some(state) => state.clone(),
            None => self.zone_inputs(input, items)?,
        };
        let skip = match self.socket(output, "Skip", SocketType::Bool, &Value::Bool(false))? {
            Data::Field(field) => crate::field::single(&field).bool(),
            _ => false,
        };
        if skip {
            return Ok(state);
        }
        let mut given = state;
        given.insert("Delta Time".into(), Data::Field(Field::Constant(Scalar::Float(delta))));
        self.zone_pass(input.id, output, body, given, items)
    }
}
