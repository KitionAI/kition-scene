# Shrinkwrap, Lattice, Decimate (un-subdivide and planar), and Displace with an image in the
# reference application, for tests/modifiers3_blender.rs. Each case writes the input mesh
# (positions and faces), the settings, and what the modifier made: every vertex for the
# deforming modifiers, counts and the sorted positions for decimation.
# Run: blender -b --factory-startup --python modifiers3.py -- out.json
import json
import math
import os
import sys
import tempfile

import bpy

out_path = sys.argv[sys.argv.index("--") + 1]
for obj in list(bpy.data.objects):
    bpy.data.objects.remove(obj)


def r(value):
    return round(value, 5)


def mesh_of(obj):
    data = obj.data
    return {
        "positions": [[r(c) for c in v.co] for v in data.vertices],
        "faces": [list(p.vertices) for p in data.polygons],
    }


def evaluated(obj):
    graph = bpy.context.evaluated_depsgraph_get()
    ev = obj.evaluated_get(graph)
    data = ev.to_mesh()
    result = {
        "positions": [[r(c) for c in v.co] for v in data.vertices],
        "faces": [list(p.vertices) for p in data.polygons],
    }
    ev.to_mesh_clear()
    return result


def subdivide(obj, cuts):
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.subdivide(number_cuts=cuts)
    bpy.ops.object.mode_set(mode="OBJECT")


cases = []

# Shrinkwrap: a sphere round a cube, and a small one inside it.
bpy.ops.mesh.primitive_cube_add(size=2)
target = bpy.context.active_object
target.name = "Target"
for radius, method, offset, negative in [
    (1.5, "NEAREST_SURFACEPOINT", 0.0, False),
    (1.5, "NEAREST_SURFACEPOINT", 0.1, False),
    (0.5, "NEAREST_SURFACEPOINT", 0.0, False),
    (1.5, "PROJECT", 0.0, False),
    (1.5, "PROJECT", 0.0, True),
    (0.5, "PROJECT", 0.05, False),
    (1.5, "NEAREST_VERTEX", 0.0, False),
]:
    bpy.ops.mesh.primitive_uv_sphere_add(segments=16, ring_count=8, radius=radius)
    sphere = bpy.context.active_object
    modifier = sphere.modifiers.new("Shrinkwrap", "SHRINKWRAP")
    modifier.target = target
    modifier.wrap_method = method
    modifier.offset = offset
    modifier.use_negative_direction = negative
    cases.append({
        "kind": "shrinkwrap",
        "method": method,
        "offset": offset,
        "negative": negative,
        "mesh": mesh_of(sphere),
        "target": mesh_of(target),
        "result": evaluated(sphere),
    })
    bpy.data.objects.remove(sphere)
bpy.data.objects.remove(target)

# Lattice: a cube cut into a 5 × 5 × 5 grid of points, bent by lattices.
bpy.ops.mesh.primitive_cube_add(size=2)
cube = bpy.context.active_object
subdivide(cube, 3)
for resolution, interpolation, strength, scale, location, moves in [
    (2, "KEY_BSPLINE", 1.0, 2.0, (0, 0, 0), [(7, (0.25, 0, 0.1)), (0, (0, 0.1, 0))]),
    (2, "KEY_LINEAR", 1.0, 2.0, (0, 0, 0), [(7, (0.25, 0, 0.1)), (0, (0, 0.1, 0))]),
    (3, "KEY_BSPLINE", 1.0, 2.0, (0, 0, 0), [(26, (0.4, 0, 0)), (13, (0, 0, 0.3)), (4, (0.1, -0.2, 0))]),
    (3, "KEY_LINEAR", 1.0, 2.0, (0, 0, 0), [(26, (0.4, 0, 0)), (13, (0, 0, 0.3)), (4, (0.1, -0.2, 0))]),
    (3, "KEY_BSPLINE", 0.5, 1.5, (0.3, 0, 0.2), [(26, (0.4, 0, 0)), (13, (0, 0, 0.3))]),
]:
    # Added as the Add menu does, so its points rest from -0.5 to 0.5 at any resolution.
    bpy.ops.object.add(type="LATTICE")
    lattice = bpy.context.active_object
    data = lattice.data
    data.points_u = data.points_v = data.points_w = resolution
    data.interpolation_type_u = data.interpolation_type_v = data.interpolation_type_w = interpolation
    lattice.scale = (scale, scale, scale)
    lattice.location = location
    for index, offset in moves:
        point = data.points[index]
        point.co_deform = [point.co[axis] + offset[axis] for axis in range(3)]
    modifier = cube.modifiers.new("Lattice", "LATTICE")
    modifier.object = lattice
    modifier.strength = strength
    cases.append({
        "kind": "lattice",
        "resolution": resolution,
        "interpolation": "linear" if interpolation == "KEY_LINEAR" else "bspline",
        "strength": strength,
        "scale": scale,
        "location": list(location),
        "moves": [[index, list(offset)] for index, offset in moves],
        "mesh": mesh_of(cube),
        "result": evaluated(cube),
    })
    cube.modifiers.remove(modifier)
    bpy.data.objects.remove(lattice)
