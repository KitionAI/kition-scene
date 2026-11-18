# Clean-room observations through Blender's public Python API only.
# blender -b --factory-startup --python uv7.py -- cases.json reference.json
import bpy
import json
import sys

args = sys.argv[sys.argv.index('--') + 1:]
cases = json.load(open(args[0]))
out = {'version': bpy.app.version_string, 'cases': []}
for case in cases:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    mesh = bpy.data.meshes.new('Chart')
    mesh.from_pydata(case['positions'], [], case['faces'])
    mesh.update()
    obj = bpy.data.objects.new('Chart', mesh)
    bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    layer = mesh.uv_layers.new(name='UVMap')
    seams = {tuple(sorted(edge)) for edge in case.get('seams', [])}
    for edge in mesh.edges:
        edge.use_seam = tuple(sorted(edge.vertices)) in seams
    for polygon in mesh.polygons:
        for corner, loop in enumerate(polygon.loop_indices):
            vertex = mesh.loops[loop].vertex_index
            layer.data[loop].uv = case['uvs'][polygon.index][corner]
            layer.data[loop].pin_uv = vertex in case.get('pinned', [])
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.uv.unwrap(method='MINIMUM_STRETCH', iterations=case.get('iterations', 30), no_flip=True,
                      fill_holes=False, correct_aspect=False, margin=0.02)
    bpy.ops.object.mode_set(mode='OBJECT')
    layer = mesh.uv_layers.active
    out['cases'].append({'name': case['name'], 'uvs': [[list(layer.data[i].uv) for i in p.loop_indices] for p in mesh.polygons]})
json.dump(out, open(args[1], 'w'), indent=2)
