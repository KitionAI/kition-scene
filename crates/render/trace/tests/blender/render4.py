# Path-traced reference renders for tests/render4_blender.rs: bladed and anamorphic depth of
# field, motion blur (objects, shape-key deformation, a moving camera), and volumes (a world
# volume and Volume Absorption, Volume Scatter, and Principled Volume in objects), made with the
# reference's path tracer (Cycles, CPU, 1024 samples, no denoising, Blur Glossy off, Standard
# view) at 160 x 90.
# The test builds the same scenes. Run:
#   blender -b --factory-startup --python render4.py -- <case> <out.png>
import math
import sys

import bpy

case, out = sys.argv[sys.argv.index("--") + 1 : sys.argv.index("--") + 3]

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
world = bpy.data.worlds.new("W")
scene.world = world
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.05, 0.05, 0.05, 1)


def principled(name, colour, roughness=0.5, transmission=0.0, ior=1.5):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = colour
    b.inputs["Roughness"].default_value = roughness
    b.inputs["Transmission Weight"].default_value = transmission
    b.inputs["IOR"].default_value = ior
    return m


def emitter(name, colour, strength):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.remove(nt.nodes["Principled BSDF"])
    e = nt.nodes.new("ShaderNodeEmission")
    e.inputs["Color"].default_value = colour
    e.inputs["Strength"].default_value = strength
    nt.links.new(e.outputs[0], nt.nodes["Material Output"].inputs["Surface"])
    return m


def volume_node(tree, kind, inputs):
    n = tree.nodes.new(kind)
    for key, value in inputs.items():
        n.inputs[key].default_value = value
    return n


def volume(name, kind, inputs, surface=None):
    """A material with a volume node in its Volume (and the Principled BSDF in its Surface when
    `surface` gives that BSDF's inputs; otherwise no surface)."""
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    b = nt.nodes["Principled BSDF"]
    if surface is None:
        nt.nodes.remove(b)
    else:
        for key, value in surface.items():
            b.inputs[key].default_value = value
    n = volume_node(nt, kind, inputs)
    nt.links.new(n.outputs[0], nt.nodes["Material Output"].inputs["Volume"])
    return m


def add(op, material=None, smooth=False, **kw):
    op(**kw)
    o = bpy.context.active_object
    if smooth:
        bpy.ops.object.shade_smooth()
    if material is not None:
        o.data.materials.append(material)
    return o


def camera(location, rotation):
    bpy.ops.object.camera_add(location=location, rotation=tuple(math.radians(a) for a in rotation))
    scene.camera = bpy.context.active_object
    return bpy.context.active_object


def key(o, path, frame, value):
    setattr(o, path, value)
    o.keyframe_insert(data_path=path, frame=frame)


if case != "bokeh":
    add(bpy.ops.mesh.primitive_plane_add, principled("Floor", (0.8, 0.8, 0.8, 1)), size=20)

if case == "bokeh":
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.01, 0.01, 0.01, 1)
    target = add(bpy.ops.mesh.primitive_cube_add, principled("Green", (0.1, 0.7, 0.2, 1)), size=1.0, location=(3, -3, 0.5))
    lights = [((-3, 4, 1.2), (1, 0.4, 0.2, 1)), ((2, 6, 2.0), (0.3, 0.5, 1, 1)), ((-5, -1, 1.5), (1, 1, 0.6, 1)), ((-1, 1, 0.6), (1, 0.3, 0.8, 1))]
    for i, (location, colour) in enumerate(lights):
        add(bpy.ops.mesh.primitive_uv_sphere_add, emitter("E%d" % i, colour, 40.0), smooth=True, radius=0.1, location=location)
    bpy.ops.object.light_add(type="SUN", rotation=(math.radians(50), 0, math.radians(20)))
    bpy.context.active_object.data.energy = 1.5
    cam = camera((7, -7, 3), (78, 0, 45))
    d = cam.data.dof
    d.use_dof = True
    d.focus_object = target
    d.aperture_fstop = 0.08
    d.aperture_blades = 6
    d.aperture_rotation = math.radians(15)
    d.aperture_ratio = 1.4
