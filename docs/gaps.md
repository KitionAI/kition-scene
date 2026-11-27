# Gaps

> **Last updated:** 2026-11-26 · **Source:** `features.yaml` (updated 2026-11-26), the engine's command list and the first ten minutes' acceptance run (`web/tests/first-ten-minutes.mjs`)

Every known shortfall of Kition Scene, from the feature catalogue: features that are not yet `has`,
then the differences recorded on features that work. Update the feature in `features.yaml` when you
land work, then this list. The comparison with the target application is in
[parity/target-app-parity.md](parity/target-app-parity.md); the order of work is in [roadmap.md](roadmap.md).

## Features not done

- `scene.animation.timeline` (P1, partial): Keyframes, frames, drivers, keying sets, actions, NLA and the graph editor. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.collab.presence` (P1, partial): Presence of other people in the scene. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.compositor.nodes` (P1, partial): Compositor. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.curve.text-lattice` (P1, partial): Curves, text objects and lattices. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.edit.history` (P1, partial): Undo, redo, delete, duplicate and selection. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.file.import-export` (P1, partial): Import and export. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.material.manage` (P1, partial): Materials. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.mesh.edit` (P1, partial): Mesh editing: select modes, extrude, inset, bevel, loop cut and the other Edit Mode tools. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.mode.switch` (P1, partial): Object, Edit, Pose, Sculpt and Weight Paint modes. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.modifier.stack` (P1, partial): Modifier stack. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.nodes.geometry` (P1, partial): Geometry node groups. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.object.add` (P1, partial): Add meshes, curves, lights, cameras and empties. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.object.transform` (P1, partial): Object transforms, parenting, origins and visibility. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.particles.hair` (P1, partial): Particle systems and grooming. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.physics.simulation` (P1, partial): Rigid bodies, cloth, soft bodies, fields and constraints. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.render.lights-world` (P1, partial): Lights, cameras, world and render settings. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.rigging.armature` (P1, partial): Bones, poses, vertex groups, shape keys and constraints. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.sculpt.paint` (P1, partial): Sculpting, multiresolution and weight painting. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.uv.editing` (P1, partial): UV projections and UV editing. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.
- `scene.view.viewport` (P1, partial): Viewport shading, views and overlays. The engine has these commands; the navigation a person does is `scene.editor.navigation` (below); no case yet shows shading and overlays working end to end in the editor and surviving save and reopen.
- `scene.editor.basics` (P1, partial): The viewport's basics by hand. The label drag's live preview covers object transforms only; the outline and edit-mode colours are not Blender's.
- `scene.editor.modal-transform` (P0, partial): Modal G, R and S. No units or arithmetic in a typed value; no trackball (R R), automatic constraint (middle button) or G G slide; the side panel shows the values only after the confirm; the pointer stops at the screen's edge.
- `scene.editor.modal-operators` (P0, partial): E, I, Ctrl/Cmd+B, Ctrl/Cmd+R as modals; X, M, P menus. Inset is per face, with no Outset, Boundary or Individual; no bevel profile; no loop cut smoothness; the Merge and Separate menus have one row each; object mode's X deletes without asking.
- `scene.editor.adjust-last` (P0, partial): Adjust Last Operation. Only the adjustable commands of `operation.rs`; a field applies on Enter, not while dragged.
- `scene.editor.navigation` (P0, partial): Navigation. No walk or fly, local view, roll, zoom to the pointer; a trackpad is told from a mouse by its events alone and no real trackpad has been tried; a document opens with the engine's view and nothing selected.
- `scene.editor.transform-tools` (P0, partial): The 3D cursor, gizmos, pivot, orientation, snapping. No Cursor or Transform tool, no Scale Cage; snapping moves only and has no grid, edge-centre or perpendicular targets.
- `scene.editor.interface` (P0, partial): The viewport header, menus by mode, Shift+A, right-click menus, F3, Z's pie, T and N. No Shift+A in edit mode; only Z is a pie; the outliner has no collections; Ctrl+P is Control+P on a Mac only.
- `scene.editor.selection` (P0, partial): Selecting by hand. The edit-mode selection is a set of vertices (rings and face loops select the band); no active element; no Select Similar, Random or Mirror.
- `scene.editor.fields-animation` (P0, partial): Number fields and animation basics. Auto-keying keys object transforms only; the timeline's own fields are the frame's; no reverse play.

## Differences on working features

