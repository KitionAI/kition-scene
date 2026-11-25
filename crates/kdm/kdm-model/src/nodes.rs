//! Node groups in the replica: the scene's `nodeGroups` list holds each group as a record of its
//! own (a compositing or shader group names its `tree`; a geometry group none; `inputs` and
//! `outputs` are its interface, which Group nodes using it give), with its nodes and links as
//! records of their own, so an agent's draft and the person's edits of different nodes (or different
//! values of one node: `values` is a map) merge. Links are identified by their ends; the repair drops
//! links whose nodes or sockets are gone, a second link into an input that takes one, and a link
//! that closes a cycle, and lets go of groups, objects, and materials that modifiers and nodes
//! name but no record holds. Group nodes let go of a group that is gone, not a geometry group, or
//! that would use itself (a draft and the person each putting one group into the other: the later
//! Group node, in key order, lets go); zone inputs let go of an output that is gone or another input already
//! pairs with.

use std::collections::{BTreeMap, BTreeSet};

use kition_kit::kdm::{ObjectId, Replica, Tx};
use kition_scene_model::node_tree::{self, GroupSocket, Link, Node, NodeGroup, Tree};
use kition_scene_model::{Id, Scene};
use serde_json::{Map, Value};

use crate::{KdmResult, held, kdm_err, root_id};

fn key(value: &Value) -> Option<Id> {
    value.as_u64().and_then(|key| u32::try_from(key).ok())
}

/// The groups the replica holds, in order. Nodes of kinds this build does not know, and links
/// that do not fit their nodes, are left out.
pub fn read_groups(replica: &Replica) -> Result<Vec<NodeGroup>, String> {
    let field = |id: &ObjectId, name: &str| replica.get(id, name).unwrap_or(Value::Null);
    let mut groups: Vec<NodeGroup> = Vec::new();
    for id in replica.list(&root_id(), "nodeGroups").map_err(|error| error.to_string())? {
        let Some(group_key) = key(&field(&id, "key")) else { continue };
        if groups.iter().any(|group| group.id == group_key) {
            continue;
        }
        let name = field(&id, "name").as_str().unwrap_or("Geometry Nodes").to_owned();
        let sockets = |which: &str| -> Result<Vec<GroupSocket>, String> {
            match field(&id, which) {
                Value::Null => Ok(Vec::new()),
                value => serde_json::from_value(value).map_err(|error| format!("damaged: node group {name}'s {which}: {error}")),
            }
        };
        let (inputs, outputs) = (sockets("inputs")?, sockets("outputs")?);
        // A group of a tree this build does not know is left out.
        let Some(tree) = tree_of(&field(&id, "tree")) else { continue };
        let mut group = NodeGroup { id: group_key, name, tree, inputs, outputs, nodes: Vec::new(), links: Vec::new() };
        for node_id in replica.list(&id, "nodes").map_err(|error| error.to_string())? {
            let Some(node_key) = key(&field(&node_id, "key")) else { continue };
            let kind = field(&node_id, "kind").as_str().unwrap_or_default().to_owned();
            if !node_tree::in_tree(&kind, tree) || group.node(node_key).is_some() {
                continue;
            }
            let location: [f32; 2] = serde_json::from_value(field(&node_id, "location")).unwrap_or([0.0, 0.0]);
            let location = if location.iter().all(|value| value.is_finite()) { location } else { [0.0, 0.0] };
            let values = replica.map_entries(&node_id, "values").map_err(|error| error.to_string())?.into_iter().collect();
            group.nodes.push(Node { id: node_key, kind, location, values });
        }
        for link_id in replica.list(&id, "links").map_err(|error| error.to_string())? {
            let (Some(from), Some(to)) = (key(&field(&link_id, "from")), key(&field(&link_id, "to"))) else { continue };
            let from_socket = field(&link_id, "fromSocket").as_str().unwrap_or_default().to_owned();
            let to_socket = field(&link_id, "toSocket").as_str().unwrap_or_default().to_owned();
            group.links.push(Link { from, from_socket, to, to_socket });
        }
        groups.push(group);
    }
    // Group nodes' sockets follow the groups they use: fit links once every group is read.
    let fitted: Vec<Vec<Link>> = groups.iter().map(|group| fitting_links(group, &groups)).collect();
    for (group, links) in groups.iter_mut().zip(fitted) {
        group.links = links;
    }
    Ok(groups)
}

