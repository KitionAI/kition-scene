"""Observe camera matrices near legal aspect and focal-length limits via the public API."""
import json
import sys
from pathlib import Path

import bpy

bpy.ops.wm.read_factory_settings(use_empty=True)
data = bpy.data.cameras.new("Extreme")
obj = bpy.data.objects.new("Extreme", data)
bpy.context.scene.collection.objects.link(obj)
data.lens = 1.0
data.sensor_width = 200.0
data.sensor_height = 200.0
data.clip_start = 0.1
data.clip_end = 100.0
cases = []
for fit in ["AUTO", "HORIZONTAL", "VERTICAL"]:
    data.sensor_fit = fit
    for width, height in [(1, 8192), (64, 8192), (8192, 1), (8192, 64)]:
        matrix = obj.calc_matrix_camera(bpy.context.evaluated_depsgraph_get(), x=width, y=height)
        cases.append({"fit": fit, "width": width, "height": height, "matrix": [list(row) for row in matrix]})
result = {"blender": bpy.app.version_string, "lens": data.lens, "sensorWidth": data.sensor_width,
          "sensorHeight": data.sensor_height, "cases": cases}
Path(sys.argv[sys.argv.index("--") + 1]).write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
