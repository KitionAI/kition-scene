# Gaps

> **Last updated:** 2026-11-17 · **Source:** `features.yaml` (updated 2026-11-17) and the engine's command list

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
- `scene.view.viewport` (P1, partial): Viewport shading, views and overlays. The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen.

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

## Catalogue

The catalogue has 51 features: 31 `has`, 20 not yet.
