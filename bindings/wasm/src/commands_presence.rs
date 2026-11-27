//! Presence in a co-edited scene (Kit UI protocol section 7e): what this person shares (the
//! objects they selected, as KDM ids; where their pointer is in the scene; the mesh and vertices
//! they edit) and where the other people's shares land in this view (their objects outlined in
//! their colour by the renderer; their pointers, labels, and edited vertices placed in viewport
//! pixels for the editor to draw). None of it is saved or undone; the agent is a peer like anyone.

use std::collections::{BTreeMap, BTreeSet};

use kition_kit::KitCommand;
use kition_scene_model::Id;
use kition_scene_model::glam::Vec3;
use kition_scene_raster::peers::PeerOutline;
use serde_json::{Value, json};

use crate::params::{bad, schema as s};
use crate::{Mode, SceneDocument};

/// At most this many of a peer's edited vertices are placed (and shared).
const VERTEX_LIMIT: usize = 256;

/// Another person in the scene, as this view knows them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Peer {
    pub principal: String,
    pub name: String,
    pub color: String,
    pub objects: BTreeSet<Id>,
    /// Where their pointer is, in the scene.
    pub pointer: Option<Vec3>,
    /// The mesh they edit, and the vertices they selected in it.
    pub editing: Option<Id>,
    pub vertices: Vec<u32>,
    /// The agent (origin `agent`), not a person.
    pub agent: bool,
}

/// Presence state of the session.
#[derive(Debug, Clone, Default)]
pub struct PresenceSession {
    pub peers: Vec<Peer>,
    /// This person's pointer in the scene, as last shared.
    pub pointer: Option<Vec3>,
}

fn command(id: &str, title: &str, schema: Value) -> KitCommand {
    KitCommand {
        id: id.into(),
        title: title.into(),
        menu: None,
        shortcut: None,
        params: None,
        schema: Some(schema),
        mutates: false,
        enabled: true,
        ..Default::default()
    }
}

/// The commands this file adds (for editors; agents need none of them).
pub fn catalogue(_doc: &SceneDocument) -> Vec<KitCommand> {
    vec![
        command(
            "presence.local",
            "Share Presence",
            s::of(vec![("pointer", s::indices("The pointer in viewport pixels [x, y]; left out when it left the view"))]),
        ),
        command(
            "presence.peers",
            "Other People",
            s::of(vec![("peers", s::required(serde_json::json!({ "type": "json", "description": "presence/peers as the host sent it" })))]),
        ),
        command("presence.place", "Place Other People", s::of(vec![])),
    ]
}

/// The ones that cannot run now (none).
pub fn disabled(_doc: &SceneDocument) -> Vec<String> {
    Vec::new()
}

/// Runs one of this file's commands; `None` for any other.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    match command {
        "presence.local" => Some(local(doc, params)),
        "presence.peers" => Some(peers(doc, params)),
        "presence.place" => Some(Ok(place(doc))),
        _ => None,
    }
}

/// The objects' KDM ids by key, and keys by KDM id.
fn id_maps(doc: &SceneDocument) -> (BTreeMap<Id, String>, BTreeMap<String, Id>) {
    let ids = kition_scene_kdm::read::object_ids(doc.replica.replica());
    let by_key: BTreeMap<Id, String> = ids.into_iter().map(|(key, id)| (key, id.as_str().to_owned())).collect();
    let by_id = by_key.iter().map(|(key, id)| (id.clone(), *key)).collect();
    (by_key, by_id)
}

/// The scene point under viewport pixel (`x`, `y`): the drawn surface there, else the ground.
fn scene_point(doc: &SceneDocument, x: f32, y: f32) -> Option<Vec3> {
    let projector = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height);
    let (origin, direction) = projector.ray(x, y);
    // The surface under the pixel, found with a ray (no software picture is drawn for it).
    if let Some(hit) = doc.pick_rays().at(&projector, x, y) {
        return Some(hit.point);
    }
    (direction.z.abs() > 1e-6).then(|| -origin.z / direction.z).filter(|t| *t > 0.0).map(|t| origin + direction * t)
}

