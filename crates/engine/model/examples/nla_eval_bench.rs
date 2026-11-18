//! Repeatable NLA evaluation measurements: `cargo run -p kition-scene-model --example nla_eval_bench`
//! prints them as CSV.
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::time::Instant;

use kition_scene_model::animation::{Action, ActionChannel, Property};
use kition_scene_model::nla::{BlendType, Nla, NlaStrip, NlaTrack, StripKind};
use kition_scene_model::{Interpolation, Keyframe};

fn action(name: &str, bones: usize, offset: f32, sparse: bool) -> Action {
    let mut channels = Vec::new();
    for bone in 0..bones {
        if sparse && bone % 4 == 0 {
            continue;
        }
        for property in [Property::BoneLocation, Property::BoneRotation, Property::BoneScale] {
            for index in 0..3 {
                let base = 1.0 + bone as f32 / 64.0 + index as f32 / 8.0 + offset;
                channels.push(ActionChannel {
                    property,
                    index,
                    path: format!("Rig.Joint.{bone:03}"),
                    keys: [1.0, 11.0]
                        .into_iter()
                        .map(|frame| Keyframe { frame, value: base + frame / 16.0, interpolation: Interpolation::Linear })
                        .collect(),
                    modifiers: Vec::new(),
                });
            }
        }
    }
    Action { name: name.into(), channels }
}

fn group(children: Vec<NlaStrip>) -> NlaStrip {
    let mut meta = NlaStrip::new("Meta", &Action { name: String::new(), channels: Vec::new() }, 1.0);
    meta.kind = StripKind::Meta;
    meta.action_start = 1.0;
    meta.action_end = 41.0;
    meta.children = children;
    meta
}

fn fixture(bones: usize, layers: usize) -> Result<(Nla, Vec<Action>), String> {
    let lower = action("Lower", bones, 0.25, false);
    let left = action("Left", bones, 0.5, false);
    let right = action("Right", bones, 1.0, true);
    let mut nla = Nla::default();
    nla.tracks.push(NlaTrack { name: "Lower".into(), muted: false, solo: false, strips: vec![NlaStrip::new("Lower", &lower, 1.0)] });
    for layer in 0..layers {
        let kind = match layer % 3 {
            0 => BlendType::Replace,
            1 => BlendType::Add,
            _ => BlendType::Combine,
        };
        let mut a = NlaStrip::new("Left", &left, 1.0);
        a.blend = kind;
        a.influence = Some(0.25);
        let mut b = NlaStrip::new("Right", &right, 31.0);
        b.blend = kind;
        b.influence = Some(0.75);
        let mut transition = NlaStrip::new("Transition", &Action { name: String::new(), channels: Vec::new() }, 11.0);
        transition.kind = StripKind::Transition;
        transition.action_start = 11.0;
        transition.action_end = 31.0;
        let mut strips = vec![a, transition, b];
        if layer % 2 == 1 {
            strips = vec![group(vec![group(strips)])];
        }
        nla.tracks.push(NlaTrack { name: format!("Layer {layer}"), muted: false, solo: false, strips });
    }
    let actions = vec![lower, left, right];
    nla.check(&actions)?;
    Ok((nla, actions))
}

fn frame(sample: usize, transitions: bool) -> f32 {
    if transitions { 11.5 + (sample % 20) as f32 } else { 1.5 + (sample % 10) as f32 }
}

fn fingerprint(nla: &Nla, actions: &[Action], transitions: bool) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for sample in 0..20 {
        for (property, index, path, value) in nla.evaluate(&[], actions, frame(sample, transitions)) {
            (property, index, path, value.to_bits()).hash(&mut hash);
        }
    }
    hash.finish()
}

fn main() -> Result<(), String> {
    println!("workload,phase,bones,layers,channels,iterations,round,us_per_evaluation,fingerprint");
    for (name, bones, layers, iterations) in [("small", 8, 1, 2000), ("large", 128, 8, 20)] {
        let (nla, actions) = fixture(bones, layers)?;
        for transitions in [false, true] {
            let phase = if transitions { "transition" } else { "action" };
            let fingerprint = fingerprint(&nla, &actions, transitions);
            for sample in 0..20 {
                black_box(nla.evaluate(&[], &actions, black_box(frame(sample, transitions))));
            }
            for round in 0..7 {
                let start = Instant::now();
                for sample in 0..iterations {
                    black_box(nla.evaluate(&[], &actions, black_box(frame(sample, transitions))));
                }
                let elapsed = start.elapsed().as_secs_f64() * 1e6 / iterations as f64;
                println!("{name},{phase},{bones},{layers},{},{iterations},{round},{elapsed:.3},{fingerprint:016x}", bones * 9);
            }
        }
    }
    Ok(())
}
