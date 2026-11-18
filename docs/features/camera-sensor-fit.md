# Camera sensor fit

`camera.set` accepts `sensorFit: "auto" | "horizontal" | "vertical"` and
`sensorHeight` in millimetres (1 to 200). `sensor` remains the sensor width.
The Camera panel exposes the fit and the sensor dimension it uses. Auto is the
default, sensor width is 36 mm, and sensor height is 24 mm. These values are in
the camera's existing `Object.data` JSON register; KDM remains schema 6.
A scene saved by an earlier published engine opens with the Auto framing it was saved with.

For perspective cameras, Auto holds the sensor width along the longer render
dimension. Horizontal holds sensor width horizontally. Vertical holds sensor
height vertically. The unused sensor dimension does not affect the image. For
orthographic cameras the same fit chooses the dimension held by orthographic
scale; sensor dimensions and focal length do not affect that frame.

All rendering uses square pixels. Non-square pixel aspect, camera shift and
panoramic projections are outside this feature. Viewport size only changes how
the render frame fits inside the viewport; the scene's render aspect determines
the camera composition. Software, WebGL, final renders, rendered previews,
camera outlines and picking use the same fitted XY frame. This does not change
the renderers' existing clipping or depth-of-field behavior. Defocus receives
the fitted horizontal sensor gate when converting physical blur sizes to pixels.

One command changes the fit and dimensions as one undo step. Concurrent edits
to one camera resolve to one complete data register, keeping its lens, fit and
sensor dimensions together. Edits to separate cameras merge independently.

## Exchange boundaries

Camera framing equality across formats assumes the **same output aspect**.
Kition Scene has one scene render aspect; imported cameras do not change it. A file
with differently proportioned camera gates cannot have every native gate shown
simultaneously at its original aspect in a single scene.

| Boundary | What is retained |
| --- | --- |
| `.kscene` | Exact fit, both sensor dimensions, lens and orthographic scale. |
| `.blend` import | Fit and both sensor dimensions are read directly from the saved camera data. Scene pixel aspect is not imported. |
| glTF/GLB export | Perspective vertical field of view and aspect, or orthographic horizontal and vertical magnitudes, encode the fitted frame. |
| glTF/GLB import | Focal angle or orthographic magnitudes become an equivalent Auto camera at the authored aspect. Sensor identity and fit policy are not retained. |
| USD export | Both apertures encode the fitted frame in native USD units. USDA, USDC and USDZ share this camera writer. |
| USD import | Native horizontal aperture becomes a Horizontal camera; vertical aperture is retained as sensor height. Framing matches when the scene aspect matches the native gate. |
| FBX export/import | The fitted frame is baked into an Auto-sized film gate and orthographic scale. Fit policy and original sensor dimensions are not retained. |

USD orthographic apertures follow the [USD camera specification](https://openusd.org/release/api/class_usd_geom_camera.html): tenths of a scene unit.
The public USD camera API confirms the exported frame. Blender 5.2.3's USD
importer instead reads that orthographic aperture as scene units, enlarging the
frame by 10. Kition Scene preserves the native USD values; orthographic framing through
that Blender importer is therefore a known boundary. The small recording at
`crates/formats/usd/tests/fixtures/camera9-boundary.json` captures both public
readers; `tests/blender/camera9_boundary.py` in the USD crate reproduces it.

FBX's importer retains expanded finite film gates, including wide portrait and
landscape fits. Its existing focal-length limit is 1–5000 mm. Framing outside
that imported range is not lossless; use `.kscene` to retain the camera settings.
These formats encode a fitted frame rather than Kition Scene's authored fit policy.
Auto cameras imported through glTF or FBX can keep their Auto behavior, while
changing aspect after importing an explicit fit can differ from the original.

## Clean-room measurements

`crates/engine/model/tests/blender/camera9.py` uses Blender 5.2.3 LTS's public
`calc_matrix_camera` and `view_frame` APIs. Eighteen recorded cases cover all
three fits, perspective and orthographic projection, landscape, portrait and
square frames, a 47 mm lens, unequal 39 × 21 mm sensor dimensions, and asymmetric
points. Offline tests compare frame dimensions and projected coordinates within
1e-6; viewport tests also use rotated and translated cameras and mismatched
viewport rectangles. A traced asymmetric silhouette checks the actual camera
rays against those measured matrices.

The public file probe in `crates/formats/blend/tests/blender/camera9.py` records
the reference application's own exchange behavior and creates the committed
compressed `.blend`. Its FBX round trip resets fit to Auto; its glTF and FBX
round trips do not retain explicit orthographic fit. Kition Scene exports the fitted
frame directly instead of relying on those policies surviving. The optional
`KITION_BLENDER` checks re-observe camera matrices and read Kition Scene's exported
camera frames through the reference application's public importers. No
reference implementation source is used.

The USD portrait regression checks that a portrait export's gate follows Auto,
as the renderer does, rather than holding the width. The picking regression covers
near-plane f32 subtraction turning a translated portrait camera's ray; unprojection
and ray subtraction retain double precision until the final origin and direction.

A second public-API recording, `camera9-extreme-reference.json`, covers 1 × 8192
and 64 × 8192 images and their landscape counterparts, with a 1 mm lens and
200 mm sensors. Perspective matrices use reciprocal frame extents directly,
avoiding an ill-conditioned f32 angle round trip. glTF angles are written and
read in f64 before converting the recovered camera parameters to the model.
