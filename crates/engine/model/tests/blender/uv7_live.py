# Observe a UV Editor transform with Live Unwrap off and on through public operators.
# blender -b --factory-startup --python uv7_live.py -- cases.json reference.json
import bpy
import bmesh
import json
import sys

args = sys.argv[sys.argv.index('--') + 1:]
case = next(c for c in json.load(open(args[0])) if c['name'] == 'pinned')
if len(args) > 2 and args[2] == 'single':
    case['pinned'] = [4]
out = {'version': bpy.app.version_string, 'cases': []}
for live in [False, True]:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    mesh = bpy.data.meshes.new('Chart')
    mesh.from_pydata(case['positions'], [], case['faces'])
    obj = bpy.data.objects.new('Chart', mesh)
    bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    mesh.uv_layers.new(name='UVMap')
    layer = mesh.uv_layers.active
    for polygon in mesh.polygons:
        for corner, loop in enumerate(polygon.loop_indices):
            vertex = mesh.loops[loop].vertex_index
            layer.data[loop].uv = case['uvs'][polygon.index][corner]
            layer.data[loop].pin_uv = vertex in case['pinned']
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.uv.unwrap(method='ANGLE_BASED', margin=0.02)
    bpy.context.scene.tool_settings.use_uv_select_sync = False
    bm = bmesh.from_edit_mesh(mesh)
    bm.verts.ensure_lookup_table()
    layer = bm.loops.layers.uv.active
    for face in bm.faces:
        for loop in face.loops:
            loop.uv_select_vert = loop.vert.index == 4
            loop.uv_select_edge = False
    bmesh.update_edit_mesh(mesh)
    area = bpy.context.window.screen.areas[0]
    area.type = 'IMAGE_EDITOR'
    area.ui_type = 'UV'
    area.spaces.active.uv_editor.use_live_unwrap = live
    region = next(r for r in area.regions if r.type == 'WINDOW')
    with bpy.context.temp_override(area=area, region=region):
        bpy.ops.transform.translate(value=(0.12, 0.07, 0.0))
    bpy.ops.object.mode_set(mode='OBJECT')
    layer = mesh.uv_layers.active
    out['cases'].append({'live': live, 'uvs': [[list(layer.data[i].uv) for i in p.loop_indices] for p in mesh.polygons]})
json.dump(out, open(args[1], 'w'), indent=2)
