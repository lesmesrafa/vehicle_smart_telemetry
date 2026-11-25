"""Reads the windows written as N-Triples and as Jelly with independent libraries and compares them."""
import sys
import rdflib
from rdflib import Literal
from rdflib.compare import isomorphic
from pyjelly.integrations.rdflib.parse import parse_jelly_grouped

def by_value(graph):
    """rdflib rewrites "0" as "0.0" when it reads N-Triples and not when it reads Jelly: compare numbers by value."""
    out = rdflib.Graph()
    for s, p, o in graph:
        if isinstance(o, Literal) and o.datatype is not None and not str(o.datatype).endswith("#integer"):
            o = Literal(repr(float(str(o))), datatype=o.datatype)
        out.add((s, p, o))
    return out

nt_path, jelly_path = sys.argv[1], sys.argv[2]
nt = rdflib.Graph(); nt.parse(nt_path, format="nt")
whole = rdflib.Graph(); whole.parse(jelly_path, format="jelly")
print(f"N-Triples, read by rdflib {rdflib.__version__}: {len(nt)} triples")
print(f"Jelly, read by pyjelly: {len(whole)} triples; same graph: {isomorphic(by_value(nt), by_value(whole))}")
with open(jelly_path, "rb") as f:
    frames = [g for g in parse_jelly_grouped(f)]
sizes = [len(g) for g in frames]
print(f"Jelly frames: {len(frames)}, each one a graph; triples per graph from {min(sizes)} to {max(sizes)}; total {sum(sizes)}")
J = "https://example.org/jedge/voc#"
windows = sum(len(list(g.subjects(rdflib.RDF.type, rdflib.URIRef(J + "Window")))) for g in frames)
print(f"window nodes: {windows}, one per frame: {windows == len(frames)}")