/// The tree a group record's `tree` names (geometry when absent); `None` for a tree this build
/// does not know.
fn tree_of(value: &Value) -> Option<Tree> {
    match value {
        Value::Null => Some(Tree::Geometry),
        Value::String(text) => Tree::parse(text),
        _ => None,
    }
}

/// The group's links that join sockets its nodes have (`groups` are the scene's, whose
/// interfaces give Group nodes their sockets), without repeats, without a second link
/// into an input that takes one, and without links that close a cycle (in list order: the
/// earlier link stays).
pub fn fitting_links(group: &NodeGroup, groups: &[NodeGroup]) -> Vec<Link> {
    let mut kept: Vec<Link> = Vec::new();
    let mut taken: BTreeSet<(Id, String)> = BTreeSet::new();
    for link in &group.links {
        let (Some(from), Some(to)) = (group.node(link.from), group.node(link.to)) else { continue };
        if !group.outputs_of(from, groups).iter().any(|(id, _)| *id == link.from_socket) {
            continue;
        }
        let Some((_, _, multi)) = group.inputs_of(to, groups).into_iter().find(|(id, _, _)| *id == link.to_socket) else { continue };
        if kept.contains(link) || (!multi && taken.contains(&(link.to, link.to_socket.clone()))) || reaches(&kept, link.to, link.from) {
            continue;
        }
        taken.insert((link.to, link.to_socket.clone()));
        kept.push(link.clone());
    }
    kept
}

/// Whether `links` lead from node `start` to node `goal`.
pub fn reaches(links: &[Link], start: Id, goal: Id) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(at) = pending.pop() {
        if at == goal {
            return true;
        }
        if seen.insert(at) {
            pending.extend(links.iter().filter(|link| link.from == at).map(|link| link.to));
        }
    }
    false
}

/// Every id the scene's groups use (their keys and their nodes' keys).
pub fn ids(scene: &Scene) -> impl Iterator<Item = Id> + '_ {
    scene.node_groups.iter().flat_map(|group| std::iter::once(group.id).chain(group.nodes.iter().map(|node| node.id)))
}

fn node_record(node: &Node) -> KdmResult<Map<String, Value>> {
    let mut fields = Map::new();
    fields.insert("key".into(), Value::from(node.id));
    fields.insert("kind".into(), Value::String(node.kind.clone()));
    fields.insert("location".into(), serde_json::to_value(node.location).map_err(kdm_err)?);
    Ok(fields)
}

fn link_record(link: &Link) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("from".into(), Value::from(link.from));
    fields.insert("fromSocket".into(), Value::String(link.from_socket.clone()));
    fields.insert("to".into(), Value::from(link.to));
    fields.insert("toSocket".into(), Value::String(link.to_socket.clone()));
    fields
}

fn sockets_value(sockets: &[GroupSocket]) -> KdmResult<Value> {
    if sockets.is_empty() { Ok(Value::Null) } else { serde_json::to_value(sockets).map_err(kdm_err) }
}

fn add_node(tx: &mut Tx<'_>, group_id: &ObjectId, node: &Node) -> KdmResult<ObjectId> {
    let id = tx.create("Node", node_record(node)?)?;
    for (name, value) in &node.values {
        tx.map_set(&id, "values", name, value.clone())?;
    }
    let length = tx.list_items(group_id, "nodes")?.len();
    tx.list_insert(group_id, "nodes", length, &id)?;
    Ok(id)
}

fn add_link(tx: &mut Tx<'_>, group_id: &ObjectId, link: &Link) -> KdmResult<ObjectId> {
    let id = tx.create("Link", link_record(link))?;
    let length = tx.list_items(group_id, "links")?.len();
    tx.list_insert(group_id, "links", length, &id)?;
    Ok(id)
}

