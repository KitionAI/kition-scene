"""Clean-room Motor observations; run Blender -b --factory-startup --python this.py -- out.json.
Only public application operations and evaluated transforms are used, never source code.
"""
import bpy
import json
import math
import sys


def body(location, mass, passive=False, rotation=(0, 0, 0)):
    bpy.ops.mesh.primitive_cube_add(size=1, location=location, rotation=rotation)
    obj = bpy.context.object
    bpy.ops.rigidbody.object_add(type='PASSIVE' if passive else 'ACTIVE')
    obj.rigid_body.mass = mass
    obj.rigid_body.linear_damping = 0
    obj.rigid_body.angular_damping = 0
    obj.rigid_body.collision_shape = 'BOX'
    return obj


def run(options):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.gravity = options.get('gravity', (0, 0, 0))
    scene.render.fps = 24
    bpy.ops.rigidbody.world_add()
    world = scene.rigidbody_world
    world.substeps_per_frame = options.get('substeps', 10)
    world.solver_iterations = options.get('iterations', 10)
    end = options.get("end", 25)
    world.point_cache.frame_end = end
    rotation = options.get('bodyRotation', (0, 0, 0))
    a = body((-2, 0, 0), options.get('massA', 1), not options.get('twoBodies', False), rotation)
    b = body((2, 0, 0), options.get('massB', 1), rotation=rotation)
    b.rigid_body.use_deactivation = options.get('deactivation', False)
    b.rigid_body.use_start_deactivated = options.get('startDeactivated', False)
    bpy.ops.object.empty_add(location=options.get('pivot', (0, 0, 0)), rotation=options.get('rotation', (0, 0, 0)))
    bpy.ops.rigidbody.constraint_add(type='MOTOR')
    c = bpy.context.object.rigid_body_constraint
    c.object1, c.object2 = a, b
    defaults = {name: getattr(c, name) for name in ('use_motor_lin', 'use_motor_ang', 'motor_lin_target_velocity', 'motor_ang_target_velocity', 'motor_lin_max_impulse', 'motor_ang_max_impulse', 'enabled', 'disable_collisions', 'use_breaking')}
    for key, value in options.get('settings', {}).items():
        setattr(c, key, value)
    frames = {}
    for frame in range(1, end + 1):
        scene.frame_set(frame)
        if frame in (1, 2, 3, 7, 13, 25, end):
            frames[str(frame)] = [{'position': list(o.matrix_world.translation), 'rotation': list(o.matrix_world.to_quaternion())} for o in (a, b)]
    return {'options': options, 'defaults': defaults, 'frames': frames}


cases = {}
def case(name, linear=False, angular=False, velocity=2, impulse=1, **options):
    settings = {'use_motor_lin': linear, 'use_motor_ang': angular, 'motor_lin_target_velocity': velocity, 'motor_ang_target_velocity': velocity,
                'motor_lin_max_impulse': impulse, 'motor_ang_max_impulse': impulse}
    settings.update(options.pop('settings', {}))
    cases[name] = run({'settings': settings, **options})

case('off', gravity=(0, 0, -9.81))
case('linear', linear=True)
case('linearFast', linear=True, impulse=1000)
case('angularFast', angular=True, impulse=1000)
case('linearNegative', linear=True, velocity=-2)
case('angular', angular=True)
case('angularNegative', angular=True, velocity=-2)
case('combined', linear=True, angular=True)
case('linearCapped', linear=True, impulse=0.001)
case('linearHeavy', linear=True, impulse=0.001, massB=2)
case('angularCapped', angular=True, impulse=0.001)
case('angularHeavy', angular=True, impulse=0.001, massB=2)
case('linearRotated', linear=True, rotation=(0, 0, math.pi/2))
case('angularRotated', angular=True, rotation=(0, 0, math.pi/2))
case('linearBodyRotated', linear=True, bodyRotation=(0, 0, math.pi/2))
case('linearOffset', linear=True, pivot=(0, 3, 0))
case('twoLinear', linear=True, twoBodies=True, massA=2, massB=3)
case('twoAngular', angular=True, twoBodies=True, massA=2, massB=3)
case('disabled', linear=True, angular=True, settings={'enabled': False})
case('zeroCap', linear=True, angular=True, impulse=0)
case('linearOneStep', linear=True, impulse=0.001, substeps=1)
case('angularOneStep', angular=True, impulse=0.001, substeps=1)
case('breakable', linear=True, settings={'use_breaking': True, 'breaking_threshold': 0.0001})
case('startAsleep', linear=True, angular=True, deactivation=True, startDeactivated=True)
case('slowSleep', linear=True, angular=True, velocity=0.1, deactivation=True, end=73)
case('breakZero', linear=True, angular=True, settings={'use_breaking': True, 'breaking_threshold': 0})
case('breakExact', linear=True, angular=True, settings={'use_breaking': True, 'breaking_threshold': 1/240})
case('breakSum', linear=True, angular=True, settings={'use_breaking': True, 'breaking_threshold': 0.006})
case('breakLinear', linear=True, settings={'use_breaking': True, 'breaking_threshold': 0.006})
with open(sys.argv[sys.argv.index('--')+1], 'w') as output:
    json.dump({'version': bpy.app.version_string, 'cases': cases}, output, indent=2)
    output.write('\n')
