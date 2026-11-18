# Transparent shadows in the reference's fast renderer (EEVEE 5.2.3), for
# tests/shadows_alpha_blender.rs: a sun straight along +Y and down at 45 degrees over a white
# floor, a black world, and four casters 2 m up (1 m planes and 1 m cubes with blended alpha 0.5
# and 0.25) and a 1 m plane whose image's alpha is an 8 x 8 checker of 0 and 1. Seen from above
# through an orthographic camera, the floor in each shadow is read back from a 32-bit OpenEXR
# render and written as its share of the open floor's light.
# Run: blender -b --factory-startup --python shadows_alpha.py -- out.json
import json
import math
import sys

import bpy

out = sys.argv[sys.argv.index("--") + 1]
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
world = bpy.data.worlds.new("W")
scene.world = world
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0, 0, 0, 1)


def material(name, alpha, image=None):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = (0.8, 0.8, 0.8, 1)
    b.inputs["Alpha"].default_value = alpha
    if image is not None:
        t = m.node_tree.nodes.new("ShaderNodeTexImage")
        t.image = image
        t.interpolation = "Closest"
        m.node_tree.links.new(t.outputs["Alpha"], b.inputs["Alpha"])
    return m


checker = bpy.data.images.new("checker", 8, 8, alpha=True)
checker.pixels = [v for y in range(8) for x in range(8) for v in (1, 1, 1, float((x + y) % 2))]

bpy.ops.mesh.primitive_plane_add(size=40)
bpy.context.active_object.data.materials.append(material("Floor", 1.0))
# Name, kind, alpha, x.
casters = [("plane50", "plane", 0.5, -6.0), ("plane25", "plane", 0.25, -3.0), ("cube50", "cube", 0.5, 0.0), ("cube25", "cube", 0.25, 3.0), ("checker", "plane", 1.0, 6.0)]
for name, kind, alpha, x in casters:
    if kind == "plane":
        bpy.ops.mesh.primitive_plane_add(size=1, location=(x, -2, 2))
    else:
        bpy.ops.mesh.primitive_cube_add(size=1, location=(x, -2, 2))
    bpy.context.active_object.data.materials.append(material(name, alpha, checker if name == "checker" else None))
bpy.ops.object.light_add(type="SUN", rotation=(math.radians(45), 0, 0))
sun = bpy.context.active_object.data
sun.energy = 3
sun.angle = 0.0
bpy.ops.object.camera_add(location=(0, 0, 30))
scene.camera = bpy.context.active_object
scene.camera.data.type = "ORTHO"
scene.camera.data.ortho_scale = 16
scene.render.engine = "BLENDER_EEVEE"
scene.render.resolution_x, scene.render.resolution_y = 320, 160
scene.view_settings.view_transform = "Standard"
scene.render.image_settings.file_format = "OPEN_EXR"
scene.render.image_settings.color_depth = "32"
path = out + ".exr"
scene.render.filepath = path
bpy.ops.render.render(write_still=True)
image = bpy.data.images.load(path)
pixels = list(image.pixels)
W, H = 320, 160


def at(x, y):
    """The floor's red at world (x, y): rows run up from the bottom of the picture."""
    column = int((x + 8) / 16 * W)
    row = int((y + 4) / 8 * H)
    return pixels[(row * W + column) * 4]


def mean(x0, y0, size, steps=16):
    values = [at(x0 + (i + 0.5) / steps * size - size / 2, y0 + (j + 0.5) / steps * size - size / 2) for i in range(steps) for j in range(steps)]
    return sum(values) / len(values)


lit = mean(0, 2.5, 1.0)
# A shadow of something 2 m up (its middle; a cube's top and bottom span 1.5 to 2.5 m) falls
# 2 m along +Y; read the middle 0.5 m of each shadow (the checker's whole 0.9 m).
shares = {name: mean(x, 0.0, 0.9 if name == "checker" else 0.5) / lit for name, _, _, x in casters}
json.dump({"lit": lit, "shares": shares}, open(out, "w"), indent=1)
