"""Trimming vector-handle Bezier curves created by the new Poly conversion."""
import copy
from curves10_cases import source

def groups():
    out = {}
    for cyclic in [False, True]:
        t = source(7, cyclic, "poly", True)
        t.then("setSplineType", {"splineType": "bezier"})
        t.then("trimCurve", {"Start": .2, "End": .8})
        out[f"vector-trim-{'cyclic' if cyclic else 'open'}-controls"] = copy.deepcopy(t).finish()
        t.then("resampleCurve", {"mode": "evaluated"})
        out[f"vector-trim-{'cyclic' if cyclic else 'open'}-evaluated"] = t.finish()
    return out
