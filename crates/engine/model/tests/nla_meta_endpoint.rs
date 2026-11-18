//! Regression for the exact meta end used by the editor's NLA resize flow.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::animation::{Action, ActionChannel, Property};
use kition_scene_model::{Interpolation, Keyframe, ObjectData, Scene};

#[test]
fn stretching_a_meta_keeps_its_last_child_on_the_requested_end() {
    let mut scene = Scene::empty();
    let object = scene.add_object("Animated", ObjectData::Empty { size: 1.0 });
    scene.animation.actions.push(Action {
        name: "Move".into(),
        channels: vec![ActionChannel {
            property: Property::Location,
            index: 0,
            path: String::new(),
            keys: vec![
                Keyframe { frame: 1.0, value: 0.0, interpolation: Interpolation::Linear },
                Keyframe { frame: 2.0, value: 1.0, interpolation: Interpolation::Linear },
            ],
            modifiers: Vec::new(),
        }],
    });
    scene.nla_add_strip(object, None, "Move", 1.0).unwrap();
    scene.object_mut(object).unwrap().nla.tracks[0].strips[0].scale = 2.0;
    scene.nla_add_strip(object, Some(0), "Move", 13.0).unwrap();
    scene.nla_add_transition(object, 0, 0).unwrap();
    scene.nla_make_meta(object, 0, 0, 2).unwrap();
    let meta = &mut scene.object_mut(object).unwrap().nla.tracks[0].strips[0];
    assert_eq!(meta.end(), 14.0);
    meta.resize_meta(1.0, 24.0).unwrap();
    assert_eq!(meta.start, 1.0);
    assert_eq!(meta.end(), 24.0);
    assert_eq!(meta.children[0].start, 1.0);
    assert_eq!(meta.children[2].end(), 24.0);
    scene.object_mut(object).unwrap().nla.settle();
    scene.validate_animation().unwrap();
    let meta = &scene.object(object).unwrap().nla.tracks[0].strips[0];
    assert_eq!(meta.children[1].start, meta.children[0].end());
    assert_eq!(meta.children[1].end(), meta.children[2].start);
    assert_eq!(meta.children[2].end(), 24.0);
}
