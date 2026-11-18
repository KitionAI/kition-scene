//! Transition and meta strips. Behaviors are measured with the reference application in
//! `tests/blender/nla6.py`: transitions interpolate the two endpoint evaluations over the
//! same lower stack; a meta is a container whose children keep their individual blending.
use crate::animation::Action;
use crate::error::{Result, SceneError};
use crate::nla::{MAX_STRIPS, Nla, NlaStrip, StripKind};
use crate::scene::{Id, Scene, unique_name};

/// Nested groups are bounded before evaluation, including files and object-model writes.
pub const MAX_META_DEPTH: usize = 16;

impl NlaStrip {
    pub fn visit_actions(&self, visit: &mut impl FnMut(&str)) {
        let mut pending = vec![self];
        let mut total = 0;
        while let Some(strip) = pending.pop() {
            total += 1;
            if total > MAX_STRIPS {
                return;
            }
            match strip.kind {
                StripKind::Action => visit(&strip.action),
                StripKind::Meta => pending.extend(&strip.children),
                StripKind::Transition => {}
            }
        }
    }

    /// Move/stretch a container and its children together. Child action ranges stay fixed.
    pub fn resize_meta(&mut self, start: f32, end: f32) -> Result<()> {
        if self.kind != StripKind::Meta || !start.is_finite() || !end.is_finite() || end <= start || self.length() <= 0.0 {
            return Err(SceneError::params("a meta strip needs a finite positive frame range"));
        }
        let scale = (end - start) / self.length();
        let from = self.start;
        let until = self.end();
        let mapped = |frame| {
            if frame == from {
                start
            } else if frame == until {
                end
            } else {
                start + (frame - from) * scale
            }
        };
        for child in &mut self.children {
            let a = mapped(child.start);
            let b = mapped(child.end());
            if child.kind == StripKind::Meta {
                child.resize_meta(a, b)?;
            } else {
                child.start = a;
                // Reconstruct the duration from both mapped boundaries. Multiplying the old
                // scale can round the last child's end away from its parent's requested end.
                child.scale = (b - a) / child.repeat / (child.action_end - child.action_start);
                child.blend_in *= scale;
                child.blend_out *= scale;
            }
        }
        self.start = start;
        self.action_start = start;
        self.action_end = end;
        self.scale = 1.0;
        self.repeat = 1.0;
        Ok(())
    }
}

/// Removes broken transitions, and makes retained transitions span their neighbors' gap.
pub(crate) fn settle_strips(strips: &mut Vec<NlaStrip>) {
    for strip in strips.iter_mut() {
        if strip.kind == StripKind::Meta {
            settle_strips(&mut strip.children);
        }
    }
    strips.retain(|strip| strip.kind != StripKind::Meta || !strip.children.is_empty());
    let mut slot = 0;
    while slot < strips.len() {
        if strips.get(slot).is_some_and(|strip| strip.kind == StripKind::Transition) {
            let bounds = slot.checked_sub(1).and_then(|left| {
                let (a, b) = (strips.get(left)?, strips.get(slot + 1)?);
                (a.kind != StripKind::Transition && b.kind != StripKind::Transition && a.end() < b.start).then_some((a.end(), b.start))
            });
            match (bounds, strips.get_mut(slot)) {
                (Some((start, end)), Some(strip)) => {
                    strip.start = start;
                    strip.action_start = start;
                    strip.action_end = end;
                    strip.scale = 1.0;
                    strip.repeat = 1.0;
                }
                _ => {
                    strips.remove(slot);
                    continue;
                }
            }
        }
        slot += 1;
    }
}

/// Validate transitions at every nesting level after the iterative size/depth gate.
pub(crate) fn check_transitions(strips: &[NlaStrip]) -> std::result::Result<(), String> {
    let mut pending = vec![strips];
    while let Some(strips) = pending.pop() {
        for (at, strip) in strips.iter().enumerate() {
            if strip.kind == StripKind::Meta {
                pending.push(&strip.children);
            }
            if strip.kind != StripKind::Transition {
                continue;
            }
            let neighbors = at.checked_sub(1).and_then(|left| strips.get(left).zip(strips.get(at + 1)));
            let Some((a, b)) = neighbors else {
                return Err("a transition needs two neighbors".into());
            };
            if a.kind == StripKind::Transition
                || b.kind == StripKind::Transition
                || a.end() >= b.start
                || (strip.start - a.end()).abs() > 1e-4
                || (strip.end() - b.start).abs() > 1e-4
                || !strip.action.is_empty()
                || strip.scale != 1.0
                || strip.repeat != 1.0
            {
                return Err("a transition must fill the gap between its two neighbors".into());
            }
        }
    }
    Ok(())
}

