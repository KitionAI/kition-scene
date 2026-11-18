# Bevel and taper objects on curves, text boxes, text on a curve, and NURBS surfaces in the
# reference application, for tests/curves2_blender.rs. Each case writes the inputs the test builds
# again (splines with their evaluated handles, text settings, surface settings) and what the
# reference made of them: vertex and face counts and every vertex position.
# Run: blender -b --factory-startup --python curves2.py -- out.json
import json
import math
import os
import sys

import bpy

out_path = sys.argv[sys.argv.index("--") + 1]
here = os.path.dirname(os.path.abspath(__file__))
font_path = os.path.normpath(os.path.join(here, "..", "..", "fonts", "inter", "Inter-Variable.ttf"))
for obj in list(bpy.data.objects):
    bpy.data.objects.remove(obj)
font = bpy.data.fonts.load(font_path)


def r(value):
    return round(value, 4)


def evaluated(obj):
    graph = bpy.context.evaluated_depsgraph_get()
    ev = obj.evaluated_get(graph)
    data = ev.to_mesh()
    result = {
        "vertices": len(data.vertices),
        "faces": len(data.polygons),
        "positions": [[r(c) for c in v.co] for v in data.vertices],
    }
    ev.to_mesh_clear()
    return result


def link(name, data):
    obj = bpy.data.objects.new(name, data)
    bpy.context.collection.objects.link(obj)
    return obj


def poly(name, points, cyclic=False):
    data = bpy.data.curves.new(name, "CURVE")
    data.dimensions = "3D"
    spline = data.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for point, co in zip(spline.points, points):
        point.co = (co[0], co[1], co[2], 1.0)
    spline.use_cyclic_u = cyclic
    return link(name, data)


def bezier(name, points, cyclic=False, resolution=12):
    data = bpy.data.curves.new(name, "CURVE")
    data.dimensions = "3D"
    spline = data.splines.new("BEZIER")
    spline.bezier_points.add(len(points) - 1)
    for point, co in zip(spline.bezier_points, points):
        point.co = co
        point.handle_left_type = "AUTO"
        point.handle_right_type = "AUTO"
    spline.use_cyclic_u = cyclic
    spline.resolution_u = resolution
    return link(name, data)


def splines_of(obj):
    out = []
    for spline in obj.data.splines:
        if spline.type == "BEZIER":
            out.append({
                "type": "bezier",
                "cyclic": spline.use_cyclic_u,
                "resolution": spline.resolution_u,
                "points": [
                    {"co": [r(c) for c in p.co], "left": [r(c) for c in p.handle_left], "right": [r(c) for c in p.handle_right], "radius": r(p.radius)}
                    for p in spline.bezier_points
                ],
            })
        else:
            out.append({
                "type": "poly",
                "cyclic": spline.use_cyclic_u,
                "points": [{"co": [r(c) for c in p.co[:3]], "radius": r(p.radius)} for p in spline.points],
            })
    return out


cases = []

# Bevel objects: an S-shaped path swept by open and closed profiles, with and without caps.
path = bezier("Path", [(-2.0, 0.0, 0.0), (0.0, 1.0, 0.5), (2.0, 0.0, 0.0)], resolution=6)
profiles = {
    "open": poly("Open", [(0.0, 0.0, 0.0), (0.3, 0.0, 0.0), (0.0, 0.5, 0.0)]),
    "closed": poly("Closed", [(-0.2, -0.1, 0.0), (0.2, -0.1, 0.0), (0.0, 0.3, 0.0)], cyclic=True),
    "round": bezier("Round", [(-0.2, 0.0, 0.0), (0.0, 0.2, 0.0), (0.2, 0.0, 0.0), (0.0, -0.2, 0.0)], cyclic=True, resolution=4),
}
profiles["round"].scale = (1.0, 2.0, 1.0)
for name, profile in profiles.items():
    for caps in (False, True):
        path.data.bevel_mode = "OBJECT"
        path.data.bevel_object = profile
        path.data.use_fill_caps = caps
        cases.append({
            "kind": "bevel",
            "name": name,
            "caps": caps,
            "path": splines_of(path),
            "profile": splines_of(profile),
            "scale": [r(c) for c in profile.scale],
            "result": evaluated(path),
        })
path.data.bevel_object = None
path.data.bevel_mode = "ROUND"
path.data.use_fill_caps = False

# Taper objects: a round bevel whose thickness follows a poly and a Bézier taper, in each radius
# mode, on points with a radius of 1.5.
path.data.bevel_depth = 0.2
path.data.bevel_resolution = 2
for point in path.data.splines[0].bezier_points:
    point.radius = 1.5
tapers = {
    "poly": poly("TaperPoly", [(0.0, 0.5, 0.0), (1.4, 1.2, 0.0), (2.0, 0.2, 0.0)]),
    "bezier": bezier("TaperBezier", [(0.0, 0.2, 0.0), (1.0, 1.0, 0.0), (3.0, 0.4, 0.0)], resolution=8),
}
for name, taper in tapers.items():
    for mode in ("OVERRIDE", "MULTIPLY", "ADD"):
        path.data.taper_object = taper
        path.data.taper_radius_mode = mode
        cases.append({
            "kind": "taper",
            "name": name,
            "mode": mode,
            "path": splines_of(path),
            "taper": splines_of(taper),
            "bevel_depth": 0.2,
            "bevel_resolution": 2,
            "result": evaluated(path),
        })