- `scene.animation.nla-groups`: Editing children inside a meta requires ungrouping it first.
- `scene.camera.sensor-fit`: Square pixels only. Exchange formats preserve the fitted frame at matching output aspect, not the editable fit policy or original sensor identity; FBX import retains its existing focal-length limits. Blender 5.2.3 imports native USD orthographic apertures at 10 times their specified span.
- `scene.mesh.smooth-by-angle`: The angle is a mesh setting rather than the reference's Smooth by Angle modifier or marked sharp edges; no Keep Sharp Edges option, and imports do not read sharp edges. Normals are area-weighted, as plain smooth shading here.
- `scene.nodes.spline-types`: NURBS to Bezier requires cubic Bezier knots from a prior Bezier or Catmull-Rom conversion and equal positive weights; other NURBS must be resampled to Poly first. NURBS order is limited to 16, weights to 100000000, and new spline coordinates, radius and tilt to magnitude 1000000000000. No custom knot vectors or object-curve type changes.
- `scene.particles.boids`: Average airborne Goal/Avoid/Separate/Flock only; at-rest births, current target snapshots, bounded neighborhoods and deterministic turns differ from Blender's hidden seed state; no ground, scene forces, collisions, volume emission or cross-system flocking.
- `scene.physics.effector-weights`: Cloth and soft bodies only, for the five existing field kinds; weight-transfer identities match public observations while the existing solvers retain their trajectory and flow limitations. No particle, hair, rigid-world, fluid, collection-filter or unsupported-field weights.
- `scene.physics.motors`: Motor targets are static settings; animated target velocities and maximum impulses are not exposed.
- `scene.render.ambient-occlusion`: Screen-space estimate from the depth picture; the automatic distance (6 % of the scene's diagonal, 0.05 to 2 m) stays the default rather than the reference's fixed 0.2 m, so existing renders keep their look. No bent normals or bounce approximation; path-traced renders ignore these settings.
- `scene.render.color-management`: Standard stays the default (the reference's new files use AgX). AgX is a published polynomial approximation rather than the reference's configuration; no Filmic, False Color, Raw, display gamma, white balance, or curves. Only the rendered viewport and final renders are colour managed, not the material view.
- `scene.sculpt.multires-recovery`: Complete manifold quad subdivisions with an identifying corner or extraordinary vertex per component only; up to six levels, 100000 base faces and 250000 highest-level faces. Nonlinear UV detail, groups, shape keys, masks, face sets, hiding, seams and pins are refused. Intermediate recovered surfaces may differ from Blender.
- `scene.uv.live-unwrap`: UV selection remains synchronized with the mesh. Live unwrap is the UV Editor option; automatic seam or mesh-edit unwrapping in the 3D viewport is not implemented.
- `scene.uv.minimum-stretch`: Importance weights and optional flips are not exposed. Nondegenerate orientable charts are required; a noninjective pinned initialization is rejected. At most 128 islands and 20000 chart vertices per operation.
- `scene.viewport.rendered`: CPU previews use at most 512 pixels per side and 256 samples; motion blur and compositing are final-render features.

## Interaction

What a person used to Blender still finds different, from the acceptance run of 2026-11-26 (the
thirty items of `web/tests/first-ten-minutes.mjs`; the full table is kept with the plan's walkthrough
notes, outside this repository):

- Object mode's X deletes at once; Blender asks "Delete selected objects?" (`web/tests/delete-focus.mjs` asserts the deletion).
- G, R and S pick the Move, Rotate and Scale tools while their transform runs; the tool from before comes back when it ends. A press right after the key, before anything moved, is the tool's own drag, and that tool stays picked.
- The main keyboard's `.` opens the pivot menu; framing the selection is keypad `.` (or the View menu).
- Ctrl+P is Control+P on a Mac only; Cmd+P and other systems' Ctrl+P are Kition's.
- A tap of Space plays (under 250 ms); holding it is the frame's hand tool.
- No collections (M), no linked duplicates (Alt+D), one object in edit mode at a time, no R R trackball, no walk or fly, no local view.
- The inset is per face; no Outset, Boundary or Individual; bevel has no profile; loop cut has no smoothness (the headers read "Cuts: n" and "Offset: … m, Segments: n" without Blender's extra fields).
- A modal's typed value takes digits, the point, minus, slash, Backspace and Tab only; units and arithmetic work in the number fields, not in G, R, S, E or I.
- The side panel shows a transform's values once it is confirmed, not while it moves.
- The moving object keeps its orange outline (Blender draws it white); the active outline is #FFAA40 (Blender #FFA028); selected vertices, edges and faces are one orange (#F15800; Blender #FF7A00, #FFD800 and a see-through #FFB700).
- A document opens with nothing selected and the engine's own view (Blender selects the cube and shows the default view).
- In a browser window (the development host), keys the browser keeps for itself (Cmd+W, Cmd+T, Cmd+Q; Ctrl+Tab in some browsers) may never reach the editor; each has another route (the header's mode drop-down for Ctrl+Tab, the menus for the rest). Cmd+P is not taken by the editor and prints the page there.

## Catalogue

The catalogue has 64 features: 35 `has`, 29 not yet.