elif case.startswith("motion"):
    scene.frame_set(1)
    bpy.ops.object.light_add(type="SUN", rotation=(math.radians(40), 0, math.radians(30)))
    bpy.context.active_object.data.energy = 3
    slider = add(bpy.ops.mesh.primitive_cube_add, principled("Red", (0.8, 0.1, 0.1, 1)), size=1.0, location=(-3, -1, 0.5))
    key(slider, "location", 1, (-3, -1, 0.5))
    key(slider, "location", 11, (3, -1, 0.5))
    spinner = add(bpy.ops.mesh.primitive_cube_add, principled("Blue", (0.1, 0.2, 0.8, 1)), size=1.4, location=(0, 2.5, 0.7))
    key(spinner, "rotation_euler", 1, (0, 0, 0))
    key(spinner, "rotation_euler", 11, (0, 0, math.radians(180)))
    if case == "motion-deform":
        # A shape key raising the top of a cube, keyed from 0 to 1.
        morph = add(bpy.ops.mesh.primitive_cube_add, principled("Yellow", (0.8, 0.7, 0.1, 1)), size=1.0, location=(2.5, 1.5, 0.5))
        morph.shape_key_add(name="Basis")
        k = morph.shape_key_add(name="Up")
        for v in k.data:
            if v.co.z > 0:
                v.co.z += 1.5
        k.value = 0.0
        k.keyframe_insert(data_path="value", frame=1)
        k.value = 1.0
        k.keyframe_insert(data_path="value", frame=11)
        cam = camera((7, -7, 4.5), (66, 0, 45))
        key(cam, "location", 1, (6, -8, 4.5))
        key(cam, "location", 11, (8, -6, 4.5))
        scene.render.motion_blur_position = "START"
    else:
        camera((7, -7, 4.5), (66, 0, 45))
    scene.render.use_motion_blur = True
    scene.render.motion_blur_shutter = 1.0
    scene.frame_set(6)
elif case == "volume-world":
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.0, 0.0, 0.0, 1)
    fog = volume_node(world.node_tree, "ShaderNodeVolumeScatter", {"Color": (0.9, 0.9, 0.9, 1), "Density": 0.06, "Anisotropy": 0.3})
    world.node_tree.links.new(fog.outputs[0], world.node_tree.nodes["World Output"].inputs["Volume"])
    add(bpy.ops.mesh.primitive_cube_add, principled("White", (0.8, 0.8, 0.8, 1)), size=1.0, location=(0.4, 0.2, 2.5))
    add(bpy.ops.mesh.primitive_cube_add, principled("Red", (0.8, 0.1, 0.1, 1)), size=1.0, location=(-1.5, -1.0, 0.5))
    bpy.ops.object.light_add(type="SPOT", location=(0, 0, 6))
    spot = bpy.context.active_object.data
    spot.energy = 3000
    spot.spot_size = math.radians(50)
    spot.shadow_soft_size = 0.05
    camera((7, -7, 4.5), (66, 0, 45))
elif case == "volume-objects":
    scatter = volume("Scatter", "ShaderNodeVolumeScatter", {"Color": (0.9, 0.6, 0.3, 1), "Density": 1.5, "Anisotropy": 0.4})
    add(bpy.ops.mesh.primitive_cube_add, scatter, size=1.5, location=(-2, 0, 0.75))
    smoke = volume(
        "Smoke",
        "ShaderNodeVolumePrincipled",
        {"Color": (0.8, 0.8, 0.8, 1), "Density": 2.0, "Absorption Color": (0.2, 0.5, 0.8, 1), "Emission Strength": 0.5, "Emission Color": (1.0, 0.5, 0.1, 1)},
    )
    add(bpy.ops.mesh.primitive_uv_sphere_add, smoke, smooth=True, radius=0.9, location=(0.5, 1.5, 0.9))
    tinted = volume(
        "Tinted",
        "ShaderNodeVolumeAbsorption",
        {"Color": (0.2, 0.6, 0.9, 1), "Density": 1.5},
        surface={"Base Color": (1, 1, 1, 1), "Roughness": 0.0, "Transmission Weight": 1.0, "IOR": 1.45},
    )
    add(bpy.ops.mesh.primitive_uv_sphere_add, tinted, smooth=True, radius=0.8, location=(1.5, -1.5, 0.8))
    bpy.ops.object.light_add(type="AREA", location=(0, -1, 5))
    light = bpy.context.active_object.data
    light.energy = 600
    light.size = 2
    camera((7, -7, 4.5), (66, 0, 45))
else:
    raise SystemExit("unknown case " + case)

scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = 1024
scene.cycles.use_denoising = False
scene.cycles.use_adaptive_sampling = False
scene.cycles.seed = 0
# Blur Glossy off, as the tracer has no such filter (it spreads caustics into shadows).
scene.cycles.blur_glossy = 0.0
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = 160, 90, 100
scene.view_settings.view_transform = "Standard"
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGB"
scene.render.filepath = out
bpy.ops.render.render(write_still=True)
