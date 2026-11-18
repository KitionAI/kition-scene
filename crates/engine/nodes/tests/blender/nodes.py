# Mesh, instance, field, group, and zone nodes in the reference application, for
# tests/nodes_blender.rs. Each case (in Cube's JSON) is a main node group, the other groups its
# Group nodes use, and the frames to look at; each group is rebuilt as a geometry node tree, the
# main one runs on a cube, and what comes out is written down at each frame (the frames are gone
# through in order from the scene's start, as playing does): the mesh (vertices, faces, edges,
# and named attributes on every domain), and the points (positions and named attributes).
# Run: blender -b --factory-startup --python nodes.py -- cases.json out.json
import json
import sys

import bpy

args = sys.argv[sys.argv.index("--") + 1:]
cases = json.load(open(args[0]))

KINDS = {
    "groupInput": "NodeGroupInput", "groupOutput": "NodeGroupOutput", "group": "GeometryNodeGroup",
    "repeatInput": "GeometryNodeRepeatInput", "repeatOutput": "GeometryNodeRepeatOutput",
    "simulationInput": "GeometryNodeSimulationInput", "simulationOutput": "GeometryNodeSimulationOutput",
    "sceneTime": "GeometryNodeInputSceneTime", "curveCircle": "GeometryNodeCurvePrimitiveCircle",
    "meshCube": "GeometryNodeMeshCube", "meshGrid": "GeometryNodeMeshGrid", "meshLine": "GeometryNodeMeshLine",
    "meshCircle": "GeometryNodeMeshCircle", "meshIcoSphere": "GeometryNodeMeshIcoSphere", "meshUvSphere": "GeometryNodeMeshUVSphere",
    "meshCylinder": "GeometryNodeMeshCylinder", "meshCone": "GeometryNodeMeshCone",
    "transform": "GeometryNodeTransform", "joinGeometry": "GeometryNodeJoinGeometry", "setPosition": "GeometryNodeSetPosition",
    "deleteGeometry": "GeometryNodeDeleteGeometry", "extrudeMesh": "GeometryNodeExtrudeMesh",
    "meshToPoints": "GeometryNodeMeshToPoints", "pointsToVertices": "GeometryNodePointsToVertices",
    "instanceOnPoints": "GeometryNodeInstanceOnPoints", "realizeInstances": "GeometryNodeRealizeInstances",
    "storeNamedAttribute": "GeometryNodeStoreNamedAttribute", "captureAttribute": "GeometryNodeCaptureAttribute",
    "position": "GeometryNodeInputPosition", "normal": "GeometryNodeInputNormal", "index": "GeometryNodeInputIndex",
    "value": "ShaderNodeValue", "vector": "FunctionNodeInputVector",
    "separateXyz": "ShaderNodeSeparateXYZ", "combineXyz": "ShaderNodeCombineXYZ", "compare": "FunctionNodeCompare",
    "math": "ShaderNodeMath", "vectorMath": "ShaderNodeVectorMath", "switch": "GeometryNodeSwitch",
    "namedAttribute": "GeometryNodeInputNamedAttribute", "domainSize": "GeometryNodeAttributeDomainSize",
    "attributeStatistic": "GeometryNodeAttributeStatistic", "accumulateField": "GeometryNodeAccumulateField",
    "meshBoolean": "GeometryNodeMeshBoolean", "dualMesh": "GeometryNodeDualMesh", "flipFaces": "GeometryNodeFlipFaces",
    "splitEdges": "GeometryNodeSplitEdges", "triangulate": "GeometryNodeTriangulate", "scaleElements": "GeometryNodeScaleElements",
    "meshIsland": "GeometryNodeInputMeshIsland", "edgeAngle": "GeometryNodeInputMeshEdgeAngle",
    "faceArea": "GeometryNodeInputMeshFaceArea", "edgeNeighbors": "GeometryNodeInputMeshEdgeNeighbors",
    "sampleIndex": "GeometryNodeSampleIndex", "sampleNearest": "GeometryNodeSampleNearest", "raycast": "GeometryNodeRaycast",
    "geometryProximity": "GeometryNodeProximity", "translateInstances": "GeometryNodeTranslateInstances",
    "rotateInstances": "GeometryNodeRotateInstances", "scaleInstances": "GeometryNodeScaleInstances",
    "instanceRotation": "GeometryNodeInputInstanceRotation", "instanceScale": "GeometryNodeInputInstanceScale",
    "rotateEuler": "FunctionNodeRotateEuler", "rotateRotation": "FunctionNodeRotateRotation",
    "rotateVector": "FunctionNodeRotateVector", "eulerToRotation": "FunctionNodeEulerToRotation",
    "rotationToEuler": "FunctionNodeRotationToEuler", "axisAngleToRotation": "FunctionNodeAxisAngleToRotation",
    "invertRotation": "FunctionNodeInvertRotation",
}
PROPS = {
    "mode": "mode", "domain": "domain", "operation": "operation", "dataType": "data_type", "inputType": "input_type",
    "component": "component", "targetElement": "target_element", "rotationType": "rotation_type", "space": "space",
    "rotationSpace": "rotation_space", "clamp": "clamp", "useClamp": "use_clamp", "fillType": "fill_type",
}
# Settings the reference keeps as menu sockets: the socket and its items.
MENUS = {
    ("triangulate", "quadMethod"): ("Quad Method", {"beauty": "Beauty", "fixed": "Fixed", "fixed_alternate": "Fixed Alternate",
                                                    "shortest_diagonal": "Shortest Diagonal", "longest_diagonal": "Longest Diagonal"}),
    ("triangulate", "ngonMethod"): ("N-gon Method", {"beauty": "Beauty", "clip": "Clip"}),
    ("raycast", "interpolation"): ("Interpolation", {"interpolated": "Interpolated", "nearest": "Nearest"}),
    ("scaleElements", "scaleMode"): ("Scale Mode", {"uniform": "Uniform", "single_axis": "Single Axis"}),
    ("meshBoolean", "solver"): ("Solver", {"exact": "Exact", "float": "Float", "manifold": "Manifold"}),
}
ALIASES = {"A_INT": "A", "B_INT": "B", "A_VEC3": "A", "B_VEC3": "B", "Min_001": "Min", "Max_001": "Max", "Vector_001": "Vector",
           "Value_001": "Value", "Group Index": "Group ID", "Source Position": "Sample Position", "Target": "Geometry"}
