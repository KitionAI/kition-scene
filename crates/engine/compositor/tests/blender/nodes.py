# Compositing nodes of round 5 in the reference application, for tests/compositor_nodes_blender.rs.
# Fixed pictures (A and B as in compositor.py; G: a green screen with a soft-edged subject and
# spill; Z: a depth ramp with a near square; M: a bright disc on a dark ramp, V: its speed, D: its
# depth; K: a 9 x 9 kernel) go through one node per case (fixtures/nodes-cases.json names the
# node, its settings, and its inputs) to the group's output; the output is read back from a
# 32-bit OpenEXR render. Pixels on the spec's grid are written as little-endian float32, base64.
# Run: blender -b --factory-startup --python nodes.py -- cases.json out.json
import base64
import json
import math
import os
import struct
import sys
import tempfile

import bpy

cases_path, out_path = sys.argv[sys.argv.index("--") + 1 : sys.argv.index("--") + 3]
spec = json.load(open(cases_path))
W, H = spec["width"], spec["height"]


def picture_a(x, y):
    u, v = (x + 0.5) / W, (y + 0.5) / H
    color = [1.2 * u, v, 0.25 + 0.5 * u * v, 1.0]
    if (x - 30) ** 2 + (y - 12) ** 2 < 9:
        color = [6.0, 5.0, 4.0, 1.0]
    if (x, y) == (10, 20):
        color = [12.0, 12.0, 12.0, 1.0]
    return color


def picture_b(x, y):
    u, v = (x + 0.5) / W, (y + 0.5) / H
    a = min(1.0, max(0.0, 1.5 * v - 0.25))
    return [0.9 * (1 - u) * a, (0.3 + 0.4 * v) * a, 0.8 * u * a, a]


def picture_g(x, y):
    u, v = (x + 0.5) / W, (y + 0.5) / H
    back = [0.1 + 0.05 * u, 0.75 - 0.1 * v, 0.12]
    color = back
    d = math.hypot(x - 18, y - 16)
    if d < 9:
        t = min(1.0, (9 - d) / 2)
        subject = [0.85, 0.45, 0.35]
        color = [b + (s - b) * t for b, s in zip(back, subject)]
    if 32 <= x <= 42 and 6 <= y <= 26:
        color = [0.4 * (0.6 + 0.4 * v), 0.6, 0.35 * (0.6 + 0.4 * v)]
    return color + [1.0]


def picture_z(x, y):
    u = (x + 0.5) / W
    z = 0.6 if 10 <= x <= 18 and 8 <= y <= 16 else 1.0 + 9.0 * u
    return [z, z, z, 1.0]


def in_disc(x, y):
    return (x - 24) ** 2 + (y - 16) ** 2 < 49


def picture_m(x, y):
    u, v = (x + 0.5) / W, (y + 0.5) / H
    return [1.0, 0.6, 0.2, 1.0] if in_disc(x, y) else [0.1 * u, 0.05 + 0.1 * v, 0.1, 1.0]


def picture_v(x, y):
    return [-4.0, 1.0, 4.0, -1.0] if in_disc(x, y) else [0.0, 0.0, 0.0, 0.0]


def picture_d(x, y):
    z = 2.0 if in_disc(x, y) else 10.0
    return [z, z, z, 1.0]


def picture_n(x, y):
    # Integer hash noise: no two regions alike (Kuwahara's quadrants never tie).
    return [((x * 73856093) ^ (y * 19349663) ^ (c * 83492791)) % 1000 / 1000.0 for c in range(3)] + [1.0]


def picture_s(x, y):
    # A binary screen: green, with a red square and one red pixel.
    if (20 <= x <= 27 and 12 <= y <= 19) or (x, y) == (8, 8):
        return [1.0, 0.0, 0.0, 1.0]
    return [0.0, 1.0, 0.0, 1.0]


def picture_q(x, y):
    return [0.5 * x / 47.0, y / 31.0, 0.0, 1.0]


def picture_p(x, y):
    return [x / 47.0, 0.6, y / 31.0, 1.0]


KW = KH = 9


def picture_k(x, y):
    if (x, y) == (4, 4):
        return [1.0, 1.0, 1.0, 1.0]
    if x == 4 or y == 4:
        f = 0.5 ** abs(x - 4 + y - 4)
        return [f, 0.5 * f, 0.25 * f, 1.0]
    return [0.0, 0.0, 0.0, 1.0]


def make(name, f, width=W, height=H):
    image = bpy.data.images.new(name, width, height, alpha=True, float_buffer=True)
    pixels = []
    for row in range(height):
        y = height - 1 - row  # the reference stores the bottom row first
        for x in range(width):
            pixels += f(x, y)
    image.pixels = pixels
    return image


