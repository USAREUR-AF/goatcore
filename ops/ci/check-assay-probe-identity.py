#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Every goatnet that declares the assay probe identity carries all of it.

The acceptance assay's goat CLI rows run as the non-person context
`assay-probe` (goatnet-acceptance-suite.md §"The probe identity"). That identity
is four facts in four files, and a missing one fails silently: the router drops
an unlisted CN's reads, so every goat row would read UNKNOWN or FAIL
while the goatnet is fine.

For each group_vars/<group>/vars.yml that sets `assay_probe_enabled: true`:
  1. ops/enrollment/manifests/<site>-certs.yaml has leaf `assay-probe` with
     cn goat-assay-probe-trial-peer and this site's own URI SAN;
  2. if the router names its universal readers (the read partitioned,
     zenoh_partner_read_partition true, or zenoh_identity_model
     per-uuid-slot-pool, where the universal read holds only the non-slot
     readers zenoh_universal_read_cns lists), zenoh_universal_read_cns lists the
     CN;
  3. ops/validation/values-<site>.yaml carries probe_context assay-probe and a
     probe_via that is a management machine (regenerated, not stale);
and, once, the router template defines the infra-assay-probe subject bound to
the infra-w-assay-probe rule, which writes nothing beyond the probe's own keys.

Usage: check-assay-probe-identity.py [--self-test]
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

CN = "goat-assay-probe-trial-peer"
WRITE_KEYS = ['"hello/assay-probe"', '"volatile/goat-assay/**"']


def _scalar(text: str, key: str) -> str | None:
    m = re.search(rf"(?m)^{re.escape(key)}:\s*(\S[^#\n]*)", text)
    return m.group(1).strip().strip("\"'") if m else None


def _list(text: str, key: str) -> list[str]:
    m = re.search(rf"(?ms)^{re.escape(key)}:\s*\n((?:\s*(?:-.*|#.*)?\n)+)", text)
    if not m:
        return []
    return [re.sub(r"\s+#.*$", "", l).strip()[1:].strip().strip("\"'")
            for l in m.group(1).splitlines() if l.strip().startswith("-")]


def universal_read_error(group_text: str) -> str | None:
    """Rule 2 for one group's group_vars text: None when the probe can read."""
    partitioned = (_scalar(group_text, "zenoh_partner_read_partition") or "").lower() == "true"
    slot_pool = _scalar(group_text, "zenoh_identity_model") == "per-uuid-slot-pool"
    if not (partitioned or slot_pool) or CN in _list(group_text, "zenoh_universal_read_cns"):
        return None
    why = "the read is partitioned" if partitioned else "per-uuid-slot-pool names its universal readers"
    return f"{why} and zenoh_universal_read_cns does not list {CN}; the router would drop the probe's reads"


def check(repo: Path) -> list[str]:
    errs: list[str] = []
    gv = repo / "ops/ansible/group_vars"
    declared = []
    for vars_file in sorted(gv.glob("*_hosts/vars.yml")):
        text = vars_file.read_text()
        if (_scalar(text, "assay_probe_enabled") or "").lower() != "true":
            continue
        group_dir = vars_file.parent
        site = _scalar(text, "site_id")
        if not site:
            errs.append(f"{vars_file}: assay_probe_enabled without a site_id")
            continue
        declared.append(site)
        manifest = repo / f"ops/enrollment/manifests/{site}-certs.yaml"
        mtext = manifest.read_text() if manifest.is_file() else ""
        row = re.search(r"(?ms)^\s*- name: assay-probe\s*\n(.*?)(?=^\s*- name:|\Z)", mtext)
        if not row:
            errs.append(f"{manifest}: no leaf 'assay-probe' (copy it from TEMPLATE-certs.yaml)")
        else:
            if f"cn: {CN}" not in row.group(1):
                errs.append(f"{manifest}: leaf assay-probe must carry cn {CN}")
            if f"urn:goat:{site}:infra:assay-probe" not in row.group(1):
                errs.append(f"{manifest}: leaf assay-probe must carry this site's URI urn:goat:{site}:infra:assay-probe")
        group_text = "".join(p.read_text() for p in sorted(group_dir.glob("*.yml")))
        read_err = universal_read_error(group_text)
        if read_err:
            errs.append(f"{group_dir}: {read_err}")
        values = repo / f"ops/validation/values-{site}.yaml"
        vtext = values.read_text() if values.is_file() else ""
        if _scalar(vtext, "probe_context") != "assay-probe" or not (_scalar(vtext, "probe_via") or "").count("@"):
            errs.append(f"{values}: no probe_context/probe_via for the declared probe — run "
                        f"ops/validation/generate-values.py {site}")
    if declared:
        tpl = (repo / "ops/ansible/roles/goat-zenoh/templates/config.json5.j2").read_text()
        rule = re.search(r'id: "infra-w-assay-probe",(.*?)\}', tpl, re.S)
        if not rule:
            errs.append("goat-zenoh config.json5.j2: no rule infra-w-assay-probe")
        else:
            keys = re.search(r"key_exprs:\s*\[(.*?)\]", rule.group(1), re.S)
            got = sorted(k.strip() for k in (keys.group(1).split(",") if keys else []) if k.strip())
            if got != sorted(WRITE_KEYS):
                errs.append(f"goat-zenoh config.json5.j2: infra-w-assay-probe writes {got}, want exactly {sorted(WRITE_KEYS)}")
        if not re.search(rf'id: "infra-assay-probe",\s*cert_common_names: \["{CN}"\]', tpl):
            errs.append(f"goat-zenoh config.json5.j2: no subject infra-assay-probe for CN {CN}")
        if '{ rules: ["infra-w-assay-probe"], subjects: ["infra-assay-probe"] }' not in tpl:
            errs.append("goat-zenoh config.json5.j2: infra-assay-probe must be bound to infra-w-assay-probe alone "
                        "(its read is the universal read, which keeps the operator-audit deny)")
    return errs


