//! Session-only rendered shading: a compiled scene survives every sampling step. The shared
//! engine worker advances bounded row batches, so navigation and edits can run between them.

use crate::{SceneDocument, params};
use kition_scene_raster::{Shading, ViewportPicture};
use kition_scene_trace::{TraceOptions, Tracer};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const DEFAULT_SAMPLES: u32 = 32;
/// Preview resolution bounds tracing work and memory independently of the window's DPR, unless
/// the caller asks for more (`maxSide`).
const MAX_SIDE: u32 = 512;
/// The range `maxSide` may ask for.
const MAX_SIDE_RANGE: std::ops::RangeInclusive<u32> = 64..=4096;

#[derive(Default)]
pub struct State {
    pub generation: u64,
    pub current: Option<Frame>,
}

pub struct Frame {
    pub revision: u64,
    pub tracer: Tracer,
    pub picture: Option<ViewportPicture>,
    /// The longest side it traces at, and whether it is denoised when done.
    pub max_side: u32,
    pub denoise: bool,
}

impl State {
    pub fn picture(&self, revision: u64) -> Option<&ViewportPicture> {
        self.current.as_ref().filter(|frame| frame.revision == revision)?.picture.as_ref()
    }

    pub fn status(&self, revision: u64) -> Value {
        match self.current.as_ref().filter(|frame| frame.revision == revision) {
            Some(frame) => {
                let progress = frame.tracer.progress();
                json!({ "samples": progress.samples, "target": progress.target, "fraction": progress.fraction,
                    "done": progress.done, "generation": self.generation, "width": frame.tracer.width(), "height": frame.tracer.height() })
            }
            None => json!({ "samples": 0, "target": DEFAULT_SAMPLES, "fraction": 0.0, "done": false, "generation": self.generation,
                "width": 0, "height": 0 }),
        }
    }
}

impl SceneDocument {
    /// A short tracing step, or a completed deterministic preview for a headless caller.
    pub fn viewport_trace_step(&mut self, params: &Value) -> Result<Value, String> {
        if self.session.editor.shading != Shading::Rendered {
            return Err("not-allowed: rendered viewport shading is not active".into());
        }
        let samples = params::integer(params, "samples")?;
        if samples.is_some_and(|samples| !(1..=256).contains(&samples)) {
            return Err(params::bad("`samples` must be between 1 and 256"));
        }
        let budget = f64::from(params::number(params, "budgetMs")?.unwrap_or(8.0)).clamp(1.0, 32.0);
        let wait = params::boolean(params, "wait")?.unwrap_or(false);
        let asked_side = params::integer(params, "maxSide")?;
        if asked_side.is_some_and(|side| !MAX_SIDE_RANGE.contains(&side)) {
            return Err(params::bad("`maxSide` must be between 64 and 4096"));
        }
        let asked_denoise = params::boolean(params, "denoise")?;
        let mut state = self.viewport_trace.borrow_mut();
        // What is left out stays as the preview under way has it.
        let current = state.current.as_ref().filter(|frame| frame.revision == self.revision);
        let max_side = asked_side.or(current.map(|frame| frame.max_side)).unwrap_or(MAX_SIDE);
        let denoise = asked_denoise.or(current.map(|frame| frame.denoise)).unwrap_or(false);
        if current.is_none_or(|frame| frame.max_side != max_side || frame.denoise != denoise) {
            let shown = crate::commands_particles::viewport_scene(self);
            let scene = shown.as_ref().unwrap_or(&self.scene);
            let mut camera = self.session.editor.view.projector(scene, self.session.width, self.session.height);
            // Keep the projection matrix exactly: rounding the reduced dimensions must never
            // change framing relative to picking, camera borders, and the ordinary viewport.
            let divisor = self.session.width.max(self.session.height).div_ceil(max_side).max(1);
            camera.width = self.session.width.div_ceil(divisor).max(1);
            camera.height = self.session.height.div_ceil(divisor).max(1);
            let overlays = self.overlays();
            let meshes: HashMap<_, _> = kition_scene_raster::viewport_meshes(scene, &overlays, &mut self.meshes.borrow_mut())
                .into_iter()
                .map(|(id, mesh, _)| {
                    if let Some(sculpt) = overlays.sculpt.as_ref().filter(|sculpt| sculpt.object == id && sculpt.hidden.iter().any(|hidden| *hidden))
                    {
                        let mut visible = mesh.mesh.clone();
                        for (index, face) in visible.faces.iter_mut().enumerate() {
                            if sculpt.hidden.get(index).copied().unwrap_or(false) {
                                face.clear();
                                if let Some(uvs) = visible.uvs.get_mut(index) {
                                    uvs.clear();
                                }
                            }
                        }
                        (id, std::sync::Arc::new(kition_scene_raster::Prepared::new(visible)))
                    } else {
                        (id, mesh)
                    }
                })
                .collect();
            let options = TraceOptions { samples: samples.unwrap_or(DEFAULT_SAMPLES), denoise, ..TraceOptions::default() };
            let tracer = Tracer::viewport(scene, camera, self.session.editor.view.camera, options, &meshes)?;
            state.generation = state.generation.saturating_add(1);
            state.current = Some(Frame { revision: self.revision, tracer, picture: None, max_side, denoise });
        }
        let Some(frame) = state.current.as_mut() else { return Err("failed: the viewport render could not start".into()) };
        if let Some(samples) = samples.filter(|samples| *samples != frame.tracer.options().samples) {
            frame.tracer.set_samples(samples);
        }
        let before = frame.tracer.progress();
        let progress = if wait { frame.tracer.finish() } else { frame.tracer.step_preview(budget, &crate::commands_render::now) };
        if progress.samples != before.samples {
            frame.picture = Some(ViewportPicture { width: frame.tracer.width(), height: frame.tracer.height(), pixels: frame.tracer.linear() });
            // Sampling changes only the displayed picture. Carry the compiled scene forward
            // to that revision so another step never re-evaluates it or rebuilds its BVH.
            self.revision = self.revision.saturating_add(1);
            frame.revision = self.revision;
        }
        Ok(state.status(self.revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_kit::KitDocument;

    #[test]
    fn a_delayed_render_reads_the_published_pass_not_later_partial_rows() {
        let mut doc = SceneDocument::create("Preview.kscene", 32, 24).unwrap();
        doc.execute("view.resize", json!({"width":32,"height":24})).unwrap();
        doc.execute("view.rendered", json!({})).unwrap();
        doc.viewport_trace_step(&json!({"samples":1,"wait":true})).unwrap();
        let baseline = doc.render(0.0, 0.0, 1.0, 32, 24).unwrap();
        let revision = doc.revision;
        // Remove the software frame to exercise delayed composition from the published
        // accumulation. A partial next pass must never leak into this same revision.
        *doc.cache.borrow_mut() = None;
        {
            let mut state = doc.viewport_trace.borrow_mut();
            let frame = state.current.as_mut().unwrap();
            frame.tracer.set_samples(2);
            let progress = frame.tracer.step_rows(3);
            assert_eq!(progress.samples, 1);
            assert!(!progress.done);
        }
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.render(0.0, 0.0, 1.0, 32, 24).unwrap(), baseline);
        doc.viewport_trace_step(&json!({"wait":true})).unwrap();
        assert!(doc.revision > revision);
        assert_ne!(doc.render(0.0, 0.0, 1.0, 32, 24).unwrap(), baseline);
    }
}