fn add_group(tx: &mut Tx<'_>, group: &NodeGroup) -> KdmResult<()> {
    let mut fields = Map::new();
    fields.insert("key".into(), Value::from(group.id));
    fields.insert("name".into(), Value::String(group.name.clone()));
    if !group.tree.is_geometry() {
        fields.insert("tree".into(), Value::String(group.tree.id().to_owned()));
    }
    for (name, sockets) in [("inputs", &group.inputs), ("outputs", &group.outputs)] {
        let value = sockets_value(sockets)?;
        if !value.is_null() {
            fields.insert(name.into(), value);
        }
    }
    let id = tx.create("NodeGroup", fields)?;
    let length = tx.list_items(&root_id(), "nodeGroups")?.len();
    tx.list_insert(&root_id(), "nodeGroups", length, &id)?;
    for node in &group.nodes {
        add_node(tx, &id, node)?;
    }
    for link in &group.links {
        add_link(tx, &id, link)?;
    }
    Ok(())
}

fn delete_group(tx: &mut Tx<'_>, id: &ObjectId) -> KdmResult<()> {
    for field in ["nodes", "links"] {
        for item in tx.list_items(id, field)? {
            tx.delete(&item)?;
        }
    }
    remove_at(tx, &root_id(), "nodeGroups", id)?;
    tx.delete(id)
}

fn remove_at(tx: &mut Tx<'_>, parent: &ObjectId, field: &str, id: &ObjectId) -> KdmResult<()> {
    while let Some(index) = tx.list_items(parent, field)?.iter().position(|other| other == id) {
        tx.list_remove(parent, field, index)?;
    }
    Ok(())
}

/// Puts list `field` of `parent` in the order of `wanted`.
fn order(tx: &mut Tx<'_>, parent: &ObjectId, field: &str, wanted: &[ObjectId]) -> KdmResult<()> {
    let mut current = tx.list_items(parent, field)?;
    for (index, id) in wanted.iter().enumerate() {
        if current.get(index) == Some(id) {
            continue;
        }
        let Some(from) = current.iter().position(|other| other == id) else { continue };
        tx.list_move(parent, field, from, index)?;
        let moved = current.remove(from);
        current.insert(index.min(current.len()), moved);
    }
    Ok(())
}

/// Records of list `field` of `parent` by `make`'s key (the first of a repeated key wins).
fn by_key<K: Ord>(tx: &Tx<'_>, parent: &ObjectId, field: &str, make: impl Fn(&Tx<'_>, &ObjectId) -> Option<K>) -> KdmResult<BTreeMap<K, ObjectId>> {
    let mut out = BTreeMap::new();
    for id in tx.list_items(parent, field)? {
        if let Some(key) = make(tx, &id) {
            out.entry(key).or_insert(id);
        }
    }
    Ok(out)
}

fn record_key(tx: &Tx<'_>, id: &ObjectId) -> Option<Id> {
    tx.get(id, "key").ok().as_ref().and_then(key)
}

fn link_key(tx: &Tx<'_>, id: &ObjectId) -> Option<Link> {
    let text = |name: &str| tx.get(id, name).ok().and_then(|value| value.as_str().map(str::to_owned));
    Some(Link {
        from: key(&tx.get(id, "from").ok()?)?,
        from_socket: text("fromSocket")?,
        to: key(&tx.get(id, "to").ok()?)?,
        to_socket: text("toSocket")?,
    })
}

/// Writes the scene's groups in a new replica.
pub fn write_groups(tx: &mut Tx<'_>, scene: &Scene) -> KdmResult<()> {
    for group in &scene.node_groups {
        add_group(tx, group)?;
    }
    Ok(())
}

/// Writes what changed in the groups between `before` and `after`.
pub fn write_group_changes(tx: &mut Tx<'_>, before: &Scene, after: &Scene) -> KdmResult<()> {
    if before.node_groups == after.node_groups {
        return Ok(());
    }
    let records = group_records(tx);
    let old: BTreeMap<Id, &NodeGroup> = before.node_groups.iter().map(|group| (group.id, group)).collect();
    let new_keys: BTreeSet<Id> = after.node_groups.iter().map(|group| group.id).collect();
    for group in old.values().filter(|group| !new_keys.contains(&group.id)) {
        if let Some(id) = records.get(&group.id) {
            delete_group(tx, id)?;
        }
    }
    for group in &after.node_groups {
        match (old.get(&group.id), records.get(&group.id)) {
            (Some(previous), Some(id)) => {
                if *previous != group {
                    write_group(tx, id, previous, group)?;
                }
            }
            _ => add_group(tx, group)?,
        }
    }
    let records = group_records(tx);
    let wanted: Vec<ObjectId> = after.node_groups.iter().filter_map(|group| records.get(&group.id).cloned()).collect();
    order(tx, &root_id(), "nodeGroups", &wanted)
}

