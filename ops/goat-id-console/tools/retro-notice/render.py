#!/usr/bin/env python3
"""
Retroactive-notice renderer — the reference implementation of the general
onboarding-email formula (docs/design/onboarding-email-formula.md). Emails an
already-onboarded person about something new, in the fixed 4-part shape:

  1. ACKNOWLEDGEMENT  (prior state, role-customized)
  2. RECAP            (what they already have, role-driven)
  3. WHAT'S NEW       (the reason block -- the ONLY campaign-specific input)
  4. WHY IT MATTERS   (short, role-aware line)

Initial onboarding and permission upgrades are the same formula with a different
prior-state + reason; this tool is the batch driver for the notice case.

DETERMINISTIC: identical (roster row, notice) -> identical bytes. No randomness,
no time-of-render values in the body.

Usage:
  render.py <roster.tsv> <out.json> <notice.json>
    roster.tsv  : name<TAB>email<TAB>groups-csv<TAB>whitelisted  (groups from
                  Kratos metadata_public.groups; e.g. "PLATFORM-ADMIN,IAM-ADMIN" or "(none)")
    notice.json : the reason block -- see notice.relay-health.json for the shape.
  Emits [{name,email,role,deliverable,subject,body}, ...] as JSON.

PII (roster, rendered bodies) is caller-supplied and MUST NOT be committed.
Sending is out of scope here -- pipe the output to SES v2 (send_manifest pattern).
"""
import json, sys

GOATNET = "ZAGROS"

# role -> (reads-as label, recap surfaces). Highest-privilege wins; "" surfaces
# means "no distinct surfaces beyond the base" (mesh + sign-on).
ROLES = {
    "PLATFORM-ADMIN":   ("a platform administrator", "the admin console, the NetBird dashboard, the portal, and the observability surfaces"),
    "IAM-ADMIN":        ("an identity & access administrator", "the admin console, identity administration, and the access surfaces"),
    "COMPLIANCE":       ("a compliance reviewer", "the compliance dashboard"),
    "OBS-ADMIN":        ("an observability administrator", "the Grafana + observability surfaces"),
    "GRAFANA-ADMIN":    ("a Grafana administrator", "the Grafana dashboards"),
    "GRAFANA-READONLY": ("an observability viewer", "read access to the Grafana dashboards"),
    "OBS-READONLY":     ("an observability viewer", "read access to the observability surfaces"),
    "DEVELOPER":        ("a developer", "the mesh, the data fabric, and the developer surfaces"),
}
ADMIN_GROUPS = {"PLATFORM-ADMIN", "IAM-ADMIN", "OBS-ADMIN", "GRAFANA-ADMIN", "COMPLIANCE"}


def role_bits(groups):
    gs = [g for g in groups if g and g != "(none)"]
    label, surfaces = ("an onboarded participant", "")
    for k in ROLES:                    # deterministic: ROLES order = privilege order
        if k in gs:
            label, surfaces = ROLES[k]
            break
    else:
        if any(g.startswith("data.") for g in gs):
            label, surfaces = ("a data-access grantee", "your granted data views")
    return label, surfaces, any(g in ADMIN_GROUPS for g in gs)


def render(name, email, groups, notice):
    label, surfaces, admin = role_bits(groups)
    first = (name.split() or ["there"])[0] if name and name != "—" else "there"
    L = [f"Hello {first},", ""]
    L.append(f"You're already an active member of the {GOATNET} goatnet as {label}. "
             "Nothing below asks you to re-onboard — this is a heads-up about something new you now have.")
    L += ["", "WHERE YOU STAND"]
    L.append(f"  You already have your mesh access and your sign-on, plus {surfaces}." if surfaces
             else "  You already have your mesh access and your sign-on.")
    L += ["", "WHAT'S NEW", f"  {notice['headline']}", f"  {notice['what']}",
          f"  Open it:  {notice['url']}", f"  {notice['howto']}"]
    L += ["", "WHY IT MATTERS", f"  {notice['why']}"]
    if admin and notice.get("why_admin"):
        L.append(f"  {notice['why_admin']}")
    L += ["", f"— the {GOATNET} team"]
    return notice["subject"], "\n".join(L)


def deliverable(email):
    return not any(email.endswith(s) for s in (".test", ".example"))


def main():
    roster, out, notice_path = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(notice_path) as source_file:
        notice = json.load(source_file)
    res = []
    with open(roster) as source_file:
        for ln in source_file:
            p = ln.rstrip("\n").split("\t")
            if len(p) < 3:
                continue
            name, email, role = p[0], p[1], p[2]
            groups = role.split(",") if role and role != "(none)" else []
            subj, body = render(name, email, groups, notice)
            res.append({"name": name, "email": email, "role": role,
                        "deliverable": deliverable(email), "subject": subj, "body": body})
    with open(out, "w") as source_file:
        json.dump(res, source_file, ensure_ascii=False, indent=0)
    dv = sum(1 for r in res if r["deliverable"])
    print(f"rendered={len(res)} deliverable={dv} undeliverable={len(res)-dv}")
    for r in res:
        if not r["deliverable"]:
            print("  UNDELIVERABLE:", r["email"])


if __name__ == "__main__":
    main()
