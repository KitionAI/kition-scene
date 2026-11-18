"""Observe a native USD orthographic gate through two public camera APIs.

The USD specification measures apertures in tenths of a scene unit. This probe
records both the public USD frustum and Blender's imported orthographic camera.
"""
import json
import sys
import tempfile
from pathlib import Path

import bpy
from pxr import Usd, UsdGeom

output = Path(sys.argv[sys.argv.index("--") + 1])
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.resolution_x = 160
scene.render.resolution_y = 90
stage = Usd.Stage.CreateInMemory()
UsdGeom.SetStageMetersPerUnit(stage, 1)
UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
camera = UsdGeom.Camera.Define(stage, "/Camera")
camera.CreateProjectionAttr("orthographic")
camera.CreateHorizontalApertureAttr(70)
camera.CreateVerticalApertureAttr(39.375)
camera.CreateFocalLengthAttr(0.47)
frustum = camera.GetCamera().frustum
with tempfile.TemporaryDirectory(prefix="kition-camera9-usd-") as directory:
    path = str(Path(directory) / "camera.usda")
    stage.Export(path)
    bpy.ops.wm.usd_import(filepath=path)
    imported = next(obj for obj in scene.objects if obj.type == "CAMERA")
    result = {
        "blender": bpy.app.version_string,
        "usd": list(Usd.GetVersion()),
        "width": 160,
        "height": 90,
        "metersPerUnit": 1,
        "horizontalAperture": 70,
        "verticalAperture": 39.375,
        "usdWindow": [list(frustum.window.GetMin()), list(frustum.window.GetMax())],
        "usdMatrix": [list(row) for row in frustum.ComputeProjectionMatrix()],
        "blenderOrthoScale": imported.data.ortho_scale,
        "blenderFit": imported.data.sensor_fit,
        "blenderMatrix": [list(row) for row in imported.calc_matrix_camera(
            bpy.context.evaluated_depsgraph_get(), x=160, y=90)],
    }
output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