/// The group records by key (the first record of a repeated key, in list order, wins).
fn group_records(tx: &Tx<'_>) -> BTreeMap<Id, ObjectId> {
    let mut out = BTreeMap::new();
    for id in tx.list_items(&root_id(), "nodeGroups").unwrap_or_default() {
        if let Some(key) = record_key(tx, &id) {
            out.entry(key).or_insert(id);
        }
    }
    out
}

fn write_group(tx: &mut Tx<'_>, id: &ObjectId, before: &NodeGroup, after: &NodeGroup) -> KdmResult<()> {
    if before.name != after.name {
        tx.set(id, "name", Value::String(after.name.clone()))?;
    }
    if before.tree != after.tree {
        tx.set(id, "tree", if after.tree.is_geometry() { Value::Null } else { Value::String(after.tree.id().to_owned()) })?;
    }
    if before.inputs != after.inputs {
        tx.set(id, "inputs", sockets_value(&after.inputs)?)?;
    }
    if before.outputs != after.outputs {
        tx.set(id, "outputs", sockets_value(&after.outputs)?)?;
    }
    // Nodes: by key; a changed node writes only its changed location and values.
    let records = by_key(tx, id, "nodes", record_key)?;
    let old: BTreeMap<Id, &Node> = before.nodes.iter().map(|node| (node.id, node)).collect();
    let new_keys: BTreeSet<Id> = after.nodes.iter().map(|node| node.id).collect();
    for node in old.values().filter(|node| !new_keys.contains(&node.id)) {
        if let Some(record) = records.get(&node.id) {
            remove_at(tx, id, "nodes", record)?;
            tx.delete(record)?;
        }
    }
    for node in &after.nodes {
        match (old.get(&node.id), records.get(&node.id)) {
            (Some(previous), Some(record)) => {
                if previous.location != node.location {
                    tx.set(record, "location", serde_json::to_value(node.location).map_err(kdm_err)?)?;
                }
                for (name, value) in &node.values {
                    if previous.values.get(name) != Some(value) {
                        tx.map_set(record, "values", name, value.clone())?;
                    }
                }
                for name in previous.values.keys().filter(|name| !node.values.contains_key(*name)) {
                    tx.map_delete(record, "values", name)?;
                }
            }
            _ => {
                add_node(tx, id, node)?;
            }
        }
    }
    let records = by_key(tx, id, "nodes", record_key)?;
    let wanted: Vec<ObjectId> = after.nodes.iter().filter_map(|node| records.get(&node.id).cloned()).collect();
    order(tx, id, "nodes", &wanted)?;
    // Links: by their ends.
    let records = by_key(tx, id, "links", link_key)?;
    let new_links: BTreeSet<&Link> = after.links.iter().collect();
    for link in before.links.iter().filter(|link| !new_links.contains(link)) {
        if let Some(record) = records.get(link) {
            remove_at(tx, id, "links", record)?;
            tx.delete(record)?;
        }
    }
    for link in after.links.iter().filter(|link| !records.contains_key(*link)) {
        add_link(tx, id, link)?;
    }
    let records = by_key(tx, id, "links", link_key)?;
    let wanted: Vec<ObjectId> = after.links.iter().filter_map(|link| records.get(link).cloned()).collect();
    order(tx, id, "links", &wanted)
}

