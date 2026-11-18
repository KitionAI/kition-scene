# Compositing nodes in the reference application, for tests/compositor_blender.rs. Two fixed
# pictures (A: a colour ramp with a hot spot and a one-pixel spike; B: a ramp with alpha,
# premultiplied) go through one node per case (fixtures/compositor-cases.json names the node,
# its settings, and its inputs) to the group's output; the output is read back from a 32-bit
# OpenEXR render. Pixels on a grid through the spike are written as little-endian float32,
# base64.
# Run: blender -b --factory-startup --python compositor.py -- cases.json out.json
import base64
import json
import os
import re
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


def make(name, f):
    image = bpy.data.images.new(name, W, H, alpha=True, float_buffer=True)
    pixels = []
    for row in range(H):
        y = H - 1 - row  # the reference stores the bottom row first
        for x in range(W):
            pixels += f(x, y)
    image.pixels = pixels
    return image


PICTURES = {"A": make("A", picture_a), "B": make("B", picture_b)}
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = W, H, 100
scene.view_settings.view_transform = "Standard"
scene.view_settings.look = "None"
scene.render.image_settings.file_format = "OPEN_EXR"
scene.render.image_settings.color_depth = "32"
scene.render.film_transparent = True

NODES = {
    "blur": "CompositorNodeBlur",
    "glare": "CompositorNodeGlare",
    "colorBalance": "CompositorNodeColorBalance",
    "rgbCurves": "CompositorNodeCurveRGB",
    "hueSaturation": "CompositorNodeHueSat",
    "brightContrast": "CompositorNodeBrightContrast",
    "exposure": "CompositorNodeExposure",
    "mixColor": "ShaderNodeMix",
    "alphaOver": "CompositorNodeAlphaOver",
    "ellipseMask": "CompositorNodeEllipseMask",
    "lensDistortion": "CompositorNodeLensdist",
    "invert": "CompositorNodeInvert",
}
CHOICES = {
    "gaussian": "Gaussian", "flat": "Flat", "tent": "Tent", "quadratic": "Quadratic", "cubic": "Cubic",
    "catrom": "Catrom", "mitch": "Mitch", "fastGaussian": "Fast Gaussian",
    "bloom": "Bloom", "fogGlow": "Fog Glow", "streaks": "Streaks", "simpleStar": "Simple Star",
    "ghosts": "Ghosts", "sunBeams": "Sun Beams", "low": "Low", "medium": "Medium", "high": "High",
    "liftGammaGain": "Lift/Gamma/Gain", "offsetPowerSlope": "Offset/Power/Slope (ASC-CDL)",
    "over": "Over", "disjointOver": "Disjoint Over", "conjointOver": "Conjoint Over",
    "add": "Add", "subtract": "Subtract", "multiply": "Multiply", "not": "Not",
    "radial": "Radial", "horizontal": "Horizontal",
}
MIX = {"Factor": "Factor_Float", "A": "A_Color", "B": "B_Color", "Result": "Result_Color"}


def socket(node, kind, ident, output=False):
    if kind == "mixColor":
        ident = MIX.get(ident, ident)
    for item in node.outputs if output else node.inputs:
        if item.identifier == ident:
            return item
    raise KeyError(ident)


def render(case):
    group = bpy.data.node_groups.new("Case", "CompositorNodeTree")
    group.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    output = group.nodes.new("NodeGroupOutput")
    node = group.nodes.new(NODES[case["kind"]])
    if case["kind"] == "mixColor":
        node.data_type = "RGBA"
    for key, value in case["values"].items():
        if key == "blendType":
            node.blend_type = re.sub(r"([A-Z])", r"_\1", value).upper()
        elif key in ("clampResult", "clampFactor"):
            setattr(node, "clamp_result" if key == "clampResult" else "clamp_factor", value)
        elif key == "curves":
            for channel, points in value.items():
                curve = node.mapping.curves[{"r": 0, "g": 1, "b": 2, "c": 3}[channel]]
                curve.points[0].location = points[0]
                curve.points[-1].location = points[-1]
                for point in points[1:-1]:
                    curve.points.new(point[0], point[1])
            node.mapping.update()
        else:
            item = socket(node, case["kind"], key)
            if isinstance(value, str):
                value = CHOICES[value]
            if isinstance(value, list):
                value = value[: len(item.default_value)]
            item.default_value = value
    for key, source in case["inputs"].items():
        name, _, out = source.partition(".")
        image = group.nodes.new("CompositorNodeImage")
        image.image = PICTURES[name]
        group.links.new(image.outputs[out or "Image"], socket(node, case["kind"], key))
    group.links.new(socket(node, case["kind"], case["output"], output=True), output.inputs[0])
    scene.compositing_node_group = group
    path = os.path.join(tempfile.gettempdir(), "kition-scene-compositor.exr")
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
