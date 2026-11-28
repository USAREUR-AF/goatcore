#!/usr/bin/env python3
"""rollout-preflight.py — the rollout readiness/reversibility FACTS, script avenue (ADR 1211).

The CLI `goatbuilder preflight` is the supported, richer path: it also measures
the readiness half (ref pullable, pins --check, the secrets gate, fresh source,
the declared end state) and validates the rollback fact. This script is the
documented avenue (ADR 1201) for the three DECLARED facts a deploy's prep phase
prints, read from ops/pins/fleet-pins.yaml and the open graduation sets, graded
exactly as the Rust does (crates/goatbuilder/src/preflight.rs::grade_facts):

  State    stateless | stateful
  Rollback pin-revert | restore-required | none
  Scope    single | coupled

FAIL CLOSED: an undeclared component reads `stateful · rollback: none
(undeclared)` and exits 3 — a gap made visible, never guessed green.

usage: rollout-preflight.py <component> --repo DIR [--json]
exit:  0 the three facts are clean and declared; 3 not-clean; 2 cannot run.
"""
import argparse, glob, json, os, sys

try:
    import yaml
except ImportError:
    sys.exit("rollout-preflight: PyYAML missing (pip install pyyaml)")

ROLLBACK_KINDS = ("pin-revert", "restore-required", "none")


def grade_facts(spec, graduation_set, component):
    at = spec.get("artifact_type") or "image"
    reversible = spec.get("reversible")
    owns = spec.get("owns_state")
    rb = spec.get("rollback")
    rf = spec.get("restore_from") or {}

    # State.
    if owns is True:
        state, state_clean = "stateful", True
    elif owns is False and at == "data":
        state, state_clean = "stateful (declared stateless, but artifact_type=data)", False
    elif owns is False:
        state, state_clean = "stateless", True
    elif at == "data":
        state, state_clean = "stateful", True
    else:
        state, state_clean = "stateful (undeclared — fail closed)", False

    # Rollback.
    safe_pin = at in ("image", "config", "release") and reversible is not False
    if rb == "pin-revert" and not safe_pin:
        extra = ", reversible=false" if reversible is False else ""
        rollback, rb_clean = f"pin-revert (declared, but artifact_type={at}{extra} — a ref swap cannot undo this)", False
    elif rb == "pin-revert":
        rollback, rb_clean = "pin-revert", True
    elif rb == "restore-required":
        pointer = (rf.get("pointer") or "").strip()
        if pointer:
            window = ""
            if rf.get("retention_days") is not None and rf.get("as_of"):
                window = f" ({rf['retention_days']}-day window)"
            elif rf.get("retention_days") is not None:
                window = f" ({rf['retention_days']}-day retention; start date not declared)"
            rollback, rb_clean = f"restore-required: {pointer}{window}", True
        else:
            rollback, rb_clean = "restore-required (no restore_from pointer declared)", False
    elif rb == "none":
        rollback, rb_clean = "none", True
    elif rb:
        rollback, rb_clean = f"{rb} (not one of {', '.join(ROLLBACK_KINDS)})", False
    else:
        rollback, rb_clean = "none (undeclared — fail closed)", False

    # Scope.
    if graduation_set:
        name, members = graduation_set
        others = [m for m in members if m != component] or ["no other members"]
        scope = f"coupled — graduation set '{name}' with {', '.join(others)}"
    elif spec.get("coupled_with"):
        scope = f"coupled — declared with {', '.join(spec['coupled_with'])}"
    else:
        scope = "single"

    return {"state": state, "state_clean": state_clean, "rollback": rollback,
            "rollback_clean": rb_clean, "scope": scope, "scope_clean": True}


def open_set_for(repo, component):
    for path in sorted(glob.glob(os.path.join(repo, "ops/release/sets/*.yaml"))):
        try:
            with open(path) as fh:
                doc = yaml.safe_load(fh) or {}
        except Exception:
            continue
        if doc.get("state") != "open":
            continue
        members = [m.get("component") for m in (doc.get("members") or []) if isinstance(m, dict)]
        if component in members:
            return (doc.get("name") or "?", members)
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("component")
    ap.add_argument("--repo", default=".")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    pins_path = os.path.join(a.repo, "ops/pins/fleet-pins.yaml")
    try:
        with open(pins_path) as fh:
            pins = yaml.safe_load(fh) or {}
    except Exception as e:
        sys.exit(f"rollout-preflight: cannot read {pins_path}: {e}")
    spec = (pins.get("components") or {}).get(a.component)
    if spec is None:
        sys.exit(f"rollout-preflight: unknown component {a.component!r} in {pins_path}")

    f = grade_facts(spec, open_set_for(a.repo, a.component), a.component)
    clean = f["state_clean"] and f["rollback_clean"] and f["scope_clean"]
    if a.json:
        print(json.dumps({**f, "component": a.component, "clean": clean}))
    else:
        mark = lambda ok: "ok  " if ok else "FAIL"
        print(f"{mark(f['state_clean'])}  State:    {f['state']}")
        print(f"{mark(f['rollback_clean'])}  Rollback: {f['rollback']}")
        print(f"{mark(f['scope_clean'])}  Scope:    {f['scope']}")
        verdict = "ready" if clean else "NOT clean (declare the missing facts in ops/pins/fleet-pins.yaml) (GB-076)"
        print(f"prep: {f['state'].split()[0]} · rollback: {f['rollback']} · {f['scope']}  — {verdict}")
    sys.exit(0 if clean else 3)


if __name__ == "__main__":
    main()
