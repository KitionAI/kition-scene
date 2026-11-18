# Render passes of the reference application's path tracer (Cycles, CPU, no denoising, Blur
# Glossy off, Standard view) for tests/passes_blender.rs, at 96 x 54. Case "still": a floor, a red
# cube, a metal ball, a small glowing box, and a sun under a grey world, with every light pass,
# normal, AO, emission, environment, and Cryptomatte (object, material, asset); the cube is a
# child of an empty "Rig", so its asset is the rig. Case "moving" (and "moving-blur", with motion
# blur on): a cube sliding along x and a turning cube, with the vector pass. "still" also
# composites Cryptomatte nodes' mattes into its output, and "pick" (16 samples) their Pick.
# Each pass is written as the means of 8 x 9 pixel blocks (12 x 6 of them), and sampled at the
# blocks' centre pixels, little-endian float32 base64, with the Cryptomatte manifests.
# Run: blender -b --factory-startup --python passes.py -- <case> <out.json>
import base64
import json
import math
import os
import struct
import sys
import tempfile

import bpy
import OpenImageIO as oiio

case, out = sys.argv[sys.argv.index("--") + 1 : sys.argv.index("--") + 3]
W, H, BW, BH = 96, 54, 8, 9

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
world = bpy.data.worlds.new("W")
scene.world = world
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.05, 0.05, 0.05, 1)


