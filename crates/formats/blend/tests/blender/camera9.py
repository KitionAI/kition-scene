"""Public-API camera file observations; no Blender implementation is read.

The output directory receives a compressed .blend and camera observations.
"""
import json
import sys
from pathlib import Path

import bpy

folder = Path(sys.argv[sys.argv.index("--") + 1])
folder.mkdir(parents=True, exist_ok=True)
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.resolution_x = 90
scene.render.resolution_y = 160
scene.render.filepath = "//"
for kind in ["PERSP", "ORTHO"]:
    for fit in ["AUTO", "HORIZONTAL", "VERTICAL"]:
        data = bpy.data.cameras.new(kind + "_" + fit)
        data.type = kind
        data.sensor_fit = fit
        data.sensor_width = 39
        data.sensor_height = 21
        data.lens = 47
        data.ortho_scale = 7
        obj = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(obj)
        scene.camera = obj
# Save only scene dependencies. Startup screens contain machine-local file-browser paths.
bpy.data.libraries.write(str(folder / "camera9.blend"), {scene}, path_remap="RELATIVE_ALL", compress=True)
observations = {}
for fmt in ["glb", "usda", "fbx"]:
    path = str(folder / ("camera9." + fmt))
    if fmt == "glb":
        bpy.ops.export_scene.gltf(filepath=path, export_cameras=True)
    elif fmt == "usda":
        bpy.ops.wm.usd_export(filepath=path, export_cameras=True)
    else:
        bpy.ops.export_scene.fbx(filepath=path, object_types={"CAMERA"}, bake_anim=False)
for fmt in ["glb", "usda", "fbx"]:
    for obj in list(scene.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    path = str(folder / ("camera9." + fmt))
    if fmt == "glb":
        bpy.ops.import_scene.gltf(filepath=path)
    elif fmt == "usda":
        bpy.ops.wm.usd_import(filepath=path)
    else:
        bpy.ops.import_scene.fbx(filepath=path)
    entries = []
    for obj in scene.objects:
        if obj.type != "CAMERA":
            continue
        data = obj.data
        entries.append({"name": obj.name, "kind": data.type, "fit": data.sensor_fit,
                        "lens": data.lens, "sensor": data.sensor_width,
                        "sensorHeight": data.sensor_height, "orthoScale": data.ortho_scale,
                        "matrix": [list(row) for row in obj.calc_matrix_camera(bpy.context.evaluated_depsgraph_get(), x=90, y=160)]})
    observations[fmt] = sorted(entries, key=lambda entry: entry["name"])
(folder / "camera9-formats.json").write_text(json.dumps(observations, indent=2) + "\n", encoding="utf-8")
