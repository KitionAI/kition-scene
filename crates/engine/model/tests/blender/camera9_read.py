"""Read an exported camera frame through the public application API."""
import json
import sys
from pathlib import Path

import bpy

path, width, height, output = sys.argv[sys.argv.index("--") + 1:]
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.resolution_x, scene.render.resolution_y = int(width), int(height)
if path.endswith(".glb"):
    bpy.ops.import_scene.gltf(filepath=path)
elif path.endswith(".fbx"):
    bpy.ops.import_scene.fbx(filepath=path)
else:
    bpy.ops.wm.usd_import(filepath=path)
result = {obj.name: [list(row) for row in obj.calc_matrix_camera(bpy.context.evaluated_depsgraph_get(), x=int(width), y=int(height))]
          for obj in scene.objects if obj.type == "CAMERA"}
if path.endswith((".usda", ".usdc")):
    # Read USD's public camera frustum independently of Blender's importer.
    from pxr import Usd, UsdGeom
    stage = Usd.Stage.Open(path)
    result["_usd"] = {prim.GetName(): [list(row) for row in UsdGeom.Camera(prim).GetCamera().frustum.ComputeProjectionMatrix()]
                      for prim in stage.Traverse() if prim.IsA(UsdGeom.Camera)}
Path(output).write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