def principled(name, colour, roughness=0.5, metallic=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = colour
    b.inputs["Roughness"].default_value = roughness
    b.inputs["Metallic"].default_value = metallic
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


def add(op, name, material=None, smooth=False, **kw):
    op(**kw)
    o = bpy.context.active_object
    o.name = name
    if smooth:
        bpy.ops.object.shade_smooth()
    if material is not None:
        o.data.materials.append(material)
    return o


def key(o, path, frame, value):
    setattr(o, path, value)
    o.keyframe_insert(data_path=path, frame=frame)


add(bpy.ops.mesh.primitive_plane_add, "Floor", principled("Floor", (0.8, 0.8, 0.8, 1)), size=20)
bpy.ops.object.light_add(type="SUN", rotation=(math.radians(40), 0, math.radians(30)))
bpy.context.active_object.data.energy = 3
bpy.ops.object.camera_add(location=(7, -7, 4.5), rotation=(math.radians(66), 0, math.radians(45)))
scene.camera = bpy.context.active_object
vl = scene.view_layers[0]
if case in ("still", "pick"):
    bpy.ops.object.empty_add(location=(0, 0, 0))
    rig = bpy.context.active_object
    rig.name = "Rig"
    cube = add(bpy.ops.mesh.primitive_cube_add, "Cube", principled("Red", (0.8, 0.1, 0.1, 1)), size=1.0, location=(-1, 0, 0.5))
    cube.parent = rig
    add(bpy.ops.mesh.primitive_uv_sphere_add, "Ball", principled("Metal", (0.9, 0.7, 0.4, 1), 0.25, 1.0), smooth=True, radius=0.6, location=(1, 0.5, 0.6))
    add(bpy.ops.mesh.primitive_cube_add, "Lamp", emitter("Glow", (1, 0.8, 0.4, 1), 5.0), size=0.4, location=(0.5, -1.5, 0.2))
    for name in ("diffuse_direct", "diffuse_indirect", "diffuse_color", "glossy_direct", "glossy_indirect", "glossy_color",
                 "emit", "environment", "ambient_occlusion", "normal", "z", "cryptomatte_object", "cryptomatte_material",
                 "cryptomatte_asset"):
        setattr(vl, "use_pass_" + name, True)
    vl.pass_cryptomatte_depth = 6
    scene.cycles.samples = 512 if case == "still" else 16
    # Compositing: the Cryptomatte node's mattes (red: the object Cube, green: the material
    # Metal, blue: the asset Rig) into the output; for "pick", the object layer's Pick.
    tree = bpy.data.node_groups.new("Mattes", "CompositorNodeTree")
    tree.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    layers = tree.nodes.new("CompositorNodeRLayers")
    combine = tree.nodes.new("CompositorNodeCombineColor")
    for socket, (layer, matte) in zip(combine.inputs, [("CryptoObject", "Cube"), ("CryptoMaterial", "Metal"), ("CryptoAsset", "Rig")]):
        node = tree.nodes.new("CompositorNodeCryptomatteV2")
        node.layer_name = "ViewLayer." + layer
        node.matte_id = matte
        tree.links.new(layers.outputs["Image"], node.inputs["Image"])
        tree.links.new(node.outputs["Matte"], socket)
        if layer == "CryptoObject":
            pick = node.outputs["Pick"]
    tree.links.new(combine.outputs[0] if case == "still" else pick, tree.nodes.new("NodeGroupOutput").inputs[0])
    scene.compositing_node_group = tree
else:
    scene.frame_set(1)
    slider = add(bpy.ops.mesh.primitive_cube_add, "Slider", principled("Red", (0.8, 0.1, 0.1, 1)), size=1.0, location=(-3, -1, 0.5))
    key(slider, "location", 1, (-3, -1, 0.5))
    key(slider, "location", 11, (3, -1, 0.5))
    spinner = add(bpy.ops.mesh.primitive_cube_add, "Spinner", principled("Blue", (0.1, 0.2, 0.8, 1)), size=1.4, location=(0, 2.5, 0.7))
    key(spinner, "rotation_euler", 1, (0, 0, 0))
    key(spinner, "rotation_euler", 11, (0, 0, math.radians(180)))
    vl.use_pass_vector = True
    vl.use_pass_z = True
    scene.render.use_motion_blur = case == "moving-blur"
    scene.render.motion_blur_shutter = 1.0
    scene.frame_set(6)
    scene.cycles.samples = 64

scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.use_denoising = False
scene.cycles.use_adaptive_sampling = False
scene.cycles.seed = 0
scene.cycles.blur_glossy = 0.0
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = W, H, 100
scene.view_settings.view_transform = "Standard"
scene.render.image_settings.media_type = "MULTI_LAYER_IMAGE"
scene.render.image_settings.file_format = "OPEN_EXR_MULTILAYER"
scene.render.image_settings.color_depth = "32"
path = os.path.join(tempfile.gettempdir(), "kition-scene-passes-%s.exr" % case)
scene.render.filepath = path
bpy.ops.render.render(write_still=True)

passes = {}
attributes = []
subimage = 0
while True:
    image = oiio.ImageBuf(path, subimage, 0)
    if image.has_error or image.nchannels == 0:
        break
    spec = image.spec()
    pixels = image.get_pixels(oiio.FLOAT)  # rows from the top, [y][x][channel]
    for index, name in enumerate(spec.channelnames):
        layer, _, channel = name.rpartition(".")
        passes.setdefault(layer, []).append((channel, [[float(pixels[y][x][index]) for x in range(W)] for y in range(H)]))
    attributes += [(attribute.name, attribute.value) for attribute in spec.extra_attribs]
    subimage += 1
def pack(values):
    return base64.b64encode(struct.pack("<%df" % len(values), *values)).decode()


result = {"blender": bpy.app.version_string, "width": W, "height": H, "block": [BW, BH], "passes": {}, "manifests": {}}
for layer, channels in sorted(passes.items()):
    means, centres = [], []
    for by in range(H // BH):
        for bx in range(W // BW):
            for _, plane in channels:
                total = 0.0
                for y in range(by * BH, by * BH + BH):
                    for x in range(bx * BW, bx * BW + BW):
                        total += plane[y][x]
                means.append(total / (BW * BH))
            cy, cx = by * BH + BH // 2, bx * BW + BW // 2
            centres += [plane[cy][cx] for _, plane in channels]
    result["passes"][layer] = {"channels": [c for c, _ in channels], "means": pack(means), "centres": pack(centres)}
for name, value in attributes:
    if name.endswith("manifest"):
        result["manifests"][name] = json.loads(value)
json.dump(result, open(out, "w"), indent=0, sort_keys=True)
