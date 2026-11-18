# Writes the vertex positions of the reference application's Multiresolution modifier, levels 1
# to 3, on its default cube, plane, and UV sphere (no sculpting): what the levels show before any
# detail is added. Run: blender -b --factory-startup --python multires.py -- <output.json>
import json
import sys

import bpy

out_path = sys.argv[sys.argv.index("--") + 1]
for name in list(bpy.data.objects.keys()):
    bpy.data.objects.remove(bpy.data.objects[name])

bpy.ops.mesh.primitive_cube_add(size=2)
cube = bpy.context.active_object
bpy.ops.mesh.primitive_plane_add(size=2)
plane = bpy.context.active_object
bpy.ops.mesh.primitive_uv_sphere_add(segments=32, ring_count=16, radius=1)
sphere = bpy.context.active_object

result = {}
for name, obj in (("cube", cube), ("plane", plane), ("sphere", sphere)):
    for other in bpy.context.selected_objects:
        other.select_set(False)
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    obj.modifiers.new("Multires", "MULTIRES")
    levels = {}
    for level in range(1, 4):
        bpy.ops.object.multires_subdivide(modifier="Multires", mode="CATMULL_CLARK")
        graph = bpy.context.evaluated_depsgraph_get()
        evaluated = obj.evaluated_get(graph)
        mesh = evaluated.to_mesh()
        levels[str(level)] = [list(vertex.co) for vertex in mesh.vertices]
        evaluated.to_mesh_clear()
    result[name] = levels

with open(out_path, "w") as handle:
    json.dump(result, handle)
