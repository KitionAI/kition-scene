"""The physics comparison scenes in the reference application: run with `-b --factory-startup
--python reference.py -- <drop|stack|domino|cloth> <out.json>`; writes per-frame results."""

import bpy, json, sys, math
from mathutils import Vector

def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.frame_start = 1
    scene.render.fps = 24
    return scene

def world(scene, end):
    if scene.rigidbody_world is None:
        bpy.ops.rigidbody.world_add()
    rbw = scene.rigidbody_world
    rbw.point_cache.frame_start = 1
    rbw.point_cache.frame_end = end
    scene.frame_end = end

def body(obj, kind, **settings):
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.rigidbody.object_add(type=kind)
    for key, value in settings.items():
        setattr(obj.rigid_body, key, value)

def run(scene, end, record):
    out = []
    for frame in range(1, end + 1):
        scene.frame_set(frame)
        out.append(record())
    return out

def drop():
    scene = reset()
    bpy.ops.mesh.primitive_plane_add(size=20)
    ground = bpy.context.object
    world(scene, 100)
    body(ground, 'PASSIVE')
    bpy.ops.mesh.primitive_cube_add(size=2, location=(0, 0, 4))
    cube = bpy.context.object
    body(cube, 'ACTIVE')
    return run(scene, 100, lambda: list(cube.matrix_world.translation))

def stack():
    scene = reset()
    bpy.ops.mesh.primitive_plane_add(size=20)
    ground = bpy.context.object
    world(scene, 120)
    body(ground, 'PASSIVE')
    boxes = []
    for level in range(5):
        bpy.ops.mesh.primitive_cube_add(size=1, location=(0, 0, 0.5 + level * 1.01))
        boxes.append(bpy.context.object)
        body(boxes[-1], 'ACTIVE')
    return run(scene, 120, lambda: [list(box.matrix_world.translation) for box in boxes])

def domino():
    scene = reset()
    bpy.ops.mesh.primitive_plane_add(size=40)
    ground = bpy.context.object
    world(scene, 120)
    body(ground, 'PASSIVE')
    pieces = []
    for index in range(8):
        bpy.ops.mesh.primitive_cube_add(size=1, location=(index * 1.0, 0, 1.0), rotation=(0, math.radians(15) if index == 0 else 0, 0), scale=(0.2, 1, 2))
        pieces.append(bpy.context.object)
        body(pieces[-1], 'ACTIVE', friction=0.5)
    def record():
        return [[math.degrees(a) for a in piece.matrix_world.to_euler('XYZ')] + list(piece.matrix_world.translation) for piece in pieces]
    return run(scene, 120, record)

def cloth():
    scene = reset()
    scene.frame_end = 60
    bpy.ops.mesh.primitive_uv_sphere_add(segments=32, ring_count=16, radius=1)
    sphere = bpy.context.object
    bpy.ops.object.modifier_add(type='COLLISION')
    bpy.ops.mesh.primitive_grid_add(x_subdivisions=20, y_subdivisions=20, size=3, location=(0, 0, 1.5))
    sheet = bpy.context.object
    bpy.ops.object.modifier_add(type='CLOTH')
    sheet.modifiers['Cloth'].point_cache.frame_end = 60
    start = [list(sheet.matrix_world @ v.co) for v in sheet.data.vertices]
    def record():
        depsgraph = bpy.context.evaluated_depsgraph_get()
        evaluated = sheet.evaluated_get(depsgraph)
        return [list(evaluated.matrix_world @ v.co) for v in evaluated.data.vertices]
    frames = run(scene, 60, record)
    return {'start': start, 'end': frames[-1], 'mid': frames[29]}

argv = sys.argv[sys.argv.index('--') + 1:]
which, path = argv[0], argv[1]
result = {'drop': drop, 'stack': stack, 'domino': domino, 'cloth': cloth}[which]()
with open(path, 'w') as handle:
    json.dump(result, handle)
