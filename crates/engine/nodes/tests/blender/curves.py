# Curve nodes in the reference application, for tests/curves_blender.rs. Each node group (in
# Cube's JSON) is rebuilt as a geometry node tree on a cube, evaluated, and what comes out is
# written down: the curves (points, handles, radius, tilt, curve settings, named attributes), the
# mesh (vertices and faces), and the points (positions, radius, named attributes).
# Run: blender -b --factory-startup --python curves.py -- groups.json out.json
import json
import sys

import bpy

args = sys.argv[sys.argv.index("--") + 1:]
groups = json.load(open(args[0]))

KINDS = {
    "groupInput": "NodeGroupInput", "groupOutput": "NodeGroupOutput",
    "meshGrid": "GeometryNodeMeshGrid", "meshLine": "GeometryNodeMeshLine", "meshCircle": "GeometryNodeMeshCircle",
    "transform": "GeometryNodeTransform", "joinGeometry": "GeometryNodeJoinGeometry", "setPosition": "GeometryNodeSetPosition",
    "instanceOnPoints": "GeometryNodeInstanceOnPoints", "realizeInstances": "GeometryNodeRealizeInstances",
    "storeNamedAttribute": "GeometryNodeStoreNamedAttribute", "position": "GeometryNodeInputPosition",
    "index": "GeometryNodeInputIndex", "separateXyz": "ShaderNodeSeparateXYZ", "compare": "FunctionNodeCompare",
    "math": "ShaderNodeMath",
    "curveLine": "GeometryNodeCurvePrimitiveLine", "curveCircle": "GeometryNodeCurvePrimitiveCircle",
    "curveBezierSegment": "GeometryNodeCurvePrimitiveBezierSegment", "curveQuadrilateral": "GeometryNodeCurvePrimitiveQuadrilateral",
    "curveSpiral": "GeometryNodeCurveSpiral", "curveStar": "GeometryNodeCurveStar", "curveToMesh": "GeometryNodeCurveToMesh",
    "meshToCurve": "GeometryNodeMeshToCurve", "curveToPoints": "GeometryNodeCurveToPoints", "fillCurve": "GeometryNodeFillCurve",
    "resampleCurve": "GeometryNodeResampleCurve", "trimCurve": "GeometryNodeTrimCurve", "reverseCurve": "GeometryNodeReverseCurve",
    "setCurveRadius": "GeometryNodeSetCurveRadius", "setCurveTilt": "GeometryNodeSetCurveTilt",
    "setSplineCyclic": "GeometryNodeSetSplineCyclic", "setSplineResolution": "GeometryNodeSetSplineResolution",
    "splineParameter": "GeometryNodeSplineParameter", "curveLength": "GeometryNodeCurveLength",
    "endpointSelection": "GeometryNodeCurveEndpointSelection",
}
PROPS = {"mode": "mode", "domain": "domain", "operation": "operation", "dataType": "data_type", "keepLastSegment": "keep_last_segment"}
# Settings the reference keeps as menu sockets: the socket and its items.
MENUS = {
    ("fillCurve", "mode"): ("Mode", {"triangles": "Triangles", "ngons": "N-gons"}),
    ("fillCurve", "fillRule"): ("Fill Rule", {"even_odd": "Even-Odd", "non_zero": "Non-Zero"}),
    ("resampleCurve", "mode"): ("Mode", {"count": "Count", "length": "Length", "evaluated": "Evaluated"}),
}
ALIASES = {"A_INT": "A", "B_INT": "B"}
BUILTIN = {"position", "radius", "tilt", "handle_left", "handle_right", "handle_type_left", "handle_type_right", "curve_type",
           "cyclic", "resolution", "normal_mode", "knots_mode", "nurbs_order", "nurbs_weight"}


def socket(sockets, key):
    for s in sockets:
        if s.identifier == key and s.enabled:
            return s
    key = ALIASES.get(key, key)
    for s in sockets:
        if (s.identifier == key or s.name == key) and s.enabled:
            return s
    raise KeyError(key)


