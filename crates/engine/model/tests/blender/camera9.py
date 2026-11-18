"""Record camera framing using only Blender's public Python API.

Run Blender --background --factory-startup --python camera9.py -- output.json.
The matrix projects asymmetric camera-local points, with square pixels unless named.
"""
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
data = bpy.data.cameras.new("Reference")
camera = bpy.data.objects.new("Reference", data)
scene.collection.objects.link(camera)
scene.camera = camera
data.lens = 47.0
data.sensor_width = 39.0
data.sensor_height = 21.0
data.ortho_scale = 7.0
data.clip_start = 0.2
data.clip_end = 80.0
points = [[-0.7, 1.3, -4.0], [1.4, -0.6, -3.0], [0.2, 0.9, -7.0]]
cases = []
for kind in ["PERSP", "ORTHO"]:
    data.type = kind
    for fit in ["AUTO", "HORIZONTAL", "VERTICAL"]:
        data.sensor_fit = fit
        for width, height in [(160, 90), (90, 160), (120, 120)]:
            scene.render.resolution_x = width
            scene.render.resolution_y = height
            scene.render.pixel_aspect_x = scene.render.pixel_aspect_y = 1.0
            matrix = camera.calc_matrix_camera(bpy.context.evaluated_depsgraph_get(), x=width, y=height)
            projected = []
            for point in points:
                clip = matrix @ Vector([*point, 1.0])
                projected.append([(clip.x / clip.w + 1.0) / 2.0, (clip.y / clip.w + 1.0) / 2.0])
            cases.append({"kind": kind, "fit": fit, "width": width, "height": height,
                          "frame": [list(v) for v in data.view_frame(scene=scene)],
                          "matrix": [list(row) for row in matrix], "projected": projected})
result = {"blender": bpy.app.version_string, "lens": data.lens,
          "sensorWidth": data.sensor_width, "sensorHeight": data.sensor_height,
          "orthoScale": data.ortho_scale, "clip": [data.clip_start, data.clip_end],
          "points": points, "cases": cases}
Path(sys.argv[sys.argv.index("--") + 1]).write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