path.data.taper_object = None

# Text boxes: wrapping, horizontal and vertical alignment, overflow.
text_data = bpy.data.curves.new("Text", "FONT")
text_data.font = font
# Fewer points per curved glyph segment keep the recorded positions small.
text_data.resolution_u = 4
# The outline alone: the reference's fill adds points of its own inside the letters.
text_data.fill_mode = "NONE"
text = link("Text", text_data)


def text_case(name, **settings):
    text_data.body = settings.get("body", "Cube sets type in a box")
    text_data.size = settings.get("size", 1.0)
    text_data.space_line = settings.get("line_spacing", 1.0)
    text_data.align_x = settings.get("align", "LEFT")
    text_data.align_y = settings.get("align_y", "TOP_BASELINE")
    text_data.overflow = settings.get("overflow", "NONE")
    box = text_data.text_boxes[0]
    box.width = settings.get("width", 0.0)
    box.height = settings.get("height", 0.0)
    box.x = settings.get("x", 0.0)
    box.y = settings.get("y", 0.0)
    text_data.follow_curve = settings.get("follow")
    cases.append({"kind": "text", "name": name, "settings": {k: v for k, v in settings.items() if k != "follow"}, "result": evaluated(text)})


text_case("no box")
text_case("wrapped", width=4.0)
for align in ("CENTER", "RIGHT", "JUSTIFY", "FLUSH"):
    text_case("wrapped " + align.lower(), width=4.0, align=align)
for align_y in ("TOP", "CENTER", "BOTTOM", "BOTTOM_BASELINE"):
    text_case("box " + align_y.lower(), width=4.0, height=2.5, align_y=align_y)
    text_case("no height " + align_y.lower(), align_y=align_y, body="Two\nlines")
text_case("size 2 top", width=7.0, height=3.0, size=2.0, align_y="TOP")
text_case("moved box", width=4.0, x=0.5, y=-1.0, line_spacing=1.3)
text_case("truncate", width=4.0, height=1.5, overflow="TRUNCATE")
text_case("scale", width=4.0, height=1.5, overflow="SCALE")

# Text on a curve: a Bézier circle and an arc, letters left, centred, and right.
circle = bezier("Circle", [(-2.0, 0.0, 0.0), (0.0, 2.0, 0.0), (2.0, 0.0, 0.0), (0.0, -2.0, 0.0)], cyclic=True)
arc = bezier("Arc", [(-3.0, 0.0, 0.0), (0.0, 2.0, 0.0), (3.0, 0.0, 0.0)], resolution=16)
for curve_name, curve in (("circle", circle), ("arc", arc)):
    for align in ("LEFT", "CENTER", "RIGHT"):
        text_case("on " + curve_name + " " + align.lower(), body="On a path", align=align, follow=curve)
        cases[-1]["curve"] = splines_of(curve)
text_case("on arc squeezed", body="Longer than the arc it is on", follow=arc)
cases[-1]["curve"] = splines_of(arc)
text_data.follow_curve = None

# NURBS surfaces: each primitive, and the patch with other orders, resolutions, and knots.
kinds = ["curve", "circle", "surface", "cylinder", "sphere", "torus"]
for kind in kinds:
    getattr(bpy.ops.surface, "primitive_nurbs_surface_%s_add" % kind)()
    obj = bpy.context.active_object
    variants = [("default", {})]
    if kind == "surface":
        variants += [
            ("order 3", {"order_u": 3, "order_v": 2}),
            ("resolution 6 and 2", {"resolution_u": 6, "resolution_v": 2}),
            ("endpoint", {"use_endpoint_u": True, "use_endpoint_v": True}),
            ("bezier", {"use_bezier_u": True, "use_endpoint_u": True, "order_u": 3}),
            ("cyclic", {"use_cyclic_u": True}),
        ]
    if kind == "cylinder":
        variants += [("resolution 8", {"resolution_u": 8, "resolution_v": 1})]
    for name, settings in variants:
        spline = obj.data.splines[0]
        for key, value in settings.items():
            setattr(spline, key, value)
        cases.append({
            "kind": "surface",
            "name": kind + " " + name,
            "primitive": kind,
            "spline": {
                "order_u": spline.order_u,
                "order_v": spline.order_v,
                "resolution_u": spline.resolution_u,
                "resolution_v": spline.resolution_v,
                "knots_u": [spline.use_cyclic_u, spline.use_endpoint_u, spline.use_bezier_u],
                "knots_v": [spline.use_cyclic_v, spline.use_endpoint_v, spline.use_bezier_v],
            },
            "result": evaluated(obj),
        })
    bpy.data.objects.remove(obj)

with open(out_path, "w") as handle:
    json.dump({"reference": bpy.app.version_string, "cases": cases}, handle)
