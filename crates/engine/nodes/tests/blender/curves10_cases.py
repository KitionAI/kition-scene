"""Small independent public-application scenes for Scene-R10 spline-type parity."""
import copy

class Tree:
    def __init__(self):
        self.group = {"id": 50, "name": "Spline types", "nodes": [], "links": []}
        self.next = 100
        self.last = None

    def add(self, kind, values=None):
        node = self.next
        self.next += 1
        self.group["nodes"].append({"id": node, "kind": kind, "values": values or {}})
        return node

    def link(self, source, target, socket):
        self.group["links"].append({"from": source[0], "fromSocket": source[1], "to": target, "toSocket": socket})

    def then(self, kind, values=None, socket="Curve", output="Curve"):
        node = self.add(kind, values)
        if self.last:
            self.link(self.last, node, socket)
        self.last = (node, output)
        return node

    def finish(self):
        out = self.add("groupOutput")
        self.link(self.last, out, "Geometry")
        return self.group


def source(count=7, cyclic=False, kind="poly", attributes=False):
    t = Tree()
    if count == 1:
        t.then("curveLine")
        t.then("resampleCurve", {"Count": 1})
    elif kind == "bezier":
        t.then("curveBezierSegment", {"Resolution": 3, "Start": [-1, 0, .2], "Start Handle": [-.4, 1, .5], "End Handle": [.7, -.6, -.2], "End": [1.3, .4, .1]})
    else:
        t.then("curveSpiral", {"Resolution": count-1, "Rotations": 1, "Start Radius": .7, "End Radius": 1.4, "Height": .6})
    t.then("setSplineCyclic", {"Cyclic": cyclic}, "Geometry", "Geometry")
    t.then("setSplineResolution", {"Resolution": 3}, "Geometry", "Geometry")
    if attributes:
        index = t.add("index")
        pos = t.add("position")
        for name, data, field in [("scalar", "float", (index, "Index")), ("vector", "float_vector", (pos, "Position"))]:
            store = t.then("storeNamedAttribute", {"Name": name, "domain": "point", "dataType": data}, "Geometry", "Geometry")
            t.link(field, store, "Value")
        radius = t.then("setCurveRadius", {"Radius": .4})
        t.link((index, "Index"), radius, "Radius")
        t.then("setCurveTilt", {"Tilt": .15})
        t.then("storeNamedAttribute", {"Name": "curveTag", "domain": "curve", "dataType": "int", "Value": 17}, "Geometry", "Geometry")
    if kind not in ["poly", "bezier"]:
        t.then("setSplineType", {"splineType": kind})
    return t


def groups():
    out = {}
    def keep(name, tree): out[name] = tree.finish()
    for cyclic in [False, True]:
        side = "cyclic" if cyclic else "open"
        for kind in ["catmullRom", "nurbs"]:
            for count in [1, 2, 3, 4, 7]:
                t = source(count, cyclic, kind)
                keep(f"{kind}-{side}-{count}-controls", copy.deepcopy(t))
                t.then("resampleCurve", {"mode": "evaluated"})
                keep(f"{kind}-{side}-{count}-evaluated", t)
        for order in range(1, 7):
            t = source(7, cyclic, "nurbs", True)
            t.then("setNurbsOrder", {"Order": order}, "Curves", "Curves")
            t.then("resampleCurve", {"mode": "evaluated"})
            keep(f"nurbs-{side}-order-{order}", t)
        for start in ["poly", "catmullRom", "bezier", "nurbs"]:
            for target in ["poly", "catmullRom", "bezier", "nurbs"]:
                if start == "nurbs" and target == "bezier":
                    continue  # This conversion is explicitly refused; tested separately in Rust.
                t = source(7, cyclic, start, True)
                t.then("setSplineType", {"splineType": target})
                keep(f"convert-{start}-{target}-{side}-controls", copy.deepcopy(t))
                t.then("resampleCurve", {"mode": "evaluated"})
                keep(f"convert-{start}-{target}-{side}-evaluated", t)
        for weight in [-1, 0, .25, 2]:
            t = source(7, cyclic, "nurbs", True)
            t.then("setNurbsWeight", {"Weight": weight}, "Curves", "Curves")
            keep(f"weight-{side}-{weight}-controls", copy.deepcopy(t))
            t.then("resampleCurve", {"mode": "evaluated"})
            keep(f"weight-{side}-{weight}-evaluated", t)
        for kind in ["catmullRom", "nurbs"]:
            for operation, values, socket, output in [
                ("reverseCurve", {}, "Curve", "Curve"),
                ("trimCurve", {"Start": .2, "End": .8}, "Curve", "Curve"),
                ("resampleCurve", {"mode": "count", "Count": 11}, "Curve", "Curve"),
                ("resampleCurve", {"mode": "length", "Length": .3}, "Curve", "Curve"),
                ("curveToMesh", {}, "Curve", "Mesh"),
                ("curveToPoints", {"mode": "evaluated"}, "Curve", "Points"),
                ("curveToPoints", {"mode": "count", "Count": 9}, "Curve", "Points"),
                ("transform", {"Translation": [.5, -.2, .4], "Rotation": [.2, .3, -.1], "Scale": [1.2, .7, 1.5]}, "Geometry", "Geometry"),
                ("setPosition", {"Offset": [.3, -.5, .2]}, "Geometry", "Geometry"),
            ]:
                t = source(7, cyclic, kind, True)
                node = t.then(operation, values, socket, output)
                if operation == "curveToPoints":
                    for attr, sock in [("tangent", "Tangent"), ("normal", "Normal")]:
                        store = t.then("storeNamedAttribute", {"Name": attr, "dataType": "float_vector", "domain": "point"}, "Geometry", "Geometry")
                        t.link((node, sock), store, "Value")
                keep(f"{kind}-{side}-{operation}-{values.get('mode', 'default')}", t)
        # A point-domain field changes selected rational weights, including a zero endpoint.
        t = source(7, cyclic, "nurbs", True)
        index = t.add("index")
        weight = t.then("setNurbsWeight", {}, "Curves", "Curves")
        t.link((index, "Index"), weight, "Weight")
        t.then("resampleCurve", {"mode": "evaluated"})
        keep(f"weights-field-{side}", t)
        # Turning cyclic on after conversion retains Endpoint knot mode.
        t = source(7, False, "nurbs", True)
        t.then("setSplineCyclic", {"Cyclic": cyclic}, "Geometry", "Geometry")
        t.then("resampleCurve", {"mode": "evaluated"})
        keep(f"endpoint-knots-{side}", t)
        # Shape-preserving cubic conversion back to Bezier.
        t = source(7, cyclic, "bezier", True)
        t.then("setSplineType", {"splineType": "nurbs"})
        t.then("setSplineType", {"splineType": "bezier"})
        keep(f"bezier-nurbs-bezier-{side}", t)
    # Curve-domain selection switches only one member of a mixed component.
    t = source(7, False, "poly", True)
    original = t.last
    second = t.add("curveCircle", {"Resolution": 5})
    join = t.add("joinGeometry")
    t.link(original, join, "Geometry")
    t.link((second, "Curve"), join, "Geometry")
    t.last = (join, "Geometry")
    index = t.add("index")
    equal = t.add("compare", {"dataType": "int", "operation": "equal", "B_INT": 1})
    t.link((index, "Index"), equal, "A_INT")
    switch = t.then("setSplineType", {"splineType": "nurbs"})
    t.link((equal, "Result"), switch, "Selection")
    keep("selected-mixed-curves", copy.deepcopy(t))
    t.then("resampleCurve", {"mode": "evaluated"})
    keep("selected-mixed-evaluated", t)
    return out
