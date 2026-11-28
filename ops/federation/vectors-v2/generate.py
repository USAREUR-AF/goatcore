#!/usr/bin/env python3
"""Writes ops/federation/vectors-v2/ (package v2, ADR 1242).

    python3 -I ops/federation/vectors-v2/generate.py <repository root>

It uses the canonical form and the Ed25519 signer of ops/federation/standup/directory.py,
and the public test keys in ops/federation/vectors/testkeys/. directory.py and
crates/goat-federation are both this project's code, so the vectors agreeing with the
Rust library shows the two are consistent with each other, not that either matches an
outside implementation (README.md, "Provenance"). Run again, it writes the same bytes:
Ed25519 signatures are deterministic.
"""
import importlib.util, json, hashlib, os, sys, base64
REPO=sys.argv[1]
spec=importlib.util.spec_from_file_location("d", f"{REPO}/ops/federation/standup/directory.py"); d=importlib.util.module_from_spec(spec); spec.loader.exec_module(d)
V1=f"{REPO}/ops/federation/vectors"; OUT=f"{REPO}/ops/federation/vectors-v2"; V2=OUT
seed={s: bytes.fromhex(open(f"{V1}/testkeys/{s}.seed.hex").read().strip()) for s in ("xaa","xab")}
def load(s): return json.loads(open(f"{V1}/compose-full/package-{s}.canonical.json","rb").read())
NEXT={"xaa":"sha256:"+hashlib.sha256(b"xaa next CA (test vector)").hexdigest(),
      "xab":"sha256:"+hashlib.sha256(b"xab next CA (test vector)").hexdigest()}
def pkg(s, version, nxt):
    v=load(s); v.pop("signature",None); v["spec"]=f"federation-package/{version}"
    if nxt: v["identity"]["ca_fingerprint_next"]=NEXT[s]
    si=d.canonical(v)
    v["signature"]=base64.b64encode(d.sign(seed[s], si)).decode()
    return v, si
def write(dirn, name, data):
    os.makedirs(dirn, exist_ok=True)
    open(os.path.join(dirn,name),"wb").write(data)
def write_pkg(dirn, s, v, si):
    write(dirn, f"package-{s}.canonical.json", d.canonical(v))
    write(dirn, f"package-{s}.signing-input.canonical.json", si)
    write(dirn, f"package-{s}.pretty.json", (json.dumps(v, indent=2, sort_keys=True, ensure_ascii=False)+"\n").encode())
agree=json.loads(open(f"{V1}/compose-full/agreement.canonical.json","rb").read())
def agreement(nexts):
    a=json.loads(json.dumps(agree)); a["spec"]="federation-agreement/v2"
    for p in a["parties"]:
        if p["sovereign_id"] in nexts: p["ca_fingerprint_next"]=NEXT[p["sovereign_id"]]
    c=d.canonical(a); return a, c, "sha256:"+hashlib.sha256(c).hexdigest()
for name, nexts in (("compose-next-one-side",{"xaa"}),("compose-next-both-sides",{"xaa","xab"}),("compose-no-next",set())):
    dd=f"{OUT}/{name}"
    for s in ("xaa","xab"):
        v,si=pkg(s,"v2",s in nexts); write_pkg(dd,s,v,si)
    a,c,g=agreement(nexts)
    write(dd,"agreement.canonical.json",c); write(dd,"agreement.digest.txt",g.encode())
    write(dd,"agreement.pretty.json",(json.dumps(a,indent=2,sort_keys=True,ensure_ascii=False)+"\n").encode())
# refuse-mixed-versions: xaa v1 (with no next), xab v2
dd=f"{OUT}/refuse-mixed-versions"
v,si=pkg("xaa","v1",False); write_pkg(dd,"xaa",v,si)
v,si=pkg("xab","v2",True); write_pkg(dd,"xab",v,si)
# refuse-both-v3
dd=f"{OUT}/refuse-both-v3"
for s in ("xaa","xab"):
    v,si=pkg(s,"v3",False); write_pkg(dd,s,v,si)
# malformed: next equals current, and a bad next format; unsigned, as v1's malformed/ are
m=f"{OUT}/malformed"
for fname, mut in (("next-equals-current.json", lambda v: v["identity"].__setitem__("ca_fingerprint_next", v["identity"]["ca_fingerprint"])),
                   ("next-not-a-fingerprint.json", lambda v: v["identity"].__setitem__("ca_fingerprint_next", "sha256:XYZ"))):
    v=load("xaa"); v.pop("signature",None); v["spec"]="federation-package/v2"; mut(v)
    write(m,fname,d.canonical(v))

def rd(p): return json.loads(open(p,"rb").read())
def put(dirn, s, v):
    os.makedirs(dirn, exist_ok=True)
    open(f"{dirn}/package-{s}.canonical.json","wb").write(d.canonical(v))
    open(f"{dirn}/package-{s}.pretty.json","wb").write((json.dumps(v,indent=2,sort_keys=True,ensure_ascii=False)+"\n").encode())
v2a=rd(f"{V2}/compose-next-one-side/package-xaa.canonical.json")
v1b=rd(f"{V1}/compose-full/package-xab.canonical.json")
v1a=rd(f"{V1}/compose-full/package-xaa.canonical.json")
v2b=rd(f"{V2}/compose-next-one-side/package-xab.canonical.json")
# a v2 package relabelled v1, its next CA kept; signature unchanged
a=json.loads(json.dumps(v2a)); a["spec"]="federation-package/v1"
put(f"{V2}/refuse-downgrade-next-kept","xaa",a); put(f"{V2}/refuse-downgrade-next-kept","xab",v1b)
# the same, its next CA dropped; signature unchanged
a=json.loads(json.dumps(v2a)); a["spec"]="federation-package/v1"; del a["identity"]["ca_fingerprint_next"]
put(f"{V2}/refuse-downgrade-next-dropped","xaa",a); put(f"{V2}/refuse-downgrade-next-dropped","xab",v1b)
# a v1 package relabelled v2; signature unchanged
a=json.loads(json.dumps(v1a)); a["spec"]="federation-package/v2"
put(f"{V2}/refuse-relabel-v1-as-v2","xaa",a); put(f"{V2}/refuse-relabel-v1-as-v2","xab",v2b)

print("vectors-v2: written")
