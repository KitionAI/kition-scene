//! What each command does and what each read command answers, for the model choosing a command
//! (`KitCommand.description`, `output_schema`; kition-sdk `testing::check_commands`).
use kition_kit::KitCommand;
use serde_json::Value;

/// Read commands that answer nothing a model reads.
pub const NO_OUTPUT: &[(&str, &str)] = &[];

/// Sets each command's description and, for read commands, its output schema.
pub fn fill(commands: &mut [KitCommand]) {
    for command in commands {
        if let Some(Value::Object(properties)) = command.schema.as_mut() {
            for (key, node) in properties.iter_mut() {
                if node.get("type").and_then(Value::as_str) == Some("json") {
                    type_key(&command.id, key, node);
                }
            }
        }
        command.description = describe(&command.id).to_owned();
        if command.output_schema.is_none() {
            command.output_schema = output(&command.id);
        }
    }
}

fn describe(id: &str) -> &'static str {
    match id {
        "edit.undo" => {
            "Takes back the caller's last edit (the person's, or the agent's with `scope: \"agent\"`), leaving the other's edits in place."
        }
        "edit.redo" => "Applies again the edit the caller's last undo took back, in the same scope as `edit.undo`.",
        "edit.delete" => "Deletes `objects` (names or ids; the selection without it) from the scene.",
        "edit.duplicate" => {
            "Duplicates `objects` (the selection without it), moved by `offset`, and answers the copies' ids; a person's duplicate selects the copies, an agent's given `objects` leaves the selection as it was."
        }
        "select.all" => {
            "Selects every object in object mode (or every element in edit mode); `action: \"toggle\"`/`\"deselect\"` changes that. Answers whether the selection changed."
        }
        "select.none" => "Clears the selection. Answers whether it changed.",
        "select.objects" => {
            "Selects the objects in `objects` (names or ids), adding with `extend` or removing with `deselect`, for selection-based commands."
        }
        "add.backdrop" => {
            "Adds a backdrop (a cyclorama): a floor curving up into a wall, smooth, so renders show no horizon. `width`, `depth` (floor in front of the curve, centred on the origin), `height`, and `radius` in metres, `segments` round the curve."
        }
        "camera.move" => {
            "Keys a preset camera move round `target` (an object or a point) from frame `start` for `duration` frames, turning to aim at the target, as one undo step: pushIn (dolly in by `amount`), pushHover (push in 18 frames swinging `yaw` and `tilt`, hover creeping in, settle back), orbit (`degrees` round at constant speed, `easeEnds` only at the ends), craneRise (from close and low, `scale` 3 to 1, rise and pull back to where the camera is), reveal (from a close-up out to where the camera is), pullBack (out by `amount`), whip (a fast pan to `to`, turning motion blur on), follow (keeps an `offset` from a moving object, aimed `lead` seconds ahead, lagging by `smoothing` seconds), zoomThrough (straight in to the target's surface, the distance shrinking in log space so the approach looks steady; `through` arrives at full speed with the surface's colour filling the frame), pullOut (the reverse: from filling the frame with the surface back out to a wide shot). Moves start where the camera is and as it looks at `start` (`from: \"current\"`), so they chain into one continuous take; `position` builds the move round that place instead. Optional `lens` to reach and `ease` [x1, y1, x2, y2]. Eases are baked as one linear key per frame."
        }
        "camera.shake" => {
            "Adds a decaying shake on top of the camera's keys from `frame` on (nothing before), as one undo step: jolts of `amplitude` metres and `rotation` degrees at `frequency` per second, dying away at `decay` per second; seeded noise, so the same `seed` gives the same shake. Run it after the camera moves it rides on."
        }
        "camera.handheld" => {
            "Adds a slow handheld float on top of the camera's keys from `start` to `end`, as one undo step: two sine sways of `period` seconds (2.7 and 4.1) up to `amount` metres and `rotation` degrees, fading to exactly nothing inside each of `holds` ([[first, last], ...]) and at its ends over 6 frames. Run it after the camera moves it rides on."
        }
        "light.studio" => {
            "Builds or updates a three-point light rig on `target` as one undo step: Key Light (a spot camera-left), Fill Light (a cool area light), Rim Light (a warm spot from behind), named so they can be edited; sets the world strength, ambient occlusion, and the AgX view. `preset` product, dramatic, or soft; `distance` in metres."
        }
        "object.drop" => {
            "Keys `objects` falling in from `height` metres above where they rest at `start`, accelerating as under gravity over `duration` frames, with an optional `bounce` and a stretch, squash and settle as they land, as one undo step."
        }
        "object.hop" => {
            "Keys `objects` hopping on a ballistic arc from where they are at `start` to `to` (a point for their centre, or an object to land on top of) over `duration` frames, rising `height` above the higher end, with optional `spin` degrees, landing with a squash that rings out (`impact` {squash, frames}), as one undo step. Answers the landing frame."
        }
        "object.gather" => {
            "Keys `objects` swirling in from where they are at `start` to an evenly spaced ring of `radius` round `center` (0: a cluster just touching), each on a spiral that turns `arc` degrees on the way, `stagger` frames apart (shrinking gaps with `accelerate`), each over `duration` frames, as one undo step."
        }
        "object.wave" => {
            "Keys a wave running out from `origin` at `speed` metres a second over `objects`, as one undo step: as it reaches each one (by its distance), it bumps in size by `amount` (property scale) or switches its own material's base colour to `color` and leaves it there (property color), over `duration` frames. Separate objects only, not an Array modifier's copies."
        }
        "object.pop" => {
            "Keys `objects` growing from nothing to their size with an overshoot over `duration` frames, one after another `stagger` frames apart (shrinking gaps with `accelerate`), as one undo step."
        }
        "add.plane" => {
            "Adds a plane mesh of `size` at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.cube" => {
            "Adds a cube mesh of `size` at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.circle" => {
            "Adds a circle mesh of `radius` and vertex count, optionally filled at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.uvSphere" => {
            "Adds a UV sphere mesh of `radius` with segments and rings at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.icoSphere" => {
            "Adds a icosphere mesh of `radius` and subdivision level at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.cylinder" => {
            "Adds a cylinder mesh of `radius` and `depth` at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.cone" => {
            "Adds a cone mesh with bottom and top radii and `depth` at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.torus" => {
            "Adds a torus mesh with major and minor radii and segments at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.grid" => {
            "Adds a grid mesh of `size` with x and y subdivisions at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.light" => {
            "Adds a light (point, sun, spot or area) with color and power at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.camera" => {
            "Adds a camera with a lens in millimetres at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.empty" => {
            "Adds a empty (an axis marker for parenting and targets) at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "object.transform" => "Sets location, rotation (degrees) and/or scale of `objects` (names or ids; the selection without it) absolutely.",
        "object.translate" => "Moves `objects` (names or ids; the selection without it) by `offset` [x, y, z].",
        "object.rotate" => "Rotates `objects` (names or ids; the selection without it) by `angle` degrees about `axis` (x, y or z).",
        "object.scale" => "Scales `objects` (names or ids; the selection without it) by `factor` (a number or [x, y, z]).",
        "object.applyTransform" => {
            "Bakes the location, rotation and/or scale of `objects` (names or ids; the selection without it) into the mesh data and resets them."
        }
        "object.originToGeometry" => {
            "Moves the origin of `objects` (names or ids; the selection without it) to the centre of their geometry without moving the geometry."
        }
        "object.order" => "Moves `object` to position `index` among its siblings in the outliner.",
        "object.rename" => "Renames `object` (name or id) to `name`.",
        "object.parent" => "Parents `objects` (names or ids; the selection without it) to `parent`, keeping their world transform.",
        "object.clearParent" => "Clears the parent of `objects` (names or ids; the selection without it), keeping their world transform.",
        "object.hide" => "Hides `objects` (names or ids; the selection without it) in the viewport and renders.",
        "object.reveal" => "Shows `objects` (names or ids; the selection without it) again, or every hidden object without it.",
        "object.shadeSmooth" => "Shades the meshes of `objects` (names or ids; the selection without it) smooth.",
        "object.shadeSmoothByAngle" => {
            "Shades the meshes of `objects` (names or ids; the selection without it) smooth, keeping edges sharper than `angle` degrees (30) sharp."
        }
        "object.shadeFlat" => "Shades the meshes of `objects` (names or ids; the selection without it) flat.",
        "object.join" => "Joins the selected meshes into the active one.",
        "object.setCamera" => "Makes camera `object` the scene's active camera for renders.",
        "object.lookAt" => "Rotates `object` so it points at `target` [x, y, z] (cameras and lights look along -Z).",
        "mode.toggle" => "Toggles the active mesh between object mode and edit mode.",
        "mode.set" => "Sets object or edit mode for `object` (else the active object). Edit-mode mesh commands need `mode: \"edit\"` first.",
        "mesh.selectMode" => "Sets edit mode's element selection to vertices, edges or faces.",
        "mesh.selectVertexMode" => "Switches edit mode to vertex selection.",
        "mesh.selectEdgeMode" => "Switches edit mode to edge selection.",
        "mesh.selectFaceMode" => "Switches edit mode to face selection.",
        "mesh.select" => "Selects vertices and/or faces of the edited mesh by index (or all), adding with `extend` or removing with `deselect`.",
        "mesh.extrude" => {
            "Extrudes the selected faces (or loose vertices and edges) along their normals by `distance`, plus `offset`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.inset" => {
            "Insets the selected faces by `thickness`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.bevel" => {
            "Bevels the selected edges with `width` and `segments`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.subdivide" => {
            "Subdivides the selected edges and faces, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.loopCut" => "Adds `cuts` edge loops across the ring of edge `edge` [vertex, vertex] at `factor` in the edited mesh.",
        "mesh.merge" => {
            "Merges the selected vertices at their centre, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.deleteFaces" => {
            "Deletes the selected faces, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.deleteVertices" => {
            "Deletes the selected vertices and the faces using them, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.fill" => "Makes a face from the vertices in `vertices` (else the selection) of the edited mesh.",
        "mesh.flip" => {
            "Flips the normals of the selected faces, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.recalculateNormals" => "Recalculates the normals of `objects` (names or ids; the selection without it) so they point outside.",
        "mesh.translate" => {
            "Moves the selected elements by `offset`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.scale" => {
            "Scales the selected elements by `factor` about `pivot`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.setGeometry" => "Replaces the mesh of `object` with `positions` and `faces` (vertex index lists), optionally smooth-shaded.",
        "modifier.add" => "Adds a modifier (subdivision, mirror, array, bevel, solidify, boolean, displace, …) with its settings to `object`.",
        "modifier.set" => "Changes the settings of modifier `index` or `name` on `object`.",
        "modifier.remove" => "Removes modifier `index` or `name` from `object`.",
        "modifier.apply" => "Applies modifier `index` or `name` to `object`'s mesh and removes it.",
        "modifier.move" => "Moves modifier `index` or `name` to position `to` in `object`'s stack.",
        "material.new" => {
            "Creates a material with base color, metallic and roughness and answers its id. It is given to the selected objects (in Edit mode, to the object edited) unless `assign: false`; the answer lists them."
        }
        "material.set" => "Changes material `material` (name or id): base color, metallic, roughness, emission, alpha, transmission and more.",
        "material.assign" => "Assigns material `material` to `objects` (names or ids; the selection without it).",
        "material.clear" => "Removes the materials from `objects` (names or ids; the selection without it).",
        "material.delete" => "Deletes material `material`; refused while objects use it unless `force`.",
        "light.set" => "Changes light `object` (else the selected light): kind, color, power, radius, spot size.",
        "camera.set" => {
            "Changes camera `object`: lens, clipping, sensor width (`sensor`), `sensorHeight` (1–200 mm), `sensorFit` (auto, horizontal, vertical), depth of field and aperture blades. Auto fits sensor width to the longer render dimension; Horizontal holds width; Vertical holds sensor height. Orthographic scale follows the same fitted dimension. Square render pixels only."
        }
        "world.set" => "Sets the world background color and strength, rotation and volume.",
        "render.set" => {
            "Sets render width and height, motion blur and shutter, ambient occlusion (on, distance, strength), and colour management (view transform, look, exposure)."
        }
        "render.image" => {
            "Renders the active camera's view (at `frame`, with `samples`) and answers it as a base64 PNG with its size. Does not change the scene."
        }
        "file.render" => crate::render::DESCRIPTION,
        "file.export" => {
            "Exports the scene (or `selectedOnly`) as glTF/GLB, OBJ, STL, FBX, USD (usda, usdc, usdz) or a PNG render and answers the file as base64. Does not change the scene."
        }
        "file.import" => "Imports a glTF/GLB, OBJ, STL, FBX, USD or .blend file from the workspace (`path`) or base64 into the scene.",
        "view.shading" => "Sets the viewport shading to wireframe, solid, material preview or rendered.",
        "view.wireframe" => "Sets the viewport shading to wireframe.",
        "view.solid" => "Sets the viewport shading to solid.",
        "view.rendered" => "Shows progressive path tracing of the viewport with scene lights, materials and editor overlays.",
        "view.renderedStep" => {
            "Advances the rendered viewport in bounded worker steps; a headless caller can wait for the completed preview. Sampling never changes the document or undo history."
        }
        "view.material" => "Sets the viewport shading to material preview.",
        "view.front" => "Looks at the scene from the front.",
        "view.right" => "Looks at the scene from the right.",
        "view.top" => "Looks at the scene from the top.",
        "view.camera" => "Looks through the active camera.",
        "view.perspective" => "Toggles the viewport between perspective and orthographic.",
        "view.frameAll" => "Frames every object in the viewport.",
        "view.frameSelected" => "Frames the selected objects in the viewport.",
        "view.grid" => "Shows or hides the grid floor in the viewport.",
        "view.overlays" => {
            "Shows or hides every overlay: the grid, lights, cameras, empties, origins, outlines and edit marks, so the viewport (a rendered camera view too) shows only the scene."
        }
        "view.orbit" => "Orbits the viewport by `turn` and `tilt` degrees.",
        "view.pan" => "Pans the viewport by `right` and `up`.",
        "view.zoom" => "Zooms the viewport by `factor` (above 1 zooms in).",
        "view.set" => "Sets the viewport's target, distance, turn, tilt and projection at once.",
        "view.resize" => "Sets the viewport size in pixels, which previews and the camera view use.",
        "mesh.extrudeIndividual" => {
            "Extrudes each selected face separately along its own normal by `distance`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.dissolveVertices" => {
            "Dissolves the selected vertices, merging the faces around them, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.dissolveEdges" => {
            "Dissolves the selected edges, merging the faces on both sides, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.dissolveFaces" => {
            "Dissolves the selected faces into one face, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.bridge" => {
            "Bridges two selected edge loops with faces, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.bisect" => {
            "Cuts the whole edited mesh, whatever is selected, along the plane through `point` (the selection's centre when left out) with `normal`, optionally filling the cut and clearing one side."
        }
        "mesh.separate" => "Moves the selected elements of the edited mesh into a new object named `name`.",
        "mesh.symmetrize" => "Makes the whole edited mesh, whatever is selected, symmetric across `axis`, copying one side to the other.",
        "mesh.spin" => "Spins the selected elements `angle` degrees about `axis` through `center` in `steps`, making a lathe.",
        "mesh.bevelVertices" => {
            "Bevels the selected vertices with `width`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.rotate" => {
            "Rotates the selected elements by `angle` degrees about `axis` through `pivot`, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.proportional" => "Turns proportional editing on or off and sets its falloff and radius; answers the setting.",
        "uv.smartProject" => "Unwraps `objects` (names or ids; the selection without it) with smart UV project (by angle limit, with margin).",
        "uv.cubeProject" => "Projects UVs of `objects` (names or ids; the selection without it) from a cube of `size`.",
        "uv.cylinderProject" => "Projects UVs of `objects` (names or ids; the selection without it) from a cylinder.",
        "uv.sphereProject" => "Projects UVs of `objects` (names or ids; the selection without it) from a sphere.",
        "uv.planarProject" => "Projects UVs of `objects` (names or ids; the selection without it) flat along `axis`.",
        "uv.reset" => "Resets the UVs of `objects` (names or ids; the selection without it) so each face fills the whole UV square.",
        "uv.clear" => "Removes the UV maps of `objects` (names or ids; the selection without it).",
        "material.setTexture" => {
            "Sets an image (workspace `path` or base64) as a texture slot (base color, normal, roughness, …) of `material`, or clears it."
        }
        "modifier.setImage" => "Sets the image a displace modifier reads (workspace path or base64), or clears it.",
        "world.setImage" => "Sets an environment image (workspace path or base64) as the world background, with rotation and strength, or clears it.",
        "render.trace.start" => {
            "Starts a path-traced render (bounces, samples, denoise) in steps; answers its progress. Continue with `render.trace.step`."
        }
        "render.trace.step" => "Continues the path-traced render for `budgetMs` and answers its progress, with the picture so far when `preview`.",
        "render.trace.image" => "Answers the path-traced picture so far as a base64 PNG.",
        "render.trace.cancel" => "Cancels the path-traced render under way.",
        "mesh.markSeam" => {
            "Marks the selected edges as UV seams, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "mesh.clearSeam" => {
            "Clears UV seams from the selected edges, in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "uv.unwrap" => {
            "Unwraps the edited mesh's selected faces along their seams (angle-based, conformal, or minimumStretch SLIM) with `margin`; minimum stretch uses 1 to 100 `iterations` and keeps pins fixed."
        }
        "uv.stitch" => "Stitches the selected UV edges to their neighbours in the UV editor.",
        "uv.pin" => "Pins the selected UVs so unwrapping keeps them in place.",
        "uv.unpin" => "Unpins the selected UVs.",
        "uv.translate" => "Moves the selected UVs by `offset`.",
        "uv.rotate" => "Rotates the selected UVs by `angle` degrees about `pivot`.",
        "uv.scale" => "Scales the selected UVs by `factor` about `pivot`.",
        "uv.liveUnwrap" => {
            "Sets local UV live unwrap preferences (`on`, `method`, `iterations`) without changing the scene. Pinned UV transforms relax their islands in one undo step."
        }
        "uv.preview" => {
            "Answers a UV layout after a proposed `offset`, including live relaxation (`source` rejects a stale drag), without changing the scene, selection, history, or preferences."
        }
        "uv.layout" => "Answers the edited mesh's UV layout (faces and their UV coordinates), all faces or the selected ones.",
        "mesh.knife" => "Cuts the edited mesh along `points` (screen-space path), through the whole mesh with `through`.",
        "mesh.edgeSlide" => {
            "Slides the selected edges along their neighbours by `factor` (-1 to 1), in edit mode on the edited mesh's selected elements (`mode.set {mode: \"edit\"}` and `mesh.select` first)."
        }
        "anim.insertKey" => {
            "Inserts keyframes for `properties` (location, rotation, scale, …) or a keying set of `objects` (names or ids; the selection without it) at `frame`."
        }
        "anim.deleteKey" => "Deletes the keyframes for `properties` of `objects` (names or ids; the selection without it) at `frame`.",
        "anim.clear" => "Clears every keyframe (or those of `properties`) of `objects` (names or ids; the selection without it).",
        "anim.setInterpolation" => {
            "Sets the interpolation (constant, linear, bezier, …) of keyframes of `objects` (names or ids; the selection without it) at `frame`."
        }
        "frame.set" => "Goes to frame `frame` and answers it; the scene shows its animated state there.",
        "frame.next" => "Goes to the next frame and answers it.",
        "frame.previous" => "Goes to the previous frame (wrapping to the end) and answers it.",
        "frame.start" => "Goes to the first frame of the range and answers it.",
        "frame.end" => "Goes to the last frame of the range and answers it.",
        "anim.nextKey" => "Goes to the next keyframe of the selected objects and answers its frame.",
        "anim.previousKey" => "Goes to the previous keyframe of the selected objects and answers its frame.",
        "frame.range" => "Sets the animation's start and end frames and frames per second.",
        "render.animation" => "Renders frames `start` to `end` (every `step`) and answers them as pictures. Does not change the scene.",
        "driver.add" => "Adds a driver to a property (of an object, bone or shape key) computed from `expression` and its variables.",
        "driver.set" => "Changes a driver's expression or variables.",
        "driver.remove" => "Removes the driver from a property.",
        "keyingSet.new" => "Adds a keying set named `name` to the scene.",
        "keyingSet.remove" => "Removes keying set `name`.",
        "keyingSet.addPaths" => "Adds properties of objects or bones to keying set `name`.",
        "keyingSet.removePath" => "Removes path `index` from keying set `name`.",
        "keyingSet.setActive" => "Makes keying set `name` the one `anim.insertKey` uses by default.",
        "action.new" => "Creates an action named `name` and assigns it to `object`.",
        "action.assign" => "Assigns action `action` to `objects` (names or ids; the selection without it).",
        "action.remove" => "Deletes action `name` from the file; objects using it lose their animation.",
        "action.rename" => "Renames action `name` to `to`.",
        "nla.pushDown" => "Pushes `object`'s active action down into a new NLA strip.",
        "nla.addTrack" => "Adds an NLA track to `object` at `index`.",
        "nla.removeTrack" => "Removes NLA track `track` from `object`.",
        "nla.setTrack" => "Changes NLA track `track`: name, mute, solo, position.",
        "nla.addStrip" => "Adds a strip of action `action` to NLA track `track` of `object` at frame `start`.",
        "nla.addTransition" => {
            "Adds a transition between strip `strip` and its next neighbor on track `track`, blending their endpoint evaluations across the gap."
        }
        "nla.makeMeta" => {
            "Groups the contiguous strips `first` through `last` on `track` into one meta strip; its start and end move or stretch the children together."
        }
        "nla.dissolveMeta" => "Ungroups meta strip `strip` on `track`, preserving the children's current timing and blending.",
        "nla.setStrip" => {
            "Changes NLA strip `strip` of track `track`: action range, blending, repeats, scale; a meta uses `start` and `end` to move or stretch its children. Transitions follow their neighbors."
        }
        "nla.removeStrip" => "Removes NLA strip `strip` from track `track` of `object`.",
        "nla.setAction" => "Sets how `object`'s active action blends over the NLA (blend mode, extrapolation, influence).",
        "nla.tweak" => "Starts tweaking the action of NLA strip `strip` so keyframe edits go into it.",
        "nla.tweakExit" => "Stops tweaking the NLA strip's action.",
        "graph.addModifier" => "Adds an F-curve modifier (cycles, noise, generator, …) to an animated property's curve.",
        "graph.setModifier" => "Changes an F-curve modifier's settings.",
        "graph.removeModifier" => "Removes an F-curve modifier from a curve.",
        "add.armature" => "Adds an armature with one bone at `location` and answers its name and object id.",
        "mode.armature" => "Toggles the armature `object` (the active one without it) between object mode and bone edit mode.",
        "mode.pose" => "Turns pose mode on or off for armature `object`.",
        "bone.add" => "Adds a bone with head and tail to the armature in edit mode, optionally parented and connected.",
        "bone.extrude" => "Extrudes a new bone from the selected bone's tail by `length` or to `tail`.",
        "bone.set" => "Changes bone `bone`: name, head, tail, roll, parent, connection, deform.",
        "bone.select" => "Selects bones by name in edit or pose mode, adding with `extend` or removing with `deselect`.",
        "pose.set" => "Sets location, rotation and/or scale of pose bones `bones`.",
        "pose.clear" => "Clears the pose of `bones` (or all) back to the rest pose.",
        "pose.insertKey" => "Inserts keyframes for pose bones `bones` at `frame`.",
        "pose.deleteKey" => "Deletes the `properties` keyframes (location, rotation, scale; all without it) of pose bones `bones` at `frame`.",
        "object.parentArmature" => {
            "Parents `objects` (names or ids; the selection without it) to `armature` with automatic, envelope or empty weights."
        }
        "vertexGroup.add" => "Adds a vertex group named `name` to `object`.",
        "vertexGroup.remove" => "Removes vertex group `name` from `object`.",
        "vertexGroup.assign" => "Assigns vertices (by index, else the selected ones) to vertex group `name` with `weight`.",
        "vertexGroup.unassign" => "Removes vertices (by index, else the selected ones) from vertex group `name`.",
        "vertexGroup.select" => "Selects the vertices of vertex group `name` in edit mode.",
        "shapeKey.add" => "Adds a shape key named `name` to `object`, from the basis or `from`, optionally with vertex `offsets`.",
        "shapeKey.remove" => "Removes shape key `name` from `object`.",
        "shapeKey.set" => "Sets shape key `name`'s value (0 to 1) or renames it.",
        "shapeKey.offset" => "Moves vertices of shape key `name` by `offset`.",
        "shapeKey.insertKey" => "Inserts a keyframe for shape key `name`'s value at `frame`.",
        "graph.moveKey" => "Moves a keyframe of an animated property from `frame` to `toFrame`.",
        "graph.deleteKey" => "Deletes the keyframe at `frame` of an animated property.",
        "graph.setInterpolation" => "Sets the interpolation of the keyframe at `frame` of an animated property.",
        "constraint.add" => "Adds a constraint (track to, copy location, IK, limit, child of, …) with its settings to `object` or one of its bones.",
        "constraint.set" => "Changes the settings of constraint `index` or `name`.",
        "constraint.remove" => "Removes constraint `index` or `name`.",
        "constraint.move" => "Moves constraint `index` or `name` to position `to` in the stack.",
        "constraint.apply" => {
            "Applies constraint `index` or `name` to the transform (its effect alone, on the object's own transform) and removes it; the others stay."
        }
        "constraint.setInverse" => "Sets the inverse of a child-of constraint so the object stays where it is.",
        "constraint.clearInverse" => "Clears the inverse of a child-of constraint.",
        "mode.weightPaint" => "Turns weight paint mode on or off for mesh `object`.",
        "paint.group" => "Sets the vertex group weight painting paints into, and answers it.",
        "paint.brush" => {
            "Sets the weight paint brush (mode, weight, radius, strength, falloff, front faces only) and X mirror (`xMirror`, `topologyMirror`, `mirrorGroups`); answers them."
        }
        "paint.tool" => "Sets what the pointer does in weight paint: `tool` brush, linearGradient, radialGradient, or sampleWeight; answers it.",
        "paint.sample" => {
            "Sets the brush weight to the paint group's weight at viewport pixel `at` (the nearest vertex of the face there); answers it."
        }
        "paint.gradient" => {
            "Fills group `group` (the paint group without it) with a `kind` linear or radial gradient from viewport pixel `from` (full brush weight) to `to`, as one undo step."
        }
        "paint.stroke" => "Paints weights into vertex group `group` along `points` with the current brush.",
        "paint.vertices" => "Sets, adds or subtracts `weight` on vertices of group `group` directly.",
        "paint.normalizeAll" => "Normalizes every vertex's weights to sum to 1, optionally keeping the active group.",
        "mode.sculpt" => "Turns sculpt mode on or off for mesh `object`.",
        "sculpt.brush" => "Sets the sculpt brush (kind, radius, strength, falloff, auto-smooth) and answers it.",
        "sculpt.stroke" => "Sculpts along `points` (or a `drag`) with a brush.",
        "sculpt.maskInvert" => "Inverts the sculpt mask.",
        "sculpt.maskClear" => "Clears the sculpt mask.",
        "sculpt.maskFill" => "Masks the whole mesh.",
        "sculpt.maskSmooth" => "Smooths the sculpt mask's edges.",
        "sculpt.tool" => "Selects a sculpt tool and answers it.",
        "sculpt.dyntopo" => "Turns dynamic topology on or off and sets its detail mode and size.",
        "sculpt.detailFlood" => "Remeshes the whole mesh to the dynamic topology detail size.",
        "sculpt.gesture" => "Applies a box or lasso gesture (mask, hide, face set, trim) over screen `points`.",
        "sculpt.faceSetsFromMask" => "Makes a face set from the masked faces.",
        "sculpt.faceSetsFromVisible" => "Makes a face set from the visible faces.",
        "sculpt.faceSetsFromLooseParts" => "Makes one face set per loose part of the mesh.",
        "sculpt.faceSetsClear" => "Removes every face set.",
        "sculpt.faceSetVisibility" => "Shows, hides or isolates face set `faceSet` (or the one under `x`, `y`).",
        "sculpt.hideFaceSet" => "Hides the face set under the pointer.",
        "sculpt.isolateFaceSet" => "Shows only the face set under the pointer.",
        "sculpt.showAllFaces" => "Shows every hidden face again.",
        "sculpt.remesh" => "Remeshes `object` with voxels of `voxelSize`.",
        "multires.subdivide" => "Adds a multiresolution subdivision level to `object`.",
        "multires.unsubdivide" => {
            "Recovers one quad subdivision level below the base mesh, preserving existing surfaces and detail. Requires a Multires modifier, manifold quad topology with a boundary corner or extraordinary interior vertex per component (at most 100000 base faces and 250000 highest-level faces), linearly subdivided UVs, and no vertex groups, shape keys, masks, face sets, hidden faces, seams or UV pins. Unsupported inputs leave the document unchanged."
        }
        "multires.rebuildSubdivisions" => {
            "Recovers all complete quad subdivision levels, up to six, from a mesh with a Multires modifier and no existing levels. Disconnected components stop at their common depth. Preserves the input surface as the highest level; has the same topology and attribute restrictions as multires.unsubdivide."
        }
        "multires.deleteHigher" => "Deletes the multiresolution levels above the current one.",
        "multires.applyBase" => "Applies the multiresolution detail to the base mesh's shape.",
        "multires.reshape" => "Copies the shape of object `from` into `object`'s multiresolution levels.",
        "particles.add" => "Adds a particle system (Newtonian or airborne averaged boid emitter, or hair) with its settings to `object`.",
        "particles.set" => {
            "Changes particle system `system`'s settings, including airborne boids with averaged Goal, Avoid, Separate and Flock rules. Targets use their current world-space positions; animated target history, collisions and force fields are unsupported."
        }
        "particles.remove" => "Removes particle system `system` from `object`.",
        "particles.convert" => "Converts particle system `system` into a mesh object.",
        "particles.stats" => "Answers particle system `system`'s counts (particles, alive, children) at `frame`.",
        "mode.particleEdit" => "Turns particle edit mode on or off for a hair system.",
        "particleEdit.brush" => "Sets the grooming brush (comb, cut, length, puff, smooth, …) and answers it.",
        "particleEdit.stroke" => "Grooms hair along `points` with a brush.",
        "particleEdit.reset" => "Deletes the grooming edits of the hair system.",
        "presence.local" => "Shares this person's pointer and selection with collaborators; answers what it shares.",
        "presence.peers" => "Tells the editor where other people's pointers and selections are, for drawing them.",
        "presence.place" => "Answers where to draw other people's pointers in the viewport.",
        "add.curveBezier" => {
            "Adds a Bezier curve at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.curveCircle" => {
            "Adds a Bezier circle curve at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.curvePath" => {
            "Adds a NURBS path curve at `location` (with rotation, scale and `name`) and answers its name and object id; name it to address it in the same run."
        }
        "add.text" => "Adds a text object with `body` and `size` at `location` and answers its name and object id.",
        "mode.shape" => "Toggles the active curve, text or lattice between object mode and its edit mode.",
        "curve.set" => "Changes curve `object`: dimensions, resolution, extrude, bevel depth and resolution, fill, cyclic.",
        "text.set" => "Changes text `object`: body, font size, alignment, extrude, bevel, spacing.",
        "object.convertToMesh" => "Converts curves and texts in `objects` (names or ids; the selection without it) into meshes.",
        "curve.setSplines" => "Replaces the splines of curve `object` with `splines` (points, handles, cyclic).",
        "curve.select" => "Selects curve points by index in curve edit mode.",
        "curve.translate" => "Moves the selected curve points by `offset`.",
        "curve.extrude" => "Extrudes new points from the selected curve end points by `offset`.",
        "curve.subdivide" => "Subdivides the segments between selected curve points.",
        "curve.toggleCyclic" => "Opens or closes the selected splines.",
        "curve.setHandleType" => "Sets the selected points' handle type (auto, vector, aligned, free).",
        "curve.deletePoints" => "Deletes the selected curve points.",
        "text.insert" => "Types `text` at the caret of text object `object` in text edit mode.",
        "text.delete" => "Deletes `count` characters backward (or forward) from the caret in text edit mode.",
        "text.caret" => "Moves the caret in text edit mode by `by`, to `to`, or to a place (start, end, line start, …) and answers it.",
        "add.lattice" => "Adds a lattice of `resolution` at `location` and answers its name and object id.",
        "lattice.set" => "Changes lattice `object`: resolution and interpolation, or resets its points.",
        "lattice.movePoints" => "Moves lattice points (by index, else the selected ones) by `offset`.",
        "lattice.select" => "Selects lattice points by index in lattice edit mode.",
        "physics.world.set" => "Sets the physics world: gravity, frame range, steps and iterations.",
        "physics.rigidBody.add" => {
            "Makes `objects` (names or ids; the selection without it) rigid bodies (active or passive) with mass, shape, friction and damping."
        }
        "physics.rigidBody.set" => "Changes the rigid body settings of `objects` (names or ids; the selection without it).",
        "physics.rigidBody.remove" => "Removes the rigid bodies from `objects` (names or ids; the selection without it).",
        "physics.cloth.add" => {
            "Adds cloth simulation to `objects` (names or ids; the selection without it) with its stiffness, damping and per-system `effectorWeights`. All multiplies each field weight; nonpositive products disable fields. Gravity is independent."
        }
        "physics.cloth.set" => {
            "Changes cloth settings and partial `effectorWeights` on `objects`. Weights default to 1 and range from -200 to 200; other systems are unaffected."
        }
        "physics.cloth.remove" => "Removes cloth from `objects` (names or ids; the selection without it).",
        "physics.softBody.add" => {
            "Adds a soft body to `objects` (names or ids; the selection without it) with its goal, edge springs, mass, friction and per-system `effectorWeights`. All multiplies each field weight; nonpositive products disable fields. Gravity is independent."
        }
        "physics.softBody.set" => {
            "Changes soft-body settings and partial `effectorWeights` on `objects`. Weights default to 1 and range from -200 to 200; other systems are unaffected."
        }
        "physics.softBody.remove" => "Removes the soft bodies from `objects` (names or ids; the selection without it).",
        "physics.collision.add" => {
            "Makes `objects` (names or ids; the selection without it) colliders for cloth, soft bodies, hair, and particles (`damping` for how much they bounce)."
        }
        "physics.collision.remove" => "Removes collision from `objects` (names or ids; the selection without it).",
        "physics.field.new" => "Adds an empty with a force field (wind, vortex, force, turbulence, …) at `location`.",
        "physics.field.add" => "Adds a force field to `objects` (names or ids; the selection without it).",
        "physics.field.set" => "Changes the force field settings of `objects` (names or ids; the selection without it).",
        "physics.field.remove" => "Removes the force fields from `objects` (names or ids; the selection without it).",
        "physics.constraint.add" => {
            "Adds a rigid body constraint (fixed, hinge, slider, spring, motor, …) between two objects. A motor independently drives translation and rotation on the constraint X axis."
        }
        "physics.constraint.set" => "Changes a rigid body constraint's settings.",
        "physics.constraint.remove" => "Removes rigid body constraints from `objects` (names or ids; the selection without it).",
        "physics.constraint.connect" => "Connects `objects` (names or ids; the selection without it) pairwise with rigid body constraints of `type`.",
        "physics.bake" => {
            "Bakes the physics simulation over the frame range; answers the cache state. Without anything simulated it fails. Nonfinite cloth/soft-body motion fails explicitly, discards the derived cache and stops automatic baking until Bake is requested again."
        }
        "physics.bakeStep" => {
            "Bakes `frames` more frames of the simulation and answers the cache state; divergent cloth/soft-body motion fails and stops automatic baking."
        }
        "physics.free" => "Frees the baked physics cache and answers its state.",
        "nodes.newGroup" => "Creates a node group (geometry, shader, compositor) for a modifier, material, world or as a library group.",
        "nodes.removeGroup" => "Deletes node group `group`.",
        "nodes.renameGroup" => "Renames node group `group`.",
        "nodes.addNode" => "Adds a node of `kind` (see `nodes.describe`) to `group` at `location` with input `values`.",
        "nodes.removeNodes" => "Removes nodes by index from `group`.",
        "nodes.moveNodes" => "Moves nodes of `group` to `location` or by `offset` in the node editor.",
        "nodes.setValue" => "Sets input or property `id` of node `node` in `group` to `value`.",
        "nodes.link" => "Links output socket `fromSocket` of node `from` to input socket `toSocket` of node `to` in `group`.",
        "nodes.unlink" => "Removes the link between two sockets in `group`.",
        "nodes.addInput" => "Adds an input socket of `type` to node group `group`'s interface.",
        "nodes.removeInput" => "Removes input `name` from node group `group`'s interface.",
        "nodes.addOutput" => "Adds an output socket of `type` to node group `group`'s interface.",
        "nodes.removeOutput" => "Removes output `name` from node group `group`'s interface.",
        "nodes.addZoneItem" => "Adds an item to a repeat or simulation zone node's state.",
        "nodes.removeZoneItem" => "Removes an item from a repeat or simulation zone node's state.",
        "nodes.describe" => "Lists the node kinds a tree takes (or one `kind`) with their inputs, outputs and properties, for `nodes.addNode`.",
        "compositor.new" => "Creates a compositing node group and uses it for renders when `use`.",
        "compositor.set" => "Turns compositing on or off and sets which group renders use.",
        "compositor.setImage" => "Sets the image an image node of the compositing group reads (workspace path or base64).",
        "compositor.pickMatte" => "Adds the name under `x`, `y` of the render to a Cryptomatte node's matte (or takes it away with `remove`).",
        "compositor.preview" => "Answers the compositing group's result (or a viewer node) as a picture of `size`.",
        _ => "",
    }
}

fn output(id: &str) -> Option<Value> {
    let schema = match id {
        "select.all" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "select.none" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "select.objects" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mode.toggle" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mode.set" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mesh.selectMode" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mesh.selectVertexMode" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mesh.selectEdgeMode" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mesh.selectFaceMode" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mesh.select" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "render.image" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"base64":{"type":"string"},"extension":{"type":"string"},"height":{"type":"integer"},"mime":{"type":"string"},"width":{"type":"integer"}}}"#
        }
        "file.export" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"base64":{"type":"string"},"extension":{"type":"string"},"format":{"type":"string"},"mime":{"type":"string"}}}"#
        }
        "view.rendered" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.renderedStep" => {
            r#"{"type":"object","additionalProperties":false,"properties":{"samples":{"type":"integer","required":true},"target":{"type":"integer","required":true},"fraction":{"type":"number","required":true},"done":{"type":"boolean","required":true},"generation":{"type":"integer","required":true},"width":{"type":"integer","required":true},"height":{"type":"integer","required":true}}}"#
        }
        "view.shading" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.wireframe" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.solid" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.material" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.front" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.right" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.top" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.camera" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.perspective" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.frameAll" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.frameSelected" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.grid" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.overlays" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.orbit" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.pan" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.zoom" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.set" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "view.resize" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "mesh.proportional" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"proportional":{"type":"object","additionalProperties":true,"properties":{"connected":{"type":"boolean"},"falloff":{"type":"string"},"radius":{"type":"number"}}}}}"#
        }
        "render.trace.start" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"done":{"type":"boolean"},"fraction":{"type":"number"},"height":{"type":"integer"},"samples":{"type":"integer"},"target":{"type":"integer"},"width":{"type":"integer"}}}"#
        }
        "render.trace.cancel" => r#"{"type":"object","additionalProperties":true,"properties":{"cancelled":{"type":"boolean"}}}"#,
        "uv.liveUnwrap" => {
            r#"{"type":"object","additionalProperties":false,"properties":{"on":{"type":"boolean"},"method":{"type":"string"},"iterations":{"type":"integer"}}}"#
        }
        "uv.layout" | "uv.preview" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"source":{"type":"string"},"liveUnwrap":{"type":"boolean"},"method":{"type":"string"},"iterations":{"type":"integer"},"faces":{"type":"array","items":{"type":"object","additionalProperties":true,"properties":{"face":{"type":"integer"},"selected":{"type":"boolean"},"uvs":{"type":"json"},"vertices":{"type":"json"}}}},"hasUvs":{"type":"boolean"},"image":{"type":"json"},"object":{"type":"integer"},"pinned":{"type":"array"},"seams":{"type":"array"},"selectedVertices":{"type":"array","items":{"type":"integer"}}}}"#
        }
        "frame.set" => r#"{"type":"object","additionalProperties":true,"properties":{"frame":{"type":"integer"}}}"#,
        "frame.next" => r#"{"type":"object","additionalProperties":true,"properties":{"frame":{"type":"integer"}}}"#,
        "frame.previous" => r#"{"type":"object","additionalProperties":true,"properties":{"frame":{"type":"integer"}}}"#,
        "frame.start" => r#"{"type":"object","additionalProperties":true,"properties":{"frame":{"type":"integer"}}}"#,
        "frame.end" => r#"{"type":"object","additionalProperties":true,"properties":{"frame":{"type":"integer"}}}"#,
        "mode.weightPaint" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"object":{"type":"integer"},"weightPaint":{"type":"boolean"}}}"#
        }
        "mode.sculpt" => r#"{"type":"object","additionalProperties":true,"properties":{"mode":{"type":"string"},"object":{"type":"integer"}}}"#,
        "presence.local" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"selection":{"type":"array","items":{"type":"string"}},"viewport":{"type":"object","additionalProperties":true,"properties":{}}}}"#
        }
        "presence.peers" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"height":{"type":"integer"},"peers":{"type":"array"},"width":{"type":"integer"}}}"#
        }
        "presence.place" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"height":{"type":"integer"},"peers":{"type":"array"},"width":{"type":"integer"}}}"#
        }
        "physics.bakeStep" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"frames":{"type":"integer"},"state":{"type":"string","enum":["none","baking","baked","outdated","failed"]},"total":{"type":"integer"},"error":{"type":"string","description":"Failure detail when state is failed; retry explicitly after correcting settings"}}}"#
        }
        "physics.free" => r#"{"type":"object","additionalProperties":true,"properties":{"state":{"type":"string"}}}"#,
        "nodes.describe" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"kinds":{"type":"array","items":{"type":"object","additionalProperties":true,"properties":{"category":{"type":"string"},"id":{"type":"string"},"inputs":{"type":"json"},"outputs":{"type":"json"},"settings":{"type":"json"},"title":{"type":"string"},"trees":{"type":"json"}}}}}}"#
        }
        "render.trace.step" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"done":{"type":"boolean"},"fraction":{"type":"number"},"height":{"type":"integer"},"samples":{"type":"integer"},"target":{"type":"integer"},"width":{"type":"integer"},"base64":{"type":"string"}},"description":"the render's progress, with the picture so far as base64 PNG when asked"}"#
        }
        "render.trace.image" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"base64":{"type":"string"},"mime":{"type":"string"},"extension":{"type":"string"},"width":{"type":"integer"},"height":{"type":"integer"}},"description":"the path-traced picture so far"}"#
        }
        "anim.nextKey" => r#"{"type":"object","additionalProperties":true,"properties":{"frame":{"type":"integer"}}}"#,
        "anim.previousKey" => r#"{"type":"object","additionalProperties":true,"properties":{"frame":{"type":"integer"}}}"#,
        "render.animation" => {
            r#"{"type":"object","additionalProperties":true,"properties":{},"description":"the rendered frames (see the description)"}"#
        }
        "mode.armature" => r#"{"type":"object","additionalProperties":true,"properties":{"mode":{"type":"string"},"object":{"type":"json"}}}"#,
        "mode.pose" => r#"{"type":"object","additionalProperties":true,"properties":{"mode":{"type":"string"},"object":{"type":"json"}}}"#,
        "mode.shape" => r#"{"type":"object","additionalProperties":true,"properties":{"mode":{"type":"string"},"object":{"type":"json"}}}"#,
        "mode.particleEdit" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"mode":{"type":"string"},"object":{"type":"json"},"system":{"type":"integer"}}}"#
        }
        "bone.select" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "curve.select" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "lattice.select" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "vertexGroup.select" => r#"{"type":"object","additionalProperties":true,"properties":{"changed":{"type":"boolean"}}}"#,
        "paint.group" => r#"{"type":"object","additionalProperties":true,"properties":{"group":{"type":"string"}}}"#,
        "paint.brush" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"mode":{"type":"string"},"weight":{"type":"number"},"radius":{"type":"number"},"strength":{"type":"number"},"falloff":{"type":"string"},"frontFacesOnly":{"type":"boolean"},"xMirror":{"type":"boolean"},"topologyMirror":{"type":"boolean"},"mirrorGroups":{"type":"boolean"}}}"#
        }
        "paint.tool" => r#"{"type":"object","additionalProperties":true,"properties":{"tool":{"type":"string"}}}"#,
        "paint.sample" => r#"{"type":"object","additionalProperties":true,"properties":{"weight":{"type":"number"}}}"#,
        "sculpt.brush" => r#"{"type":"object","additionalProperties":true,"properties":{"brush":{"type":"json"}}}"#,
        "particleEdit.brush" => r#"{"type":"object","additionalProperties":true,"properties":{"brush":{"type":"json"}}}"#,
        "sculpt.tool" => r#"{"type":"object","additionalProperties":true,"properties":{"tool":{"type":"json"}}}"#,
        "particles.stats" => r#"{"type":"object","additionalProperties":true,"properties":{},"description":"the selected particle system's counts"}"#,
        "text.caret" => r#"{"type":"object","additionalProperties":true,"properties":{"caret":{"type":"json"}}}"#,
        "physics.bake" => {
            r#"{"type":"object","additionalProperties":true,"properties":{"frames":{"type":"integer"},"state":{"type":"string","enum":["none","baking","baked","outdated","failed"]},"total":{"type":"integer"},"error":{"type":"string","description":"Failure detail when state is failed; retry explicitly after correcting settings"}}}"#
        }
        "compositor.preview" => {
            r#"{"type":"object","additionalProperties":true,"properties":{},"description":"the compositing group's preview picture"}"#
        }
        _ => return None,
    };
    serde_json::from_str(schema).ok()
}

/// Types for keys the schema leaves as any value (`json`), from how the command reads them.
fn type_key(id: &str, key: &str, node: &mut Value) {
    let typed = match key {
        "factor" => serde_json::json!({ "oneOf": [{ "type": "number" }, { "type": "array", "items": { "type": "number" } }] }),
        "group" if id == "compositor.set" => serde_json::json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }, { "type": "null" }] }),
        "group" => serde_json::json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }] }),
        _ => return,
    };
    let (description, required) = (node.get("description").cloned(), node.get("required").cloned());
    *node = typed;
    if let Some(description) = description {
        node["description"] = description;
    }
    if let Some(required) = required {
        node["required"] = required;
    }
}
