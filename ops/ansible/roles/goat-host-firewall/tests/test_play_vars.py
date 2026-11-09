# SPDX-License-Identifier: Apache-2.0
"""What the peering plays hand the role: the core's spire-bundle rule goes only on the SPIRE
server host and is admitted on the mesh interface, never from the edge's ansible_host
(which may be an underlay address)."""
import pathlib

import yaml

ANSIBLE = pathlib.Path(__file__).resolve().parents[3]


def _include_vars(play, profile):
    doc = yaml.safe_load((ANSIBLE / play).read_text())
    for task in doc[0]["post_tasks"]:
        v = task.get("vars") or {}
        if v.get("goat_host_firewall_profile") == profile:
            return v
    raise AssertionError(f"{play}: no {profile} include")


def test_core_spire_bundle_only_on_the_spire_server_host():
    v = _include_vars("peering-republisher.yml", "core")
    assert "peering_spire_server_host" in v["goat_host_firewall_spire_bundle_port"]
    assert "inventory_hostname" in v["goat_host_firewall_spire_bundle_port"]


def test_core_spire_bundle_is_not_sourced_from_ansible_host():
    v = _include_vars("peering-republisher.yml", "core")
    assert "ansible_host" not in str(v.get("goat_host_firewall_spire_bundle_sources", ""))