/// The post-merge repair of node groups; answers whether it wrote anything.
pub fn repair(tx: &mut Tx<'_>) -> KdmResult<bool> {
    let root = root_id();
    let mut wrote = false;
    let groups: Vec<ObjectId> = group_records(tx).into_values().collect();
    // Each group's nodes and links lists: entries of deleted records and repeats go; records no
    // group lists go.
    let mut listed: BTreeMap<&str, BTreeSet<ObjectId>> = BTreeMap::new();
    for group in &groups {
        for (field, ty) in [("nodes", "Node"), ("links", "Link")] {
            let items = tx.list_items(group, field)?;
            for index in (0..items.len()).rev() {
                let Some(id) = items.get(index) else { continue };
                let first = items.iter().position(|other| other == id) == Some(index);
                if !tx.exists(id) || !first || tx.object_type(id).ok().as_deref() != Some(ty) {
                    tx.list_remove(group, field, index)?;
                    wrote = true;
                } else {
                    listed.entry(ty).or_default().insert(id.clone());
                }
            }
        }
    }
    for ty in ["Node", "Link"] {
        for id in tx.objects_of_type(ty) {
            if !listed.get(ty).is_some_and(|set| set.contains(&id)) {
                tx.delete(&id)?;
                wrote = true;
            }
        }
    }
    // Links that do not fit (a node or socket gone, a second link into a single input, a cycle
    // a merged draft closed) go: the earlier link in list order stays.
    let objects: BTreeSet<u64> = tx.list_items(&root, "objects")?.iter().filter_map(|id| tx.get(id, "key").ok()?.as_u64()).collect();
    let materials: BTreeSet<u64> = tx.list_items(&root, "materials")?.iter().filter_map(|id| tx.get(id, "key").ok()?.as_u64()).collect();
    let mut geometry_keys = BTreeSet::new();
    let mut compositors = BTreeSet::new();
    let mut shaders = BTreeSet::new();
    // Every group as its records hold it (Group nodes' sockets follow the groups they use).
    let mut loaded: Vec<(ObjectId, NodeGroup, BTreeMap<Id, ObjectId>)> = Vec::new();
    for group_id in &groups {
        let Some(group_key) = record_key(tx, group_id) else { continue };
        let inputs: Vec<GroupSocket> = serde_json::from_value(tx.get(group_id, "inputs")?).unwrap_or_default();
        let outputs: Vec<GroupSocket> = serde_json::from_value(tx.get(group_id, "outputs")?).unwrap_or_default();
        let Some(tree) = tree_of(&tx.get(group_id, "tree")?) else { continue };
        match tree {
            Tree::Geometry => geometry_keys.insert(u64::from(group_key)),
            Tree::Compositor => compositors.insert(u64::from(group_key)),
            Tree::Shader => shaders.insert(u64::from(group_key)),
        };
        let mut group = NodeGroup { id: group_key, name: String::new(), tree, inputs, outputs, nodes: Vec::new(), links: Vec::new() };
        let mut records = BTreeMap::new();
        for node_id in tx.list_items(group_id, "nodes")? {
            let (Some(node_key), Some(kind)) = (record_key(tx, &node_id), tx.get(&node_id, "kind")?.as_str().map(str::to_owned)) else { continue };
            // Values naming an object or a material no record holds are let go.
            for (name, set) in [("Object", &objects), ("Material", &materials)] {
                if let Some(target) = tx.map_get(&node_id, "values", name)?.as_u64()
                    && !set.contains(&target)
                {
                    tx.map_delete(&node_id, "values", name)?;
                    wrote = true;
                }
            }
            if group.node(node_key).is_none() {
                // Settings decide which sockets show, so the check reads them.
                let values = tx.map_entries(&node_id, "values")?.into_iter().collect();
                group.nodes.push(Node { id: node_key, kind, location: [0.0, 0.0], values });
                records.insert(node_key, node_id);
            }
        }
        group.links = tx.list_items(group_id, "links")?.iter().filter_map(|id| link_key(tx, id)).collect();
        loaded.push((group_id.clone(), group, records));
    }
    // Group nodes let go of a group that is gone or not a geometry group, then of one that would
    // make a group use itself (in key order of groups and nodes: the later use goes).
    let mut uses: Vec<NodeGroup> = Vec::new();
    for at in 0..loaded.len() {
        let Some((_, group, records)) = loaded.get(at) else { continue };
        let (group_key, records) = (group.id, records.clone());
        let mut kept = NodeGroup { nodes: Vec::new(), links: Vec::new(), ..group.clone() };
        let nodes = group.nodes.clone();
        for node in nodes {
            let mut node = node;
            if let Some(used) = node_tree::used_group(&node) {
                kept.nodes.push(node.clone());
                let mut trial = uses.clone();
                trial.push(kept.clone());
                let gone = !geometry_keys.contains(&u64::from(used));
                if gone || node_tree::uses_group(&trial, group_key, group_key) || used == group_key {
                    if let Some(record) = records.get(&node.id) {
                        tx.map_delete(record, "values", "group")?;
                        wrote = true;
                    }
                    node.values.remove("group");
                    kept.nodes.pop();
                    kept.nodes.push(node.clone());
                }
            } else {
                kept.nodes.push(node.clone());
            }
            if let Some((_, group, _)) = loaded.get_mut(at)
                && let Some(slot) = group.node_mut(node.id)
            {
                *slot = node;
            }
        }
        uses.push(kept);
    }
    // Zone inputs let go of an output that is gone, of the wrong kind, or another input (in node
    // order) pairs with already.
    for (_, group, records) in &mut loaded {
        let mut paired = BTreeSet::new();
        let snapshot = group.clone();
        for node in &mut group.nodes {
            let Some(output) = node_tree::paired_output(node) else { continue };
            let fits = snapshot.zone_output_of(node).is_some() && paired.insert(output);
            if !fits {
                if let Some(record) = records.get(&node.id) {
                    tx.map_delete(record, "values", "output")?;
                    wrote = true;
                }
                node.values.remove("output");
            }
        }
    }
    // Links that do not fit (a node or socket gone, a second link into a single input, a cycle
    // a merged draft closed) go: the earlier link in list order stays.
    let all: Vec<NodeGroup> = loaded.iter().map(|(_, group, _)| group.clone()).collect();
    for (group_id, group, _) in &loaded {
        let tree = group.tree;
        let read: Vec<(ObjectId, Option<Link>)> = tx
            .list_items(group_id, "links")?
            .into_iter()
            .map(|id| {
                let link = link_key(tx, &id);
                (id, link)
            })
            .collect();
        let fit = fitting_links(group, &all);
        let mut kept: BTreeSet<Link> = BTreeSet::new();
        for (link_id, link) in read {
            let stays = link.is_some_and(|link| {
                // A link at a node of a kind this build does not know is left to builds that do.
                let unknown = [link.from, link.to].iter().any(|end| group.node(*end).is_some_and(|node| !node_tree::in_tree(&node.kind, tree)));
                unknown || (fit.contains(&link) && kept.insert(link))
            });
            if !stays {
                remove_at(tx, group_id, "links", &link_id)?;
                tx.delete(&link_id)?;
                wrote = true;
            }
        }
    }
    // Modifiers naming a group that is gone (or a compositing group) let go of it.
    for id in tx.list_items(&root, "objects")? {
        for (record, mut modifier) in held::entries(tx, &id, "modifiers", &held::MODIFIER)? {
            if modifier.get("type").and_then(Value::as_str) == Some("geometryNodes")
                && let Some(group) = modifier.get("group").and_then(Value::as_u64)
                && !geometry_keys.contains(&group)
                && let Some(map) = modifier.as_object_mut()
            {
                map.remove("group");
                held::update(tx, &record, &held::MODIFIER, &modifier, held::Mint::Derived)?;
                wrote = true;
            }
        }
    }
    // Renders let go of a compositing group that is gone.
    let mut render = tx.get(&root, "render")?;
    if let Some(group) = render.get("compositor").and_then(Value::as_u64)
        && !compositors.contains(&group)
        && let Some(map) = render.as_object_mut()
    {
        map.remove("compositor");
        tx.set(&root, "render", render)?;
        wrote = true;
    }
    // The world lets go of a volume group that is gone.
    let mut world = tx.get(&root, "world")?;
    if let Some(group) = world.get("volume").and_then(Value::as_u64)
        && !shaders.contains(&group)
        && let Some(map) = world.as_object_mut()
    {
        map.remove("volume");
        tx.set(&root, "world", world)?;
        wrote = true;
    }
    // Materials let go of a shader group that is gone.
    for id in tx.list_items(&root, "materials")? {
        let mut data = tx.get(&id, "data")?;
        if let Some(group) = data.get("shader").and_then(Value::as_u64)
            && !shaders.contains(&group)
            && let Some(map) = data.as_object_mut()
        {
            map.remove("shader");
            tx.set(&id, "data", data)?;
            wrote = true;
        }
    }
    Ok(wrote)
}