fn follow(strips: &mut Vec<NlaStrip>, name: &str, to: Option<&str>) {
    for strip in strips.iter_mut() {
        if strip.kind == StripKind::Meta {
            follow(&mut strip.children, name, to);
        }
        if strip.kind == StripKind::Action
            && strip.action == name
            && let Some(to) = to
        {
            strip.action = to.to_string();
        }
    }
    if to.is_none() {
        strips.retain(|strip| strip.kind != StripKind::Action || strip.action != name);
    }
    settle_strips(strips);
}

impl Nla {
    /// Gate raw register JSON before recursive serde deserialization.
    pub fn check_json_structure(value: &serde_json::Value) -> std::result::Result<(), String> {
        let Some(tracks) = value.get("tracks").and_then(serde_json::Value::as_array) else { return Ok(()) };
        if tracks.len() > crate::nla::MAX_TRACKS {
            return Err("too many NLA tracks".into());
        }
        for track in tracks {
            let Some(strips) = track.get("strips").and_then(serde_json::Value::as_array) else { continue };
            if strips.len() > MAX_STRIPS {
                return Err("too many nested NLA strips".into());
            }
            let mut pending: Vec<(&serde_json::Value, usize)> = strips.iter().map(|strip| (strip, 0)).collect();
            let mut total = 0;
            while let Some((strip, depth)) = pending.pop() {
                total += 1;
                if depth > MAX_META_DEPTH || total + pending.len() > MAX_STRIPS {
                    return Err("too many nested NLA strips".into());
                }
                if let Some(children) = strip.get("children").and_then(serde_json::Value::as_array) {
                    if children.len() > MAX_STRIPS - total - pending.len() {
                        return Err("too many nested NLA strips".into());
                    }
                    pending.extend(children.iter().map(|child| (child, depth + 1)));
                }
            }
        }
        Ok(())
    }
    /// Iterative boundary check, before any recursive walk of untrusted nested strips.
    pub fn check_structure(&self) -> std::result::Result<(), String> {
        if self.tracks.len() > crate::nla::MAX_TRACKS {
            return Err("too many NLA tracks".into());
        }
        for track in &self.tracks {
            let mut pending: Vec<(&NlaStrip, usize)> = track.strips.iter().map(|strip| (strip, 0)).collect();
            let mut total = 0;
            while let Some((strip, depth)) = pending.pop() {
                total += 1;
                if depth > MAX_META_DEPTH || total + pending.len() > MAX_STRIPS {
                    return Err("too many nested NLA strips".into());
                }
                if strip.children.len() > MAX_STRIPS - total - pending.len() {
                    return Err("too many nested NLA strips".into());
                }
                pending.extend(strip.children.iter().map(|child| (child, depth + 1)));
            }
        }
        Ok(())
    }

    pub fn follow_action(&mut self, name: &str, to: Option<&str>) {
        if self.check_structure().is_err() {
            return;
        }
        // The tweaked strip, by its name and start in its track: strips before it may go.
        let tweaked = self.tweaked().map(|strip| (strip.name.clone(), strip.start.to_bits()));
        for track in &mut self.tracks {
            follow(&mut track.strips, name, to);
        }
        self.settle();
        if let (Some(tweak), Some((strip_name, start))) = (self.tweak.as_mut(), tweaked)
            && let Some(at) = self
                .tracks
                .get(tweak.track)
                .and_then(|track| track.strips.iter().position(|strip| strip.name == strip_name && strip.start.to_bits() == start))
        {
            tweak.strip = at;
        }
    }

