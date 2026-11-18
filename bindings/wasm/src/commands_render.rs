//! Path-traced renders: start a render, advance it a time budget at a time (so the editor's
//! worker never blocks for long), read the picture so far, or cancel. The render is session
//! state: it never changes the scene, the undo history, or the file. With `wait`, `start`
//! renders to the end and answers the picture at once (for agents and the headless kit).
//!
//! The editor can split a render across workers, each with the engine and the scene: `start`
//! with `collect` makes a render that only merges their sums (the `traceMerge` extra), and each
//! worker traces its rows after the `traceShard` extra and answers them with `traceSums`.

use kition_kit::KitCommand;
use kition_scene_trace::{Progress, TraceOptions, Tracer};
use serde_json::{Value, json};

use crate::SceneDocument;
use crate::params::{self, bad, schema as s};

struct Spec {
    id: &'static str,
    title: &'static str,
    schema: fn() -> Value,
}

static SPECS: &[Spec] = &[
    Spec {
        id: "render.trace.start",
        title: "Path Trace",
        schema: || {
            s::of(vec![
                ("samples", s::integer("Samples per pixel, 1 to 4096 (64)")),
                ("bounces", s::integer("The longest light path, in bounces (4)")),
                (
                    "volumeBounces",
                    s::integer("How often light may scatter in a volume before the path ends (0: each point in a volume is lit directly only)"),
                ),
                ("denoise", s::boolean("Smooth the remaining noise away (false)")),
                ("frame", s::number("The frame to render; the current frame when left out")),
                ("seed", s::integer("Changes the noise pattern (0)")),
                ("wait", s::boolean("Render to the end and answer the image at once (false)")),
                ("collect", s::boolean("Trace nothing here: merge the sums of workers' shards (false)")),
            ])
        },
    },
    Spec {
        id: "render.trace.step",
        title: "Continue Path Trace",
        schema: || {
            s::of(vec![
                ("budgetMs", s::number("About how long to render, in milliseconds (50)")),
                ("preview", s::boolean("Answer the picture so far as a PNG (false)")),
            ])
        },
    },
    Spec { id: "render.trace.image", title: "Path Traced Image", schema: || s::of(vec![]) },
    Spec { id: "render.trace.cancel", title: "Cancel Path Trace", schema: || s::of(vec![]) },
];

fn enabled(doc: &SceneDocument, id: &str) -> bool {
    match id {
        "render.trace.start" => doc.scene().camera.is_some(),
        _ => doc.trace.is_some(),
    }
}

/// These commands, with whether each can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (spec.id == "render.trace.start").then(|| vec!["Render".to_string()]),
            shortcut: None,
            params: None,
            schema: Some((spec.schema)()),
            mutates: false,
            enabled: enabled(doc, spec.id),
            ..Default::default()
        })
        .collect()
}

/// The ones that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.id)).map(|spec| spec.id.to_string()).collect()
}

/// Milliseconds on a clock that only moves forward.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_arch = "wasm32")]
mod clock {
    use kition_kit::wasm_bindgen::prelude::*;

    #[wasm_bindgen(wasm_bindgen = kition_kit::wasm_bindgen)]
    extern "C" {
        #[wasm_bindgen(js_namespace = Date, js_name = now)]
        pub fn date_now() -> f64;
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now() -> f64 {
    clock::date_now()
}

/// The tracer's picture as a PNG: composited once the render is done.
fn trace_picture(scene: &kition_scene_model::Scene, tracer: &Tracer) -> Result<Vec<u8>, String> {
    if !(tracer.progress().done && crate::commands_compositor::compositing(scene)) {
        return crate::encode_png(tracer.width(), tracer.height(), &tracer.image());
    }
    let pixels = tracer.linear().into_iter().map(|pixel| [pixel.x, pixel.y, pixel.z, 1.0]).collect();
    let extra = crate::commands_compositor::Extra { light: tracer.passes() };
    let pixels = crate::commands_compositor::composite(scene, tracer.width(), tracer.height(), pixels, None, &extra)?;
    crate::encode_png(tracer.width(), tracer.height(), &crate::commands_compositor::to_rgba8(&pixels, &scene.render.color_management))
}

fn progress_json(tracer: &Tracer, progress: Progress) -> Value {
    json!({
        "samples": progress.samples,
        "target": progress.target,
        "fraction": progress.fraction,
        "done": progress.done,
        "width": tracer.width(),
        "height": tracer.height(),
    })
}

impl SceneDocument {
    /// Advances the path-traced render by about `budget_ms` milliseconds.
    pub fn trace_step(&mut self, budget_ms: f64) -> Result<Value, String> {
        let tracer = self.trace.as_mut().ok_or("not-allowed: no path-traced render is under way")?;
        let budget = if budget_ms.is_finite() { budget_ms.clamp(1.0, 5000.0) } else { 50.0 };
        let progress = tracer.step(budget, &now);
        Ok(progress_json(tracer, progress))
    }

    /// The path-traced picture so far as PNG bytes; once done, through the scene's compositing
    /// group when compositing is on.
    pub fn trace_png(&self) -> Result<Vec<u8>, String> {
        let tracer = self.trace.as_ref().ok_or("not-allowed: no path-traced render is under way")?;
        trace_picture(&self.scene, tracer)
    }
}

/// Runs `command` if it is one of these; `None` for every other command.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    SPECS.iter().find(|spec| spec.id == command)?;
    Some(run(doc, command, params))
}

/// The options of `render.trace.start` (and of a shard).
fn options(params: &Value) -> Result<TraceOptions, String> {
    let defaults = TraceOptions::default();
    let options = TraceOptions {
        samples: params::integer(params, "samples")?.unwrap_or(defaults.samples),
        bounces: params::integer(params, "bounces")?.unwrap_or(defaults.bounces),
        volume_bounces: params::integer(params, "volumeBounces")?.unwrap_or(defaults.volume_bounces).min(kition_scene_trace::MAX_BOUNCES),
        denoise: params::boolean(params, "denoise")?.unwrap_or(false),
        seed: u64::from(params::integer(params, "seed")?.unwrap_or(0)),
        ..defaults
    };
    if !(1..=kition_scene_trace::MAX_SAMPLES).contains(&options.samples) {
        return Err(bad(format!("`samples` must be between 1 and {}", kition_scene_trace::MAX_SAMPLES)));
    }
    Ok(options)
}

impl SceneDocument {
    /// Starts tracing shard `index` of `count` of a render (JSON: `index`, `count`, and
    /// `render.trace.start`'s options) in place of any render under way; answers its progress.
    pub fn trace_shard(&mut self, json: &str) -> Result<Value, String> {
        let params: Value = serde_json::from_str(json).map_err(|error| bad(format!("the shard is not JSON: {error}")))?;
        let options = options(&params)?;
        let index = params::integer(&params, "index")?.unwrap_or(0);
        let count = params::integer(&params, "count")?.unwrap_or(1);
        let scene = match params::number(&params, "frame")? {
            Some(frame) => self.scene().at_frame(frame),
            None => self.scene().clone(),
        };
        let scene = crate::commands_particles::render_scene(self, scene);
        let options = TraceOptions { passes: crate::commands_compositor::wants_light_passes(&scene), ..options };
        let tracer = Tracer::shard(&scene, options, index, count)?;
        let answer = progress_json(&tracer, tracer.progress());
        self.trace = Some(tracer);
        Ok(answer)
    }

