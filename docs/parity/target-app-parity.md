# Target-app parity

> **Last updated:** 2026-11-17 · **Source:** `features.yaml` (updated 2026-11-17) and the engine's command list

Each feature in `features.yaml` records its status against what the feature promises and, where it
has one, the target application it is compared with (Blender 5.2.3 LTS).
The difference column is the feature's recorded gap.

| Feature | Priority | Status | Target | Difference |
| --- | --- | --- | --- | --- |
| `scene.agent.act` An agent replaces a material (assign, then delete the old one) | P0 | has | — | — |
| `scene.agent.context` An agent's call names the objects, time and mesh elements it acts on, and leaves the editor's selection, active object, mode, current frame and viewport as they were | P0 | has | — | — |
| `scene.agent.ids` One id space at the agent's tools; the scene in memory is the scene its saved file opens as | P0 | has | — | — |
| `scene.agent.observe` An agent reads the scene and places objects from it | P0 | has | — | — |
| `scene.agent.places` World points, element indexes and names are places; a write by them is refused after others changed what they name | P0 | has | — | — |
| `scene.agent.premises` Each call holds what it reads or replaces, for every write command; object parameters are marked; derived topology and bounds | P0 | has | — | — |
| `scene.agent.recover` An agent fixes what the check reports | P0 | has | — | — |
| `scene.animation.keyframes` Keyframe insertion through the command bridge | P0 | has | — | — |
| `scene.material.basics` Make, rename, colour and assign materials (roughness, transmission, IOR) | P0 | has | Blender 5.2.3 LTS | — |
| `scene.mesh.basics` Select faces in edit mode, inset and extrude them | P0 | has | Blender 5.2.3 LTS | — |
| `scene.modifier.basics` Add, change and reorder modifiers (subdivision, bevel, mirror) | P0 | has | Blender 5.2.3 LTS | — |
| `scene.object.basics` Add cubes, cylinders and spheres, delete objects, and place, turn and size them | P0 | has | Blender 5.2.3 LTS | — |
| `scene.agent.conversation` An agent builds a modifier stack over a conversation | P1 | has | — | — |
| `scene.agent.preview` An agent previews the scene from a named view or a camera at a time, in a pass, or a sheet of views or times, whatever the editor shows | P1 | has | — | — |
| `scene.agent.receipts` Compact equal-offset receipts and carried world transforms | P1 | has | — | — |
| `scene.animation.limits` F-curve Limits modifier | P1 | has | Blender 5.2.3 LTS | — |
| `scene.animation.nla-groups` NLA transition and meta strips | P1 | has | Blender 5.2.3 LTS | Editing children inside a meta requires ungrouping it first. |
| `scene.animation.timeline` Keyframes, frames, drivers, keying sets, actions, NLA and the graph editor | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.camera.sensor-fit` Camera sensor fit and sensor height | P1 | has | Blender 5.2.3 LTS | Square pixels only. Exchange formats preserve the fitted frame at matching output aspect, not the editable fit policy or original sensor identity; FBX import retains its existing focal-length limits. Blender 5.2.3 imports native USD orthographic apertures at 10 times their specified span. |
| `scene.collab.presence` Presence of other people in the scene | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.compositor.nodes` Compositor | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.curve.text-lattice` Curves, text objects and lattices | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.edit.history` Undo, redo, delete, duplicate and selection | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.editor.display-modes` The editor in each display mode (Menus, Single line, Classic, Focus); Focus shows the viewport alone | P1 | has | — | — |
| `scene.file.import-export` Import and export | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.files.usdc` Export binary USD layers with typed scene data | P1 | has | — | — |
| `scene.material.manage` Materials | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.mesh.edit` Mesh editing: select modes, extrude, inset, bevel, loop cut and the other Edit Mode tools | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.mesh.smooth-by-angle` Shade Smooth by Angle (split normals at sharp edges) | P1 | has | Blender 5.2.3 LTS | The angle is a mesh setting rather than the reference's Smooth by Angle modifier or marked sharp edges; no Keep Sharp Edges option, and imports do not read sharp edges. Normals are area-weighted, as plain smooth shading here. |
| `scene.mode.switch` Object, Edit, Pose, Sculpt and Weight Paint modes | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.modifier.stack` Modifier stack | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.nodes.geometry` Geometry node groups | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.nodes.spline-types` Geometry-node Catmull-Rom and NURBS splines | P1 | has | Blender 5.2.3 LTS | NURBS to Bezier requires cubic Bezier knots from a prior Bezier or Catmull-Rom conversion and equal positive weights; other NURBS must be resampled to Poly first. NURBS order is limited to 16, weights to 100000000, and new spline coordinates, radius and tilt to magnitude 1000000000000. No custom knot vectors or object-curve type changes. |
| `scene.object.add` Add meshes, curves, lights, cameras and empties | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.object.transform` Object transforms, parenting, origins and visibility | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.particles.boids` Airborne boids with averaged steering rules | P1 | has | Blender 5.2.3 LTS | Average airborne Goal/Avoid/Separate/Flock only; at-rest births, current target snapshots, bounded neighborhoods and deterministic turns differ from Blender's hidden seed state; no ground, scene forces, collisions, volume emission or cross-system flocking. |
| `scene.particles.hair` Particle systems and grooming | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.physics.effector-weights` Independent cloth and soft-body field weights | P1 | has | Blender 5.2.3 LTS | Cloth and soft bodies only, for the five existing field kinds; weight-transfer identities match public observations while the existing solvers retain their trajectory and flow limitations. No particle, hair, rigid-world, fluid, collection-filter or unsupported-field weights. |
| `scene.physics.motors` Rigid body constraint motors | P1 | has | Blender 5.2.3 LTS | Motor targets are static settings; animated target velocities and maximum impulses are not exposed. |
| `scene.physics.simulation` Rigid bodies, cloth, soft bodies, fields and constraints | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.render.ambient-occlusion` Ambient occlusion settings for fast renders | P1 | has | Blender 5.2.3 LTS | Screen-space estimate from the depth picture; the automatic distance (6 % of the scene's diagonal, 0.05 to 2 m) stays the default rather than the reference's fixed 0.2 m, so existing renders keep their look. No bent normals or bounce approximation; path-traced renders ignore these settings. |
| `scene.render.color-management` Colour management of renders (view transform, look, exposure) | P1 | has | Blender 5.2.3 LTS | Standard stays the default (the reference's new files use AgX). AgX is a published polynomial approximation rather than the reference's configuration; no Filmic, False Color, Raw, display gamma, white balance, or curves. Only the rendered viewport and final renders are colour managed, not the material view. |
| `scene.render.lights-world` Lights, cameras, world and render settings | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.rigging.armature` Bones, poses, vertex groups, shape keys and constraints | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.sculpt.multires-recovery` Multiresolution Unsubdivide and Rebuild Subdivisions | P1 | has | Blender 5.2.3 LTS | Complete manifold quad subdivisions with an identifying corner or extraordinary vertex per component only; up to six levels, 100000 base faces and 250000 highest-level faces. Nonlinear UV detail, groups, shape keys, masks, face sets, hiding, seams and pins are refused. Intermediate recovered surfaces may differ from Blender. |
| `scene.sculpt.paint` Sculpting, multiresolution and weight painting | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.uv.editing` UV projections and UV editing | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.uv.live-unwrap` Live relaxation while pinned UVs move | P1 | has | Blender 5.2.3 LTS | UV selection remains synchronized with the mesh. Live unwrap is the UV Editor option; automatic seam or mesh-edit unwrapping in the 3D viewport is not implemented. |
| `scene.uv.minimum-stretch` Minimum-stretch UV unwrapping | P1 | has | Blender 5.2.3 LTS | Importance weights and optional flips are not exposed. Nondegenerate orientable charts are required; a noninjective pinned initialization is rejected. At most 128 islands and 20000 chart vertices per operation. |
| `scene.view.viewport` Viewport shading, views and overlays | P1 | partial | Blender 5.2.3 LTS | The engine has these commands; no case yet shows the area working end to end in the editor and surviving save and reopen. |
| `scene.viewport.rendered` Progressive rendered shading in the viewport | P1 | has | Blender 5.2.3 LTS | CPU previews use at most 512 pixels per side and 256 samples; motion blur and compositing are final-render features. |

| Priority | has | partial | missing |
| --- | ---: | ---: | ---: |
| P0 | 12 | 0 | 0 |
| P1 | 19 | 20 | 0 |
