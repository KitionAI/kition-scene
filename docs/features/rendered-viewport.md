# Rendered viewport

The Rendered shading button and `view.rendered` use the existing CPU path tracer for the
current orbit or scene camera. Scene lights, world lighting, materials, glass, textures,
shader groups and volumes use the same tracing code as final renders. Orbit views work
without a scene camera. Camera views keep their framing and depth of field.

The engine worker compiles the visible geometry, material programs, textures and BVH once
per input revision. `view.renderedStep` advances it, normally in cooperative 8 ms steps;
a row is indivisible and may take longer than that budget. The editor keeps at most one
request in flight, yields between requests and stops scheduling when shading changes or
the document closes. Scene edits, undo, redo, peer edits, navigation, resizing and timeline
changes discard old samples. A sampling step carries its compiled scene to its new display
revision. Changing the sample target also keeps that scene; lowering it resets accumulation.
Neither sampling nor shading changes dirty the file or enter undo history. Shading is saved
as editor session state; accumulated samples are not saved and do not change the KDM schema.

The preview defaults to 32 samples and is reduced by an integer factor to at most 512 pixels
on its longer side, then expanded to the document viewport. Its projection matrix remains
identical to the full-size viewport. Picking, selection outlines, grid, gizmos and editing
overlays stay at full document resolution. Sculpt masks and weight colors keep their working
surface display. The initial preparation view shows solid shading until the first complete
sample is available. The progress label reports completed samples. The GPU canvas steps
aside in rendered mode; the software canvas, headless render and agent screenshot all read
one accumulated frame. Final renders remain independent.

Headless callers can run `view.renderedStep` with `wait: true` and `samples: 1..256`, then
call the normal viewport render operation. `wait` is not used by the interactive editor.
The command reports `generation`, which counts compiled scenes. The regression suite checks
that increasing and decreasing sample targets leave it unchanged and produce exactly the
same image as fresh deterministic accumulation. This is structural performance evidence;
no elapsed-time or frame-rate improvement is claimed from this loaded machine.

Reference observations are made only through Blender's public Python API. The script
`crates/render/trace/tests/blender/viewport7.py` records Blender 5.2.3 LTS's rendered-mode
scene-light/world defaults and Cycles pictures through cameras equivalent to perspective
and orthographic orbits. Committed fixtures run offline; `KITION_BLENDER` checks them live.
See the [Blender manual](https://docs.blender.org/manual/en/latest/editors/3dview/display/shading.html).

Limitations: CPU tracing and the preview resolution/sample caps trade detail for interaction.
Scene preparation is synchronous within the worker. Motion blur and the compositor are final
render features; the viewport shows the current frame without them. Existing tracer fidelity
gaps remain, including the darker caustic through absorbing glass. Rendered shading does not
add GPU tracing, material-preview ambient occlusion, or more GPU lights.