    /// The rows of the render under way so far, for the collecting render to merge.
    pub fn trace_sums(&self) -> Result<Vec<u8>, String> {
        Ok(self.trace.as_ref().ok_or("not-allowed: no path-traced render is under way")?.sums())
    }

    /// Merges a shard's sums into the render under way; answers its progress.
    pub fn trace_merge(&mut self, sums: &[u8]) -> Result<Value, String> {
        let tracer = self.trace.as_mut().ok_or("not-allowed: no path-traced render is under way")?;
        let progress = tracer.merge(sums)?;
        Ok(progress_json(tracer, progress))
    }
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    if !params.is_object() && !params.is_null() {
        return Err(bad("the parameters must be an object"));
    }
    match command {
        "render.trace.start" => {
            let options = options(params)?;
            let scene = match params::number(params, "frame")? {
                Some(frame) => doc.scene().at_frame(frame),
                None => doc.scene().clone(),
            };
            let scene = crate::commands_particles::render_scene(doc, scene);
            let options = TraceOptions { passes: crate::commands_compositor::wants_light_passes(&scene), ..options };
            if params::boolean(params, "collect")?.unwrap_or(false) {
                let tracer = Tracer::collector(&scene, options)?;
                let answer = progress_json(&tracer, tracer.progress());
                doc.trace = Some(tracer);
                return Ok(answer);
            }
            let mut tracer = Tracer::new(&scene, options)?;
            if params::boolean(params, "wait")?.unwrap_or(false) {
                let progress = tracer.finish();
                let png = trace_picture(doc.scene(), &tracer)?;
                let mut answer = progress_json(&tracer, progress);
                if let Some(fields) = answer.as_object_mut() {
                    fields.insert("mime".into(), json!("image/png"));
                    fields.insert("extension".into(), json!("png"));
                    fields.insert("base64".into(), json!(crate::commands::base64(&png)));
                }
                doc.trace = Some(tracer);
                return Ok(answer);
            }
            let answer = progress_json(&tracer, tracer.progress());
            doc.trace = Some(tracer);
            Ok(answer)
        }
        "render.trace.step" => {
            let budget = f64::from(params::number(params, "budgetMs")?.unwrap_or(50.0));
            let mut answer = doc.trace_step(budget)?;
            if params::boolean(params, "preview")?.unwrap_or(false) {
                let png = doc.trace_png()?;
                if let Some(fields) = answer.as_object_mut() {
                    fields.insert("base64".into(), json!(crate::commands::base64(&png)));
                }
            }
            Ok(answer)
        }
        "render.trace.image" => {
            let png = doc.trace_png()?;
            let tracer = doc.trace.as_ref().ok_or("not-allowed: no path-traced render is under way")?;
            let mut answer = progress_json(tracer, tracer.progress());
            if let Some(fields) = answer.as_object_mut() {
                fields.insert("mime".into(), json!("image/png"));
                fields.insert("extension".into(), json!("png"));
                fields.insert("base64".into(), json!(crate::commands::base64(&png)));
            }
            Ok(answer)
        }
        "render.trace.cancel" => {
            let had = doc.trace.take().is_some();
            Ok(json!({ "cancelled": had }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}