/// What this person shares now.
fn shared(doc: &SceneDocument) -> Value {
    let (by_key, _) = id_maps(doc);
    let selection: Vec<&String> = doc.session.editor.selected.iter().filter_map(|key| by_key.get(key)).collect();
    let mut viewport = serde_json::Map::new();
    if let Some(point) = doc.session.presence.pointer {
        viewport.insert("pointer".into(), json!([point.x, point.y, point.z]));
    }
    if let Mode::Edit(object) = doc.session.mode
        && let Some(id) = by_key.get(&object)
    {
        viewport.insert("editing".into(), json!(id));
        viewport.insert("vertices".into(), json!(doc.session.vertices.iter().take(VERTEX_LIMIT).collect::<Vec<_>>()));
    }
    json!({ "selection": selection, "viewport": viewport })
}

/// `presence.local`: takes the pointer and answers what this person shares.
fn local(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    doc.session.presence.pointer = match params.get("pointer").and_then(Value::as_array) {
        Some(pair) => match pair.as_slice() {
            [x, y] => {
                let (x, y) = (x.as_f64().ok_or_else(|| bad("`pointer` is [x, y]"))?, y.as_f64().ok_or_else(|| bad("`pointer` is [x, y]"))?);
                if !(x.is_finite() && y.is_finite()) {
                    return Err(bad("`pointer` must be finite"));
                }
                scene_point(doc, x as f32, y as f32)
            }
            _ => return Err(bad("`pointer` is [x, y]")),
        },
        None => None,
    };
    Ok(shared(doc))
}

fn parse_peer(value: &Value, by_id: &BTreeMap<String, Id>) -> Option<Peer> {
    let principal = value.get("principal")?.as_str()?.to_owned();
    let name = value.get("name")?.as_str()?.to_owned();
    let color = value.get("color").and_then(Value::as_str).unwrap_or_default().to_owned();
    let objects = value
        .get("selection")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(|id| id.as_str().and_then(|id| by_id.get(id)).copied()).collect())
        .unwrap_or_default();
    let viewport = value.get("viewport");
    let pointer = viewport.and_then(|viewport| viewport.get("pointer")).and_then(Value::as_array).and_then(|values| match values.as_slice() {
        [x, y, z] => Some(Vec3::new(x.as_f64()? as f32, y.as_f64()? as f32, z.as_f64()? as f32)).filter(|point| point.is_finite()),
        _ => None,
    });
    let editing = viewport.and_then(|viewport| viewport.get("editing")).and_then(Value::as_str).and_then(|id| by_id.get(id)).copied();
    let vertices = viewport
        .and_then(|viewport| viewport.get("vertices"))
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(|value| value.as_u64().and_then(|vertex| u32::try_from(vertex).ok())).take(VERTEX_LIMIT).collect())
        .unwrap_or_default();
    // Hosts pass a peer's principal, name, colour, and presence (Kit UI 7e); the agent's principal
    // is `agent` (or `agent:<session>`), and its viewport may say so too.
    let agent = principal == "agent"
        || principal.starts_with("agent:")
        || value.get("agent").is_some_and(|agent| !agent.is_null())
        || viewport.and_then(|viewport| viewport.get("origin")).and_then(Value::as_str) == Some("agent");
    Some(Peer { principal, name, color, objects, pointer, editing, vertices, agent })
}

/// `presence.peers`: takes the other people and answers where they land in this view.
fn peers(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let list = params.get("peers").and_then(Value::as_array).ok_or_else(|| bad("`peers` is a list"))?;
    let (_, by_id) = id_maps(doc);
    let next: Vec<Peer> = list.iter().filter_map(|peer| parse_peer(peer, &by_id)).collect();
    let outlines = |peers: &[Peer]| peers.iter().map(|peer| (peer.color.clone(), peer.objects.clone())).collect::<Vec<_>>();
    if outlines(&next) != outlines(&doc.session.presence.peers) {
        doc.touched();
    }
    doc.session.presence.peers = next;
    Ok(place(doc))
}