PICTURES = {
    "A": make("A", picture_a),
    "B": make("B", picture_b),
    "G": make("G", picture_g),
    "Z": make("Z", picture_z),
    "M": make("M", picture_m),
    "V": make("V", picture_v),
    "D": make("D", picture_d),
    "P": make("P", picture_p),
    "Q": make("Q", picture_q),
    "S": make("S", picture_s),
    "N": make("N", picture_n),
    "K": make("K", picture_k, KW, KH),
}
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = W, H, 100
scene.view_settings.view_transform = "Standard"
scene.view_settings.look = "None"
scene.render.image_settings.file_format = "OPEN_EXR"
scene.render.image_settings.color_depth = "32"
scene.render.film_transparent = True
# The camera Defocus reads with its Z buffer.
camera_data = bpy.data.cameras.new("Camera")
camera_data.lens, camera_data.sensor_width = 50.0, 36.0
camera_data.dof.focus_distance = 4.0
camera = bpy.data.objects.new("Camera", camera_data)
scene.collection.objects.link(camera)
scene.camera = camera

NODES = {
    "translate": "CompositorNodeTranslate",
    "rotate": "CompositorNodeRotate",
    "scale": "CompositorNodeScale",
    "flip": "CompositorNodeFlip",
    "crop": "CompositorNodeCrop",
    "imageTransform": "CompositorNodeTransform",
    "kuwahara": "CompositorNodeKuwahara",
    "keying": "CompositorNodeKeying",
    "chromaKey": "CompositorNodeChromaMatte",
    "colorKey": "CompositorNodeColorMatte",
    "differenceKey": "CompositorNodeDiffMatte",
    "distanceKey": "CompositorNodeDistanceMatte",
    "luminanceKey": "CompositorNodeLumaMatte",
    "colorBalance": "CompositorNodeColorBalance",
    "glare": "CompositorNodeGlare",
    "cryptomatte": "CompositorNodeCryptomatteV2",
    "blur": "CompositorNodeBlur",
    "defocus": "CompositorNodeDefocus",
    "vectorBlur": "CompositorNodeVecBlur",
}
CHOICES = {
    "nearest": "Nearest", "bilinear": "Bilinear", "bicubic": "Bicubic", "anisotropic": "Anisotropic",
    "clip": "Clip", "extend": "Extend", "repeat": "Repeat",
    "relative": "Relative", "absolute": "Absolute", "sceneSize": "Scene Size", "renderSize": "Render Size",
    "stretch": "Stretch", "fit": "Fit", "crop": "Crop",
    "classic": "Classic",
    "smooth": "Smooth", "sphere": "Sphere", "root": "Root", "inverseSquare": "Inverse Square", "sharp": "Sharp", "linear": "Linear",
    "rgb": "RGB", "ycc": "YCC",
    "whitePoint": "White Point", "liftGammaGain": "Lift/Gamma/Gain",
    "kernel": "Kernel", "float": "Float", "color": "Color", "high": "High", "medium": "Medium", "low": "Low",
    "gaussian": "Gaussian", "flat": "Flat", "tent": "Tent", "fogGlow": "Fog Glow", "bloom": "Bloom",
}
# Settings the reference keeps on the node rather than in sockets: our id, its attribute.
PROPS = {
    "defocus": {"Bokeh": "bokeh", "Angle": "angle", "F-Stop": "f_stop", "Max Blur": "blur_max", "Use Z-Buffer": "use_zbuffer", "Z Scale": "z_scale"},
}


def socket(node, ident, output=False):
    for item in node.outputs if output else node.inputs:
        if item.identifier == ident:
            return item
    raise KeyError(ident)


def render(case):
    group = bpy.data.node_groups.new("Case", "CompositorNodeTree")
    group.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    output = group.nodes.new("NodeGroupOutput")
    node = group.nodes.new(NODES[case["kind"]])
    props = PROPS.get(case["kind"], {})
    for key, value in case["values"].items():
        if key in props:
            if isinstance(value, str):
                value = value.upper()
            setattr(node, props[key], value)
            continue
        item = socket(node, key)
        if isinstance(value, str):
            value = CHOICES[value]
        if isinstance(value, list):
            value = value[: len(item.default_value)]
        item.default_value = value
    for key, source in case["inputs"].items():
        name, _, out = source.partition(".")
        image = group.nodes.new("CompositorNodeImage")
        image.image = PICTURES[name]
        group.links.new(image.outputs[out or "Image"], socket(node, key))
    group.links.new(socket(node, case["output"], output=True), output.inputs[0])
    scene.compositing_node_group = group
    path = os.path.join(tempfile.gettempdir(), "kition-scene-compositor-nodes.exr")
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    result = bpy.data.images.load(path, check_existing=False)
    pixels = list(result.pixels)
    bpy.data.images.remove(result)
    bpy.data.node_groups.remove(group)
    out = []
    for y in spec["rows"]:
        row = H - 1 - y
        for x in spec["columns"]:
            out += pixels[(row * W + x) * 4 : (row * W + x) * 4 + 4]
    return base64.b64encode(struct.pack("<%df" % len(out), *out)).decode()


results = {case["name"]: render(case) for case in spec["cases"]}
json.dump({"blender": bpy.app.version_string, "results": results}, open(out_path, "w"), indent=0, sort_keys=True)