    /// Repair references inside containers as well as top-level strips; returns whether changed.
    pub fn retain_actions(&mut self, names: &std::collections::BTreeSet<&str>) -> bool {
        if self.check_structure().is_err() {
            return false;
        }
        let mut missing = std::collections::BTreeSet::new();
        for track in &self.tracks {
            for strip in &track.strips {
                strip.visit_actions(&mut |name| {
                    if !names.contains(name) {
                        missing.insert(name.to_string());
                    }
                });
            }
        }
        for name in &missing {
            self.follow_action(name, None);
        }
        !missing.is_empty()
    }
}

impl Scene {
    /// Add a transition in the positive gap after `strip` and before its next neighbor.
    pub fn nla_add_transition(&mut self, object: Id, track: usize, strip: usize) -> Result<usize> {
        let nla = &mut self.object_mut(object)?.nla;
        if nla.tweak.is_some() {
            return Err(SceneError::NotAllowed("leave tweak mode first".into()));
        }
        let row = nla.tracks.get_mut(track).ok_or_else(|| SceneError::NotFound(format!("no track {track}")))?;
        let (a, b) = (row.strips.get(strip), row.strips.get(strip.saturating_add(1)));
        let (Some(a), Some(b)) = (a, b) else {
            return Err(SceneError::params("a transition needs two adjacent strips"));
        };
        if a.kind == StripKind::Transition || b.kind == StripKind::Transition || a.end() >= b.start || row.strips.len() >= MAX_STRIPS {
            return Err(SceneError::params("a transition needs a positive gap between two action or meta strips"));
        }
        let name = unique_name("Transition", |taken| row.strips.iter().any(|strip| strip.name == taken));
        let mut transition = NlaStrip::new(name, &Action { name: String::new(), channels: Vec::new() }, a.end());
        transition.kind = StripKind::Transition;
        transition.action_start = a.end();
        transition.action_end = b.start;
        let at = strip + 1;
        row.strips.insert(at, transition);
        nla.settle();
        Ok(at)
    }

    /// Group a contiguous range, preserving child timing and blending; groups may nest.
    pub fn nla_make_meta(&mut self, object: Id, track: usize, first: usize, last: usize) -> Result<usize> {
        let nla = &mut self.object_mut(object)?.nla;
        if nla.tweak.is_some() {
            return Err(SceneError::NotAllowed("leave tweak mode first".into()));
        }
        let row = nla.tracks.get_mut(track).ok_or_else(|| SceneError::NotFound(format!("no track {track}")))?;
        if first > last || last >= row.strips.len() {
            return Err(SceneError::params("a meta needs a contiguous strip range"));
        }
        let (Some(a), Some(b)) = (row.strips.get(first), row.strips.get(last)) else {
            return Err(SceneError::params("no such strips"));
        };
        if a.kind == StripKind::Transition || b.kind == StripKind::Transition {
            return Err(SceneError::params("include both endpoints of every grouped transition"));
        }
        let name = unique_name("Meta", |taken| row.strips.iter().any(|strip| strip.name == taken));
        let mut meta = NlaStrip::new(name, &Action { name: String::new(), channels: Vec::new() }, a.start);
        meta.kind = StripKind::Meta;
        meta.action_start = a.start;
        meta.action_end = b.end();
        meta.children = row.strips.drain(first..=last).collect();
        row.strips.insert(first, meta);
        nla.settle();
        Ok(first)
    }

    /// Ungroup one meta. Its children's current timing is already in scene frames.
    pub fn nla_dissolve_meta(&mut self, object: Id, track: usize, strip: usize) -> Result<()> {
        let nla = &mut self.object_mut(object)?.nla;
        if nla.tweak.is_some() {
            return Err(SceneError::NotAllowed("leave tweak mode first".into()));
        }
        let row = nla.tracks.get_mut(track).ok_or_else(|| SceneError::NotFound(format!("no track {track}")))?;
        let meta = row.strips.get(strip).ok_or_else(|| SceneError::NotFound(format!("no strip {strip}")))?;
        if meta.kind != StripKind::Meta {
            return Err(SceneError::params("the strip is not a meta"));
        }
        if row.strips.len() - 1 + meta.children.len() > MAX_STRIPS {
            return Err(SceneError::params("too many strips after ungrouping"));
        }
        let meta = row.strips.remove(strip);
        row.strips.splice(strip..strip, meta.children);
        nla.settle();
        Ok(())
    }
}