bpy.data.objects.remove(cube)


def decimation(name, obj, settings):
    modifier = obj.modifiers.new("Decimate", "DECIMATE")
    for key, value in settings.items():
        setattr(modifier, key, value)
    result = evaluated(obj)
    obj.modifiers.remove(modifier)
    sizes = {}
    for face in result["faces"]:
        sizes[str(len(face))] = sizes.get(str(len(face)), 0) + 1
    return {
        "kind": "decimate",
        "name": name,
        "settings": {key: (r(value) if isinstance(value, float) else value) for key, value in settings.items()},
        "mesh": mesh_of(obj),
        "vertices": len(result["positions"]),
        "faces": len(result["faces"]),
        "sizes": sizes,
        "positions": sorted(result["positions"]),
    }


bpy.ops.mesh.primitive_grid_add(x_subdivisions=9, y_subdivisions=9, size=2)
grid = bpy.context.active_object
cases.append(decimation("grid", grid, {"decimate_type": "UNSUBDIV", "iterations": 1}))
cases.append(decimation("grid", grid, {"decimate_type": "DISSOLVE", "angle_limit": math.radians(5)}))
bpy.data.objects.remove(grid)
bpy.ops.mesh.primitive_cube_add(size=2)
cube = bpy.context.active_object
subdivide(cube, 1)
for iterations in (1, 2):
    cases.append(decimation("cube subdivided once", cube, {"decimate_type": "UNSUBDIV", "iterations": iterations}))
subdivide(cube, 1)
for iterations in (1, 2, 3, 4):
    cases.append(decimation("cube subdivided twice", cube, {"decimate_type": "UNSUBDIV", "iterations": iterations}))
cases.append(decimation("cube subdivided twice", cube, {"decimate_type": "DISSOLVE", "angle_limit": math.radians(5)}))
bpy.data.objects.remove(cube)
bpy.ops.mesh.primitive_uv_sphere_add(segments=32, ring_count=16, radius=1)
sphere = bpy.context.active_object
cases.append(decimation("sphere", sphere, {"decimate_type": "UNSUBDIV", "iterations": 1}))
for degrees in (5, 15, 30):
    cases.append(decimation("sphere", sphere, {"decimate_type": "DISSOLVE", "angle_limit": math.radians(degrees)}))
bpy.data.objects.remove(sphere)
bpy.ops.mesh.primitive_cylinder_add(vertices=32, radius=1, depth=2)
cylinder = bpy.context.active_object
subdivide(cylinder, 1)
cases.append(decimation("cylinder", cylinder, {"decimate_type": "DISSOLVE", "angle_limit": math.radians(5)}))
bpy.data.objects.remove(cylinder)

# Displace by images: ramps across and up, and flat colours.
folder = tempfile.mkdtemp()


def image(name, width, height, pixel):
    made = bpy.data.images.new(name, width, height)
    values = []
    for y in range(height):
        for x in range(width):
            values += pixel(x, y)
    made.pixels = values
    path = os.path.join(folder, name + ".png")
    made.filepath_raw = path
    made.file_format = "PNG"
    made.save()
    loaded = bpy.data.images.load(path)
    # The bytes as stored, top row first.
    stored = list(loaded.pixels)
    rows = []
    for y in reversed(range(height)):
        row = []
        for x in range(width):
            at = (y * width + x) * 4
            row.append([round(stored[at + channel] * 255) for channel in range(3)])
        rows.append(row)
    return loaded, rows


images = [
    image("across", 8, 1, lambda x, y: [x / 7, x / 7, x / 7, 1]),
    image("up", 1, 8, lambda x, y: [y / 7, y / 7, y / 7, 1]),
    image("colours", 4, 4, lambda x, y: [x / 3, y / 3, 0.5, 1]),
]
bpy.ops.mesh.primitive_plane_add(size=2, location=(0.5, 0.25, 0))
plane = bpy.context.active_object
subdivide(plane, 7)
for loaded, rows in images:
    for coordinates in ("LOCAL", "UV", "GLOBAL"):
        texture = bpy.data.textures.new(loaded.name, "IMAGE")
        texture.image = loaded
        modifier = plane.modifiers.new("Displace", "DISPLACE")
        modifier.texture = texture
        modifier.texture_coords = coordinates
        modifier.strength = 0.5
        mesh = mesh_of(plane)
        mesh["uvs"] = [[[r(c) for c in plane.data.uv_layers.active.data[loop].uv] for loop in p.loop_indices] for p in plane.data.polygons]
        cases.append({
            "kind": "displace",
            "image": rows,
            "coordinates": coordinates.lower(),
            "strength": 0.5,
            "location": list(plane.location),
            "mesh": mesh,
            "result": evaluated(plane),
        })
        plane.modifiers.remove(modifier)

with open(out_path, "w") as handle:
    json.dump({"cases": cases}, handle, separators=(",", ":"))