TYPES = {"geometry": "GEOMETRY", "float": "FLOAT", "int": "INT", "bool": "BOOLEAN", "vector": "VECTOR", "rotation": "ROTATION"}
SOCKET_TYPES = {"geometry": "NodeSocketGeometry", "float": "NodeSocketFloat", "int": "NodeSocketInt", "bool": "NodeSocketBool",
                "vector": "NodeSocketVector", "rotation": "NodeSocketRotation"}
BUILTIN = {"position", "radius", "id", "sharp_face", "sharp_edge", "material_index", "instance_transform", "UVMap", "uv_map"}


def socket(sockets, key, nth=0):
    found = [s for s in sockets if s.identifier == key and s.enabled]
    if not found:
        key = ALIASES.get(key, key)
        found = [s for s in sockets if (s.identifier == key or s.name == key) and s.enabled]
    if not found:
        raise KeyError(key)
    return found[min(nth, len(found) - 1)]


def r(value):
    return round(value, 5)


def values(attribute):
    out = []
    for item in attribute.data:
        if hasattr(item, "vector"):
            out.append([r(c) for c in item.vector])
        elif hasattr(item, "value") and isinstance(item.value, float):
            out.append(r(item.value))
        elif hasattr(item, "value") and hasattr(item.value, "to_euler"):
            out.append([r(c) for c in item.value.to_euler()])
        else:
            out.append(item.value)
    return out


def named(attributes, edge_order=None):
    out = {}
    for a in attributes:
        if a.name in BUILTIN or a.name.startswith("."):
            continue
        found = values(a)
        if a.domain == "EDGE" and edge_order is not None:
            found = [found[i] for i in edge_order]
        out[a.name] = {"domain": a.domain.lower(), "values": found}
    return out


def record(ob):
    graph = bpy.context.evaluated_depsgraph_get()
    geometry = ob.evaluated_get(graph).evaluated_geometry()
    out = {}
    mesh = geometry.mesh
    if mesh is not None and len(mesh.vertices):
        # Edges by their vertex pairs, in order (the reference's own edge order can vary from
        # run to run).
        pairs = [sorted(e.vertices) for e in mesh.edges]
        order = sorted(range(len(pairs)), key=lambda i: pairs[i])
        out["mesh"] = {
            "positions": [[r(c) for c in v.co] for v in mesh.vertices],
            "faces": [list(p.vertices) for p in mesh.polygons],
            "edges": [pairs[i] for i in order],
            "attributes": named(mesh.attributes, order),
        }
    cloud = geometry.pointcloud
    if cloud is not None and len(cloud.points):
        attributes = {a.name: a for a in cloud.attributes}
        out["points"] = {
            "positions": values(attributes["position"]),
            "attributes": named(cloud.attributes),
        }
    return out


