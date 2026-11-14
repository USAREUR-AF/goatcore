#!/usr/bin/env python3
"""row-list.py — which rows reach the status page, and what the page is told
about each one (#1518).

The role builds one row list (templates/isitup-rows.json.j2) and renders both
services.env (what check.sh probes) and data/meta.json (lifecycle, source,
owner) from it. This pins the rules that decide the list:

  - one row per id, first home wins: isitup_services, then the off-mesh
    catalog, then external-service declarations;
  - `listed: false` (and a declaration's `status.list` / per-hostname
    `status.hostnames.<h>.list`) keeps a row off the page and out of the probe;
  - a row past its `retire_after` date is dropped, whatever `enabled` says;
  - `lifecycle` and `owner` reach meta.json, and every external declaration
    row is `source: external`, so the page can move them to /external;
  - a hostname can carry its own probe path;
  - meta.json carries the stamp of the list itself (`rendered`: digest, row
    count, source commits, time), and the digest follows the rows.

Needs python3 + jinja2; skips without jinja2, like free-text-columns.sh.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
try:
    from render_services_env import RENDERED_AT, SOURCE, render, rows_sha256
except ImportError:
    print("[isitup/row-list] SKIP (python3 jinja2 not importable)")
    sys.exit(0)

TODAY = "2026-09-27"

CONTEXT = {
    "site_id": "s",
    "site_dns_zone": "zone.invalid",
    "isitup_services": [
        {"id": "portal", "name": "Portal", "fqdn": "portal.s.zone.invalid",
         "container": "portal", "port": 8090, "category": "platform"},
        # Same id as a catalog row below: this one must win, the other vanish.
        {"id": "collab-home", "name": "Collab (site copy)", "fqdn": "collab.example",
         "type": "external", "category": "external", "access": "offmesh"},
        {"id": "newthing", "name": "New thing", "fqdn": "newthing.s.zone.invalid",
         "container": "newthing", "port": 80, "lifecycle": "planned"},
        {"id": "oldthing", "name": "Old thing", "fqdn": "oldthing.s.zone.invalid",
         "container": "oldthing", "port": 80, "lifecycle": "retiring",
         "retire_after": "2026-10-31"},
        {"id": "hidden", "name": "Hidden", "fqdn": "hidden.s.zone.invalid",
         "container": "hidden", "port": 80, "listed": False},
        {"id": "gone", "name": "Gone", "fqdn": "gone.s.zone.invalid",
         "container": "gone", "port": 80, "retire_after": "2026-09-26"},
        # Hidden here, and present again in the catalog below: must stay hidden.
        {"id": "collab-chat-hidden", "name": "Chat (hidden)", "fqdn": "hiddenchat.collab.example",
         "type": "external", "category": "external", "listed": False},
        # Hidden here, and the same hostname as a declaration below: the
        # declaration must not bring it back.
        {"id": "boardroom", "name": "Board", "fqdn": "board.s.zone.invalid",
         "type": "external", "listed": False},
    ],
    "isitup_external_services": [
        {"id": "collab-home", "name": "Collab (catalog copy)", "fqdn": "collab.example",
         "type": "external", "category": "external", "access": "offmesh"},
        {"id": "collab-chat", "name": "Chat", "fqdn": "chat.collab.example",
         "type": "external", "category": "external", "access": "offmesh"},
        {"id": "collab-chat-hidden", "name": "Chat (catalog copy)", "fqdn": "hiddenchat.collab.example",
         "type": "external", "category": "external", "access": "offmesh"},
    ],
    "external_service_platform": {
        "name": "platform",
        "hostnames": ["console.platform", "api.platform", "heartbeat.platform", "keycloak.platform"],
        "health_path": "/health/liveness",
        "owner": "Platform Co (partner)",
        "retire_after": "2026-12-31",
        "status": {
            "name": "Partner platform",
            "category": "exercise",
            "hostnames": {
                "api.platform": {"list": False},
                "heartbeat.platform": {"health_path": "/health", "lifecycle": "planned"},
            },
        },
        "enabled": True,
    },
    "external_service_quiet": {
        "name": "quiet",
        "owner": "someone",
        "status": {"list": False},
        "enabled": True,
    },
    "external_service_expired": {
        "name": "expired",
        "owner": "someone",
        "retire_after": "2026-09-26",
        "enabled": True,
    },
    "external_service_off": {"name": "off", "owner": "someone", "enabled": False},
    "external_service_board": {"name": "board", "owner": "someone", "enabled": True},
}

fail = 0


def check(ok, msg):
    global fail
    print("[isitup/row-list] %s  %s" % ("PASS" if ok else "FAIL", msg))
    fail |= (not ok)


rows, services_env, meta = render(CONTEXT, TODAY)
ids = [r["id"] for r in rows]
by_id = {r["id"]: r for r in rows}
env_ids = [line.strip().strip('"').split("|")[0]
           for line in services_env.splitlines() if line.startswith('  "')]

check(len(ids) == len(set(ids)), "every row id is unique: %s" % ids)
check(by_id.get("collab-home", {}).get("name") == "Collab (site copy)",
      "an id on isitup_services wins over the same id in the catalog")
check("collab-chat" in ids, "a catalog row with its own id is kept")
check("hidden" not in ids, "listed: false keeps a row off the page")
check("gone" not in ids, "a row past retire_after is dropped")
check("oldthing" in ids, "a row before retire_after is kept")
check(by_id.get("newthing", {}).get("lifecycle") == "planned", "lifecycle: planned is carried")
check(by_id.get("oldthing", {}).get("lifecycle") == "retiring", "lifecycle: retiring is carried")
check(by_id.get("portal", {}).get("lifecycle") == "live", "lifecycle defaults to live")
check(by_id.get("portal", {}).get("source") == "core", "an on-mesh row is source: core")
check(by_id.get("collab-chat", {}).get("source") == "offmesh", "a category: external row is source: offmesh")

check("platform" in ids, "a declaration's front door is listed")
check("platform-api" not in ids, "status.hostnames.<h>.list: false drops that hostname")
check("platform-keycloak" in ids, "an unmentioned hostname follows the declaration default")
hb = by_id.get("platform-heartbeat", {})
check(hb.get("health_path") == "/health", "a hostname can override the probe path")
check(by_id.get("platform-keycloak", {}).get("health_path") == "/health/liveness",
      "other hostnames keep the declaration's probe path")
check(hb.get("lifecycle") == "planned", "a hostname can carry its own lifecycle")
check(all(by_id[i]["source"] == "external" for i in ids if i.startswith("platform")),
      "every declaration row is source: external")
check(by_id.get("platform", {}).get("owner") == "Platform Co (partner)", "the owner is carried")
check("quiet" not in ids, "status.list: false drops every hostname of a declaration")
check("expired" not in ids, "a declaration past retire_after is not listed")
check("off" not in ids, "an enabled: false declaration is not listed")
check("collab-chat-hidden" not in ids,
      "a row hidden on isitup_services is not brought back by the catalog's copy")
check("board" not in ids and "boardroom" not in ids,
      "a hidden row's hostname is not brought back by a declaration")

check(env_ids == ids, "services.env probes exactly the row list, in order")
check(sorted(meta.get("rows", {})) == sorted(ids), "meta.json describes exactly the row list")
check(meta["rows"].get("platform", {}).get("retire_after") == "2026-12-31",
      "meta.json carries retire_after for the page")

# The stamp of the list itself: what the page shows and isitup-list.py compares.
stamp = meta.get("rendered", {})
check(stamp.get("sha256") == rows_sha256(rows), "meta.json carries the digest of the row list")
check(stamp.get("rows") == len(rows), "meta.json carries the row count")
check(stamp.get("source") == SOURCE and stamp.get("at") == RENDERED_AT,
      "meta.json carries the source commits and the render time")
check(len(stamp.get("sha256", "")) == 64, "the digest is a sha256")

# Same declaration, same digest; a removed row, a different one. A page that
# still carries the old list is then told apart from the declaration.
rows_again, _, _ = render(CONTEXT, TODAY)
check(rows_sha256(rows_again) == rows_sha256(rows), "two renders of one declaration give one digest")
without = dict(CONTEXT, isitup_services=[s for s in CONTEXT["isitup_services"] if s["id"] != "portal"])
rows_without, _, meta_without = render(without, TODAY)
check(meta_without["rendered"]["sha256"] != stamp.get("sha256"),
      "removing a row from the declaration changes the digest")
# A retire_after date that passes drops the row, so the digest moves with it.
rows_later, _, _ = render(CONTEXT, "2026-11-01")
check("oldthing" not in [r["id"] for r in rows_later] and rows_sha256(rows_later) != rows_sha256(rows),
      "a row passing its retire_after date changes the digest")

sys.exit(fail)