/// A `#rgb` or `#rrggbb` colour as linear RGB, or a neutral grey.
pub fn linear(color: &str) -> [f32; 3] {
    let hex = color.strip_prefix('#').unwrap_or_default();
    let full = match hex.len() {
        3 => hex.chars().flat_map(|digit| [digit, digit]).collect::<String>(),
        6 | 8 => hex.get(..6).unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    u32::from_str_radix(&full, 16).map_or(kition_scene_raster::draw::hex(0x6b_72_80), kition_scene_raster::draw::hex)
}

/// The peers' outlines for the renderer.
pub fn outlines(doc: &SceneDocument) -> Vec<PeerOutline> {
    doc.session.presence.peers.iter().map(|peer| PeerOutline { color: linear(&peer.color), objects: peer.objects.clone() }).collect()
}

/// `presence.place`: where each peer's pointer, label, and edited vertices land in this view.
fn place(doc: &SceneDocument) -> Value {
    let projector = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height);
    let (width, height) = (doc.session.width as f32, doc.session.height as f32);
    let screen = |point: Vec3| -> Option<[f32; 2]> {
        let clip = projector.view_projection() * point.extend(1.0);
        if clip.w <= 0.0 || clip.z < 0.0 {
            return None;
        }
        let at = projector.to_screen(clip);
        (at.x >= 0.0 && at.y >= 0.0 && at.x <= width && at.y <= height).then_some([at.x, at.y])
    };
    let peers: Vec<Value> = doc
        .session
        .presence
        .peers
        .iter()
        .map(|peer| {
            // The label sits at the top left of the first selected object's box on screen.
            let label = peer.objects.iter().find_map(|id| {
                let bounds = doc.scene.bounds(Some(&[*id]));
                if bounds.is_empty() {
                    return None;
                }
                let corners = (0..8).map(|bits| {
                    Vec3::new(
                        if bits & 1 == 0 { bounds.min.x } else { bounds.max.x },
                        if bits & 2 == 0 { bounds.min.y } else { bounds.max.y },
                        if bits & 4 == 0 { bounds.min.z } else { bounds.max.z },
                    )
                });
                let placed: Vec<[f32; 2]> = corners
                    .filter_map(|corner| {
                        let clip = projector.view_projection() * corner.extend(1.0);
                        (clip.w > 0.0).then(|| {
                            let at = projector.to_screen(clip);
                            [at.x, at.y]
                        })
                    })
                    .collect();
                let x = placed.iter().map(|point| point[0]).fold(f32::INFINITY, f32::min);
                let y = placed.iter().map(|point| point[1]).fold(f32::INFINITY, f32::min);
                (x.is_finite() && y.is_finite()).then(|| [x.clamp(0.0, width), y.clamp(0.0, height)])
            });
            let vertices: Vec<[f32; 2]> = match (peer.editing, doc.scene.object(peer.editing.unwrap_or(Id::MAX))) {
                (Some(id), Ok(object)) => {
                    let world = doc.scene.world_matrix(id).unwrap_or_default();
                    object.mesh().map_or_else(Vec::new, |mesh| {
                        peer.vertices
                            .iter()
                            .filter_map(|vertex| mesh.positions.get(*vertex as usize))
                            .filter_map(|point| screen(world.transform_point3(*point)))
                            .collect()
                    })
                }
                _ => Vec::new(),
            };
            json!({
                "principal": peer.principal,
                "name": peer.name,
                "color": peer.color,
                "agent": peer.agent,
                "objects": peer.objects,
                "editing": peer.editing,
                "pointer": peer.pointer.and_then(screen),
                "label": label,
                "vertices": vertices,
            })
        })
        .collect();
    json!({ "peers": peers, "width": doc.session.width, "height": doc.session.height })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_short_long_and_bad() {
        assert_eq!(linear("#ff0000"), [1.0, 0.0, 0.0]);
        assert_eq!(linear("#f00"), [1.0, 0.0, 0.0]);
        assert_eq!(linear("#ff000080"), [1.0, 0.0, 0.0]);
        assert_eq!(linear("tomato"), kition_scene_raster::draw::hex(0x6b_72_80));
    }
}
