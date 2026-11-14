"""Tests for the ADR 1124 scope-qualified reference resolver
(ops/ansible/filter_plugins/acl_scope.py).

Run: python3 -m pytest -q tests/ansible_filter_plugins/test_acl_scope.py

What it locks. Each of these is a constraint ADR 1124 decided, and a
regression in any of them silently widens a cross-scope grant:

  - bare suffixes still resolve in the rule's own scope (no behaviour change),
  - HOST: resolves via host_site; a concrete scope name resolves via the map,
  - an unresolvable HOST / unknown scope FAILS rather than falling back local,
  - bidirectional + qualified is refused (the reverse-grant hole),
  - qualified on both sides is refused,
  - UPWARD destinations are limited to INFRA and must be port-scoped,
  - DOWNWARD sources are limited to PLATFORM-ADMIN and MAY be any-port,
  - PLATFORM-ADMIN -> HOST:INFRA (a site admin administering the goatnet
    substrate) is refused: the boundary the tactical-site scope exists to hold,
  - template rules may use HOST: but never a concrete scope name.
"""

import importlib.util
from pathlib import Path

import pytest

FILTERS = Path(__file__).resolve().parents[2] / "ops" / "ansible" / "filter_plugins"
_SPEC = importlib.util.spec_from_file_location("acl_scope", FILTERS / "acl_scope.py")
acl_scope = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(acl_scope)

AclScopeError = acl_scope.AclScopeError
resolve_ref = acl_scope.resolve_ref
resolve_refs = acl_scope.resolve_refs
rule_uses_qualified = acl_scope.rule_uses_qualified
validate_rule = acl_scope.validate_rule

SCOPES = {"zagros-prod": "ZAGROS-PROD", "site-one": "SITE_ONE", "zagros": "ZAGROS"}


def upward(**kw):
    rule = {
        "name": "gear -> host-site infra",
        "sources": ["KID"],
        "destinations": ["HOST:INFRA"],
        "protocol": "tcp",
        "ports": ["7447"],
        "bidirectional": False,
    }
    rule.update(kw)
    return rule


def downward(**kw):
    rule = {
        "name": "host-site platform-admin -> gear",
        "sources": ["HOST:PLATFORM-ADMIN"],
        "destinations": ["KID"],
        "protocol": "all",
        "bidirectional": False,
    }
    rule.update(kw)
    return rule


# --- resolution ------------------------------------------------------------


def test_a_bare_suffix_resolves_in_its_own_scope():
    assert resolve_ref("INFRA", "SITE_ONE") == "SITE_ONE-INFRA"


def test_host_resolves_via_the_host_prefix():
    assert resolve_ref("HOST:INFRA", "SITE_ONE", "ZAGROS-PROD") == "ZAGROS-PROD-INFRA"


def test_a_concrete_scope_name_resolves_via_the_scope_map():
    assert resolve_ref("zagros-prod:INFRA", "SITE_ONE", None, SCOPES) == "ZAGROS-PROD-INFRA"


def test_mixed_bare_and_qualified_lists_resolve_elementwise():
    assert resolve_refs(["KID", "HOST:INFRA"], "SITE_ONE", "ZAGROS-PROD") == [
        "SITE_ONE-KID",
        "ZAGROS-PROD-INFRA",
    ]


def test_an_empty_list_resolves_to_an_empty_list():
    assert resolve_refs([], "SITE_ONE") == []


# Fail closed: never silently resolve to the local scope.
@pytest.mark.parametrize(
    "call",
    [
        pytest.param(lambda: resolve_ref("HOST:INFRA", "SITE_ONE"), id="host-with-no-host-prefix"),
        pytest.param(lambda: resolve_ref("nosuchscope:INFRA", "SITE_ONE", None, SCOPES), id="undeclared-scope-name"),
        pytest.param(lambda: resolve_ref("HOST:", "SITE_ONE", "ZAGROS-PROD"), id="empty-suffix"),
    ],
)
def test_an_unresolvable_reference_raises(call):
    with pytest.raises(AclScopeError):
        call()


# --- detection -------------------------------------------------------------


def test_a_qualified_destination_is_detected():
    assert rule_uses_qualified({"sources": ["KID"], "destinations": ["HOST:INFRA"]})


def test_an_all_bare_rule_is_not_flagged_as_qualified():
    assert not rule_uses_qualified({"sources": ["KID"], "destinations": ["INFRA"]})


# --- the rules that must pass ----------------------------------------------


@pytest.mark.parametrize(
    "rule,template",
    [
        pytest.param(upward(), False, id="canonical-upward"),
        pytest.param(upward(protocol="icmp", ports=None), False, id="icmp-upward-needs-no-ports"),
        pytest.param(downward(), False, id="canonical-downward-admin-protocol-all"),
        pytest.param(
            {"name": "intra", "sources": ["P2P"], "destinations": ["KID"], "protocol": "all"},
            False,
            id="unqualified-rule-untouched",
        ),
        pytest.param(upward(), True, id="host-legal-inside-a-template"),
    ],
)
def test_a_legal_rule_validates(rule, template):
    assert validate_rule(rule, template=template) == []


# --- the rules that must fail ----------------------------------------------


@pytest.mark.parametrize(
    "rule,template",
    [
        pytest.param(upward(bidirectional=True), False, id="bidirectional-and-qualified"),
        pytest.param(upward(destinations=["HOST:PLATFORM-ADMIN"]), False, id="upward-destination-outside-infra"),
        pytest.param(upward(destinations=["HOST:BACKEND-INFRA"]), False, id="hidden-backend-tier-of-another-scope"),
        pytest.param(upward(protocol="all", ports=None), False, id="protocol-all-upward"),
        pytest.param(upward(ports=None), False, id="upward-tcp-with-no-ports"),
        pytest.param(downward(sources=["HOST:INFRA"]), False, id="infra-as-a-qualified-source"),
        pytest.param(
            {
                "name": "both",
                "sources": ["HOST:PLATFORM-ADMIN"],
                "destinations": ["HOST:INFRA"],
                "protocol": "tcp",
                "ports": ["443"],
                "bidirectional": False,
            },
            False,
            id="qualified-on-both-sides",
        ),
        pytest.param(upward(destinations=["zagros-prod:INFRA"]), True, id="concrete-scope-name-in-a-template"),
    ],
)
def test_an_illegal_rule_is_refused(rule, template):
    assert validate_rule(rule, template=template)


def test_a_site_admin_reaching_the_host_substrate_on_any_port_is_refused():
    # THE boundary case, stated as its own test because it is the one the
    # tactical scope exists to hold: a site admin must never gain any-port
    # administrative reach to the goatnet substrate.
    site_admin_up = {
        "name": "platform-admin -> host-site infra (any port)",
        "sources": ["PLATFORM-ADMIN"],
        "destinations": ["HOST:INFRA"],
        "protocol": "all",
        "bidirectional": False,
    }
    assert validate_rule(site_admin_up)