def r(value):
    return round(value, 5)


def values(attribute):
    out = []
    for item in attribute.data:
        if hasattr(item, "vector"):
            out.append([r(c) for c in item.vector])
        elif isinstance(item.value, float):
            out.append(r(item.value))
        else:
            out.append(item.value)
    return out


def named(attributes):
    return {a.name: {"domain": a.domain.lower(), "values": values(a)} for a in attributes if a.name not in BUILTIN and not a.name.startswith(".")}


def record(ob):
    graph = bpy.context.evaluated_depsgraph_get()
    geometry = ob.evaluated_get(graph).evaluated_geometry()
    out = {}
    curves = geometry.curves
    if curves is not None and len(curves.curves):
        attributes = {a.name: a for a in curves.attributes}
        get = lambda name: values(attributes[name]) if name in attributes else None
        out["curves"] = {
            "offsets": [c.first_point_index for c in curves.curves] + [len(curves.points)],
            "types": get("curve_type"), "cyclic": get("cyclic"), "resolution": get("resolution"),
            "positions": get("position"), "radius": get("radius"), "tilt": get("tilt"),
            "handle_left": get("handle_left"), "handle_right": get("handle_right"),
            "handle_type_left": get("handle_type_left"), "handle_type_right": get("handle_type_right"),
            "attributes": named(curves.attributes),
        }
    mesh = geometry.mesh
    if mesh is not None and len(mesh.vertices):
        out["mesh"] = {
            "positions": [[r(c) for c in v.co] for v in mesh.vertices],
            "faces": [list(p.vertices) for p in mesh.polygons],
            "edges": len(mesh.edges),
        }
    cloud = geometry.pointcloud
    if cloud is not None and len(cloud.points):
        attributes = {a.name: a for a in cloud.attributes}
        out["points"] = {
            "positions": values(attributes["position"]),
            "radius": values(attributes["radius"]) if "radius" in attributes else None,
            "attributes": named(cloud.attributes),
        }
    return out


def build(group):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.mesh.primitive_cube_add(size=2)
    ob = bpy.context.object
    tree = bpy.data.node_groups.new("T", "GeometryNodeTree")
    tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    nodes = {}
    for node in group["nodes"]:
        kind = node["kind"]
        made = tree.nodes.new(KINDS[kind])
        values_of = node.get("values", {})
        for key, value in values_of.items():
            if (kind, key) in MENUS:
                name, items = MENUS[(kind, key)]
                made.inputs[name].default_value = items[value]
            elif key in PROPS:
                if key == "keepLastSegment":
                    value = value == "true"
                elif isinstance(value, str):
                    value = value.upper()
                    if kind == "compare" and key == "dataType" and value == "FLOAT_VECTOR":
                        value = "VECTOR"
                setattr(made, PROPS[key], value)
        for key, value in values_of.items():
            if key in PROPS or (kind, key) in MENUS:
                continue
            socket(made.inputs, key).default_value = value
        nodes[node["id"]] = made
    links = list(group["links"])
    # A multi-input socket lists its newest link first: link those in reverse to keep the order.
    multi = [link for link in links if nodes[link["to"]].bl_idname == "GeometryNodeJoinGeometry"]
    for link in [link for link in links if link not in multi] + list(reversed(multi)):
        a, b = nodes[link["from"]], nodes[link["to"]]
        out = a.outputs[0] if link["fromSocket"] == "Geometry" and a.bl_idname == "NodeGroupInput" else socket(a.outputs, link["fromSocket"])
        into = b.inputs[0] if b.bl_idname == "NodeGroupOutput" else socket(b.inputs, link["toSocket"])
        tree.links.new(out, into)
    mod = ob.modifiers.new("GN", "NODES")
    mod.node_group = tree
    return record(ob)


version = bpy.app.version_string
json.dump({"blender": version, "cases": {name: build(group) for name, group in groups.items()}}, open(args[1], "w"), indent=0, sort_keys=True)
