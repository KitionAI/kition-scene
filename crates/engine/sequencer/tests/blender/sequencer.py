# The reference application's sequencer output for each case in fixtures/sequencer-cases.json.
# Run: <blender> -b --factory-startup --python sequencer.py -- <cases.json> <out.json>
# Writes { case name: base64 of the frame's straight RGBA8 pixels, rows from the top }.
import base64
import json
import math
import os
import struct
import sys
import tempfile
import zlib

import bpy

args = sys.argv[sys.argv.index("--") + 1:]
CASES, OUT = args[0], args[1]
spec = json.load(open(CASES))
work = tempfile.mkdtemp()


def picture(name, width, height):
    """The test pictures, the same formulas as the Rust test's: straight RGBA8, rows from the top."""
    rows = []
    for r in range(height):
        row = []
        for x in range(width):
            if name == "A":
                pixel = ((x * 255) // (width - 1), (r * 255) // (height - 1), ((x + r) * 7) % 256, 255)
            elif name == "B":
                pixel = ((x * 255) // (width - 1), 200 - r * 15, 60 + r * 10, min(255, x * 13 + r * 11))
            else:
                pixel = (30, (x * 5) % 256, 200 + r * 2, 255)
            row.extend(pixel)
        rows.append(bytes(row))
    return rows


def write_png(path, width, height, rows):
    raw = b"".join(b"\x00" + row for row in rows)
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    with open(path, "wb") as file:
        file.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def read_png(path):
    data = open(path, "rb").read()
    at, idat, width, height, depth, kind = 8, b"", 0, 0, 8, 6
    while at < len(data):
        length = struct.unpack(">I", data[at:at + 4])[0]
        name = data[at + 4:at + 8]
        body = data[at + 8:at + 8 + length]
        if name == b"IHDR":
            width, height, depth, kind = struct.unpack(">IIBB", body[:10])
        elif name == b"IDAT":
            idat += body
        at += 12 + length
    assert depth == 8 and kind == 6, (depth, kind)
    raw = zlib.decompress(idat)
    stride = width * 4
    rows, previous = [], bytearray(stride)
    for y in range(height):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            left = line[i - 4] if i >= 4 else 0
            up = previous[i]
            corner = previous[i - 4] if i >= 4 else 0
            if kind == 1:
                line[i] = (line[i] + left) & 255
            elif kind == 2:
                line[i] = (line[i] + up) & 255
            elif kind == 3:
                line[i] = (line[i] + (left + up) // 2) & 255
            elif kind == 4:
                p = left + up - corner
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - corner)
                line[i] = (line[i] + (left if pa <= pb and pa <= pc else up if pb <= pc else corner)) & 255
        rows.append(bytes(line))
        previous = line
    return width, height, b"".join(rows)


paths = {}
for name, (width, height) in spec["images"].items():
    paths[name] = os.path.join(work, name + ".png")
    write_png(paths[name], width, height, picture(name, width, height))

BLEND = {"colorBurn": "BURN", "colorDodge": "DODGE"}
SHAPES = {"single": "SINGLE", "double": "DOUBLE", "iris": "IRIS", "clock": "CLOCK"}


def blend_name(mode):
    if mode in BLEND:
        return BLEND[mode]
    return "".join("_" + c if c.isupper() else c.upper() for c in mode)


scene = bpy.context.scene
scene.view_settings.view_transform = "Standard"
scene.render.resolution_percentage = 100
scene.render.use_sequencer = True
scene.render.film_transparent = True
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"
scene.render.image_settings.color_depth = "8"
editor = scene.sequence_editor_create()
out = {}
for case in spec["cases"]:
    for strip in [s for s in editor.strips if s.type in ("CROSS", "GAMMA_CROSS", "WIPE")]:
        editor.strips.remove(strip)
    while len(editor.strips):
        editor.strips.remove(editor.strips[0])
    scene.render.resolution_x, scene.render.resolution_y = case["size"]
    made = {}
    for item in case["strips"]:
        kind = item["kind"]
        if kind == "image":
            strip = editor.strips.new_image(name=item["name"], filepath=paths[item["image"]], channel=item["channel"], frame_start=item["start"])
            strip.frame_final_duration = item["length"]
        elif kind in ("cross", "gammaCross", "wipe"):
            first, second = (made[name] for name in item["inputs"])
            strip = editor.strips.new_effect(name=item["name"], type={"cross": "CROSS", "gammaCross": "GAMMA_CROSS", "wipe": "WIPE"}[kind], channel=item["channel"], frame_start=first.left_handle, input1=first, input2=second)
            if kind == "wipe":
                strip.transition_type = SHAPES[item["shape"]]
                strip.direction = item["direction"].upper()
                strip.angle = math.radians(item["angle"])
                strip.blur_width = item.get("blur", 0.0)
        else:
            strip = editor.strips.new_effect(name=item["name"], type={"color": "COLOR", "text": "TEXT", "adjustment": "ADJUSTMENT"}[kind], channel=item["channel"], frame_start=item["start"], length=item["length"])
            if kind == "color":
                strip.color = item["color"]
            if kind == "text":
                strip.text = item["text"]
                strip.font_size = item["size"]
                strip.location = item.get("location", [0.5, 0.5])
                strip.anchor_x = "CENTER"
                strip.anchor_y = "CENTER"
                strip.alignment_x = "CENTER"
                strip.color = (1.0, 1.0, 1.0, 1.0)
        if "trimStart" in item:
            strip.left_handle = item["start"] + item["trimStart"]
            strip.right_handle = item["start"] + item["length"] - item["trimEnd"]
        if "blend" in item:
            strip.blend_type = blend_name(item["blend"])
        if "opacity" in item:
            strip.blend_alpha = item["opacity"]
        if "mute" in item:
            strip.mute = item["mute"]
        if "saturation" in item:
            strip.color_saturation = item["saturation"]
        if "multiply" in item:
            strip.color_multiply = item["multiply"]
        transform = item.get("transform", {})
        if "offset" in transform:
            strip.transform.offset_x, strip.transform.offset_y = transform["offset"]
        if "scale" in transform:
            strip.transform.scale_x, strip.transform.scale_y = transform["scale"]
        if "rotation" in transform:
            strip.transform.rotation = math.radians(transform["rotation"])
        if "origin" in transform:
            strip.transform.origin = transform["origin"]
        if "filter" in transform:
            strip.transform.filter = transform["filter"].upper()
        if transform.get("flipX"):
            strip.use_flip_x = True
        if transform.get("flipY"):
            strip.use_flip_y = True
        crop = item.get("crop", {})
        strip.crop.min_x = crop.get("left", 0)
        strip.crop.max_x = crop.get("right", 0)
        strip.crop.min_y = crop.get("bottom", 0)
        strip.crop.max_y = crop.get("top", 0)
        made[item["name"]] = strip
    scene.frame_set(case["frame"])
    path = os.path.join(work, "out.png")
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    width, height, pixels = read_png(path)
    assert [width, height] == case["size"], (case["name"], width, height)
    out[case["name"]] = base64.b64encode(pixels).decode()
json.dump(out, open(OUT, "w"), indent=0, sort_keys=True)