def build_tree(group, trees):
    tree = trees[group["id"]]
    tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    for output in group.get("outputs", []):
        tree.interface.new_socket(output["name"], in_out="OUTPUT", socket_type=SOCKET_TYPES[output["type"]])
    tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    for put in group.get("inputs", []):
        made = tree.interface.new_socket(put["name"], in_out="INPUT", socket_type=SOCKET_TYPES[put["type"]])
        if put.get("default") is not None:
            made.default_value = put["default"]
    nodes = {}
    for node in group["nodes"]:
        kind = node["kind"]
        made = tree.nodes.new(KINDS[kind])
        nodes[node["id"]] = made
        if kind == "group":
            made.node_tree = trees[node["values"]["group"]]
    # Zones: pair each input with its output and give the output its items.
    for node in group["nodes"]:
        if node["kind"] in ("repeatInput", "simulationInput"):
            output = nodes[node["values"]["output"]]
            nodes[node["id"]].pair_with_output(output)
    for node in group["nodes"]:
        if node["kind"] in ("repeatOutput", "simulationOutput") and "items" in node.get("values", {}):
            made = nodes[node["id"]]
            items = made.repeat_items if node["kind"] == "repeatOutput" else made.state_items
            items.clear()
            for item in node["values"]["items"]:
                items.new(TYPES[item["type"]], item["name"])
    for node in group["nodes"]:
        kind = node["kind"]
        made = nodes[node["id"]]
        values_of = node.get("values", {})
        for key, value in values_of.items():
            if (kind, key) in MENUS:
                name, items = MENUS[(kind, key)]
                made.inputs[name].default_value = items[value]
            elif key in PROPS:
                if value in ("true", "false"):
                    value = value == "true"
                elif isinstance(value, str):
                    value = value.upper()
                    if kind == "compare" and key == "dataType" and value == "FLOAT_VECTOR":
                        value = "VECTOR"
                    if kind == "domainSize" and value == "INSTANCES":
                        value = "INSTANCES"
                setattr(made, PROPS[key], value)
        for key, value in values_of.items():
            if key in PROPS or (kind, key) in MENUS or key in ("group", "output", "items"):
                continue
            if kind == "vector":
                made.vector = value
            elif kind == "value":
                made.outputs[0].default_value = value
            else:
                socket(made.inputs, key).default_value = value
    links = list(group["links"])
    multi = [link for link in links if nodes[link["to"]].bl_idname in ("GeometryNodeJoinGeometry", "GeometryNodeMeshBoolean")]
    for link in [link for link in links if link not in multi] + list(reversed(multi)):
        a, b = nodes[link["from"]], nodes[link["to"]]
        out = socket(a.outputs, link["fromSocket"])
        into = socket(b.inputs, link["toSocket"])
        tree.links.new(out, into)


def build(case):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.mesh.primitive_cube_add(size=2)
    ob = bpy.context.object
    scene = bpy.context.scene
    groups = [case["group"]] + case.get("groups", [])
    trees = {group["id"]: bpy.data.node_groups.new(group["name"], "GeometryNodeTree") for group in groups}
    # Used groups first, so a Group node's sockets are there when it is linked.
    for group in reversed(groups):
        build_tree(group, trees)
    mod = ob.modifiers.new("GN", "NODES")
    mod.node_group = trees[case["group"]["id"]]
    frames = case.get("frames")
    if not frames:
        return record(ob)
    out = {}
    for frame in range(scene.frame_start, max(frames) + 1):
        scene.frame_set(frame)
        if frame in frames:
            out[str(frame)] = record(ob)
    for frame in [frame for frame in frames if frame < scene.frame_start]:
        scene.frame_set(frame)
        out[str(frame)] = record(ob)
    return out


version = bpy.app.version_string
json.dump({"blender": version, "cases": {name: build(case) for name, case in cases.items()}}, open(args[1], "w"), indent=0, sort_keys=True)
