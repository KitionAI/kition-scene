"""Rigid body constraints, deactivation, and force fields on rigid bodies and cloth in the
reference application, for tests/constraints.rs. Run: blender -b --factory-startup --python
constraints.py -- out.json; writes every number the test compares."""

import bpy, json, sys, math


def reset(end=100):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.frame_start, scene.frame_end, scene.render.fps = 1, end, 24
    bpy.ops.rigidbody.world_add()
    scene.rigidbody_world.point_cache.frame_start = 1
    scene.rigidbody_world.point_cache.frame_end = end
    return scene


def cube(size, location, kind='ACTIVE', **settings):
    bpy.ops.mesh.primitive_cube_add(size=size, location=location)
    obj = bpy.context.object
    bpy.ops.rigidbody.object_add(type=kind)
    for key, value in settings.items():
        setattr(obj.rigid_body, key, value)
    return obj


def ground():
    bpy.ops.mesh.primitive_plane_add(size=20)
    bpy.ops.rigidbody.object_add(type='PASSIVE')


def joint(kind, location, rotation, first, second, **settings):
    bpy.ops.object.empty_add(location=location, rotation=rotation)
    bpy.ops.rigidbody.constraint_add(type=kind)
    constraint = bpy.context.object.rigid_body_constraint
    constraint.object1, constraint.object2 = first, second
    for key, value in settings.items():
        setattr(constraint, key, value)


def at(objects, frames):
    """World positions of `objects` at each of `frames`."""
    scene, out = bpy.context.scene, {}
    for frame in range(1, max(frames) + 1):
        scene.frame_set(frame)
        if frame in frames:
            out[str(frame)] = [list(obj.matrix_world.translation) for obj in objects]
    return out


def pendulum(kind, **settings):
    reset()
    anchor = cube(0.2, (0, 0, 0), 'PASSIVE')
    bob = cube(0.5, (2, 0, 0))
    joint(kind, (0, 0, 0), (math.pi / 2, 0, 0), anchor, bob, **settings)
    return at([bob], [10, 20, 30, 40, 60, 80, 100])


def slider(**settings):
    reset(60)
    anchor = cube(0.2, (0, 0, 0), 'PASSIVE')
    bob = cube(0.5, (0, 0, -1))
    joint('SLIDER', (0, 0, 0), (0, math.radians(45), 0), anchor, bob, **settings)
    return at([bob], [30, 60])


def spring():
    reset()
    anchor = cube(0.2, (0, 0, 0), 'PASSIVE')
    bob = cube(0.5, (0, 0, -1))
    locks = {f'use_limit_{kind}_{axis}': True for kind in ('lin', 'ang') for axis in 'xyz' if (kind, axis) != ('lin', 'z')}
    ends = {f'limit_{kind}_{axis}_{end}': 0.0 for kind in ('lin', 'ang') for axis in 'xyz' for end in ('lower', 'upper') if (kind, axis) != ('lin', 'z')}
    joint('GENERIC_SPRING', (0, 0, 0), (0, 0, 0), anchor, bob, use_spring_z=True, spring_stiffness_z=10, spring_damping_z=0.5, **locks, **ends)
    return at([bob], [25, 50, 100])


def fixed_pair():
    reset()
    ground()
    first = cube(1, (0, 0, 3))
    second = cube(1, (1.2, 0, 3.5))
    joint('FIXED', (0.6, 0, 3.2), (0, 0, 0), first, second)
    return at([first, second], [15, 100])


def breaking(threshold):
    reset(40)
    anchor = cube(0.2, (0, 0, 0), 'PASSIVE')
    bob = cube(0.5, (2, 0, 0))
    joint('FIXED', (0, 0, 0), (0, 0, 0), anchor, bob, use_breaking=True, breaking_threshold=threshold)
    return at([bob], [40])


def asleep(woken):
    reset(80)
    ground()
    sleeper = cube(1, (0, 0, 3), use_deactivation=True, use_start_deactivated=True)
    if woken:
        cube(1, (0.2, 0, 6))
    return at([sleeper], [60, 80])


def pushed(kind, mass=1.0, flow=None):
    scene = reset(48)
    scene.rigidbody_world.effector_weights.gravity = 0
    body = cube(1, (0, 0, 0), mass=mass)
    if kind == 'FORCE':
        bpy.ops.object.effector_add(type='FORCE', location=(3, 0, 0))
    else:
        bpy.ops.object.effector_add(type='WIND', location=(-3, 0, 0), rotation=(0, math.pi / 2, 0))
    if flow is not None:
        bpy.context.object.field.flow = flow
    return at([body], [47])


def cloth(kind, facing=False, mass=0.3):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.frame_start, scene.frame_end, scene.render.fps = 1, 60, 24
    bpy.ops.mesh.primitive_grid_add(x_subdivisions=10, y_subdivisions=10, size=1, rotation=(0, math.pi / 2 if facing else 0, 0))
    sheet = bpy.context.object
    modifier = sheet.modifiers.new('Cloth', 'CLOTH')
    modifier.settings.effector_weights.gravity = 0
    modifier.settings.mass = mass
    modifier.point_cache.frame_end = 60
    if kind == 'FORCE':
        bpy.ops.object.effector_add(type='FORCE', location=(3, 0, 0))
    else:
        bpy.ops.object.effector_add(type='WIND', location=(-3, 0, 0), rotation=(0, math.pi / 2, 0))
    out = {}
    for frame in range(1, 49):
        scene.frame_set(frame)
        if frame in (24, 48):
            evaluated = sheet.evaluated_get(bpy.context.evaluated_depsgraph_get())
            points = [evaluated.matrix_world @ vertex.co for vertex in evaluated.data.vertices]
            out[str(frame)] = sum(point.x for point in points) / len(points)
    return out


result = {
    'hinge': pendulum('HINGE'),
    'point': pendulum('POINT'),
    'hingeLimit': pendulum('HINGE', use_limit_ang_z=True, limit_ang_z_lower=-math.radians(30), limit_ang_z_upper=math.radians(30)),
    'slider': slider(),
    'sliderLimit': slider(use_limit_lin_x=True, limit_lin_x_lower=-1, limit_lin_x_upper=1),
    'spring': spring(),
    'fixed': fixed_pair(),
    'weak': breaking(0.05),
    'strong': breaking(0.2),
    'asleep': asleep(False),
    'woken': asleep(True),
    'force': pushed('FORCE'),
    'forceHeavy': pushed('FORCE', mass=2.0),
    'wind': pushed('WIND'),
    'windStill': pushed('WIND', flow=0.0),
    'clothForce': cloth('FORCE'),
    'clothForceHeavy': cloth('FORCE', mass=1.0),
    'clothWindFacing': cloth('WIND', facing=True),
    'clothWindFlat': cloth('WIND'),
}
with open(sys.argv[sys.argv.index('--') + 1], 'w') as handle:
    json.dump(result, handle)
