"""Record public Multires operators, without reading application implementation code.

blender -b --factory-startup --python multires8.py -- <output.json>
"""
import json
import math
import sys
import bpy


def mesh_data(mesh):
    return {"positions": [list(v.co) for v in mesh.vertices],
            "faces": [list(p.vertices) for p in mesh.polygons]}


def evaluated(obj):
    bpy.context.view_layer.update()
    value = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    mesh = value.to_mesh()
    result = mesh_data(mesh)
    value.to_mesh_clear()
    return result


result = {"version": bpy.app.version_string, "cases": []}
for primitive, depth, deform, operation, existing in [
    ("cube", 1, False, "unsubdivide", 0),
    ("cube", 2, True, "unsubdivide", 0),
    ("cube", 2, True, "rebuild", 0),
    ("plane", 2, True, "rebuild", 0),
    ("cube", 1, True, "unsubdivide", 1),
    ("cube", 0, False, "unsubdivide", 0),
    ("triangle", 0, False, "unsubdivide", 0),
]:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    if primitive == "cube":
        bpy.ops.mesh.primitive_cube_add(size=2)
    else:
        bpy.ops.mesh.primitive_plane_add(size=2)
    obj = bpy.context.object
    if primitive == "triangle":
        modifier = obj.modifiers.new("Triangles", "TRIANGULATE")
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    if depth:
        modifier = obj.modifiers.new("Input", "SUBSURF")
        modifier.levels = depth
        modifier.subdivision_type = "SIMPLE"
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    if deform:
        for v in obj.data.vertices:
            x, y, z = v.co
            v.co.z += 0.17 * math.sin(1.7*x + 0.4*y) + 0.08*x*y
            v.co.x += 0.04 * math.cos(y + z)
    source = mesh_data(obj.data)
    modifier = obj.modifiers.new("Multires", "MULTIRES")
    for _ in range(existing):
        bpy.ops.object.multires_subdivide(modifier=modifier.name)
    before = evaluated(obj)
    try:
        op = bpy.ops.object.multires_unsubdivide if operation == "unsubdivide" else bpy.ops.object.multires_rebuild_subdiv
        status = sorted(op(modifier=modifier.name))
    except RuntimeError:
        status = ["CANCELLED"]
    levels = []
    for level in range(modifier.total_levels + 1):
        modifier.levels = level
        levels.append(evaluated(obj))
    result["cases"].append({"name": f"{primitive}-{depth}-{operation}-{existing}",
                            "operation": operation, "existing": existing,
                            "input": source, "before": before, "status": status,
                            "base": mesh_data(obj.data), "levels": levels,
                            "total": modifier.total_levels,
                            "sculpt": modifier.sculpt_levels})
with open(sys.argv[sys.argv.index("--") + 1], "w") as handle:
    json.dump(result, handle, indent=2)
    handle.write("\n")