def _fixture(root: Path, *, listed: bool, values_ok: bool, manifest_ok: bool, wide_rule: bool,
             mode: str = "partition") -> None:
    gv = root / "ops/ansible/group_vars/x_hosts"
    gv.mkdir(parents=True)
    (gv / "vars.yml").write_text("site_id: x\nassay_probe_enabled: true\n")
    cns = f"  - {CN}\n" if listed else "  - other-trial-peer\n"
    head = {"partition": "zenoh_partner_read_partition: true\n",
            "slot-pool": "zenoh_identity_model: per-uuid-slot-pool\n",
            "per-vendor": "zenoh_identity_model: per-vendor\n"}[mode]
    (gv / "data-access.yml").write_text(f"{head}zenoh_universal_read_cns:\n{cns}")
    man = root / "ops/enrollment/manifests"
    man.mkdir(parents=True)
    cn = CN if manifest_ok else "operator-trial-peer"
    (man / "x-certs.yaml").write_text(
        f"leaves:\n  - name: assay-probe\n    cn: {cn}\n    uri_sans: [\"urn:goat:x:infra:assay-probe\"]\n")
    val = root / "ops/validation"
    val.mkdir(parents=True)
    (val / "values-x.yaml").write_text('probe_context: "assay-probe"\nprobe_via: "u@h"\n' if values_ok else "")
    tpl = root / "ops/ansible/roles/goat-zenoh/templates"
    tpl.mkdir(parents=True)
    keys = '"hello/assay-probe", "volatile/goat-assay/**"' + (', "ops/**"' if wide_rule else "")
    (tpl / "config.json5.j2").write_text(
        f'      {{\n        id: "infra-w-assay-probe",\n        key_exprs: [{keys}],\n      }},\n'
        f'      {{\n        id: "infra-assay-probe",\n        cert_common_names: ["{CN}"],\n      }},\n'
        '      { rules: ["infra-w-assay-probe"], subjects: ["infra-assay-probe"] },\n')


def self_test() -> int:
    cases = [
        ("complete", dict(listed=True, values_ok=True, manifest_ok=True, wide_rule=False), 0),
        ("CN not in the partitioned read", dict(listed=False, values_ok=True, manifest_ok=True, wide_rule=False), 1),
        ("values not regenerated", dict(listed=True, values_ok=False, manifest_ok=True, wide_rule=False), 1),
        ("manifest leaf has another CN", dict(listed=True, values_ok=True, manifest_ok=False, wide_rule=False), 1),
        ("rule writes beyond its own keys", dict(listed=True, values_ok=True, manifest_ok=True, wide_rule=True), 1),
        ("slot pool, CN listed", dict(listed=True, values_ok=True, manifest_ok=True, wide_rule=False, mode="slot-pool"), 0),
        ("slot pool, CN not in the named read", dict(listed=False, values_ok=True, manifest_ok=True, wide_rule=False, mode="slot-pool"), 1),
        ("per-vendor, unpartitioned, CN not listed", dict(listed=False, values_ok=True, manifest_ok=True, wide_rule=False, mode="per-vendor"), 0),
    ]
    bad = 0
    for name, kw, want in cases:
        with tempfile.TemporaryDirectory() as d:
            _fixture(Path(d), **kw)
            got = len(check(Path(d)))
            ok = (got == 0) == (want == 0)
            print(f"  {'ok  ' if ok else 'FAIL'} {name}: {got} error(s)")
            bad += not ok
    print("self-test: " + ("PASS" if not bad else f"{bad} case(s) wrong"))
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = Path(__file__).resolve().parents[2]
    errs = check(repo)
    for e in errs:
        print(f"FAIL: {e}")
    if errs:
        return 1
    print("ok: every goatnet that declares the assay probe identity carries all of it")
    return 0


if __name__ == "__main__":
    sys.exit(main())
