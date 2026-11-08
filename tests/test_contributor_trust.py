"""Tests for .github/contributor-trust/check.py (ADR 1221)."""
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("contributor_trust", ROOT / ".github/contributor-trust/check.py")
ct = importlib.util.module_from_spec(_spec)
sys.modules["contributor_trust"] = ct  # dataclasses look the module up by name
_spec.loader.exec_module(ct)

PATTERNS = ct.load_patterns((ROOT / ".github/contributor-trust/sensitive-paths.txt").read_text())


def facts(**kw):
    base = dict(author="someone", association="NONE", registered=False, merged_prs=0,
                changed=["docs/readme-fix.md"], label_present=False)
    base.update(kw)
    return ct.Facts(**base)


def test_glob_semantics():
    rx = ct.glob_to_regex
    assert rx(".github/**").match(".github/workflows/ci.yml")
    assert rx("go.sum").match("ops/snitch/go.sum")
    assert rx("go.sum").match("go.sum")
    assert rx("ops/goat-zenoh-portal/internal/api/auth*").match("ops/goat-zenoh-portal/internal/api/auth.go")
    assert not rx("ops/goat-zenoh-portal/internal/api/auth*").match("ops/goat-zenoh-portal/internal/api/x/auth.go")
    assert rx("ops/snitch/**/auth*").match("ops/snitch/internal/api/auth.go")
    assert not rx("ops/pins/**").match("ops/pinsx/a")


def test_sensitive_list_covers_what_it_says():
    hit = ct.sensitive_matches([
        ".github/workflows/goat-relay.yml",
        "goat-relay/src/routing.rs",
        "ops/goat-zenoh-portal/internal/access/dataroles.go",
        "ops/snitch/Dockerfile",
        "docs/design/netbird-relay-wire-protocol.md",
    ], PATTERNS)
    assert "docs/design/netbird-relay-wire-protocol.md" not in hit
    assert len(hit) == 4


def test_maintainer_and_automation_pass_without_review():
    for f in (facts(association="MEMBER", changed=[".github/workflows/x.yml"]),
              facts(author="dependabot[bot]", changed=["Dockerfile"])):
        v = ct.decide(f, PATTERNS)
        assert v.passed and not v.needs_review


def test_unknown_needs_review_even_for_docs():
    v = ct.decide(facts(), PATTERNS)
    assert v.tier == "unknown" and v.needs_review and not v.passed


def test_roster_not_configured_reads_as_unknown_not_registered():
    tier, why = ct.tier_of(facts(registered=None))
    assert tier == "unknown"
    assert any("could not be checked" in w for w in why)


def test_known_by_registration_or_prior_merge():
    assert ct.tier_of(facts(registered=True))[0] == "known"
    assert ct.tier_of(facts(merged_prs=2))[0] == "known"


def test_known_passes_outside_sensitive_paths_and_needs_review_inside():
    assert ct.decide(facts(merged_prs=1), PATTERNS).passed
    v = ct.decide(facts(merged_prs=1, changed=["goat-relay/src/routing.rs"]), PATTERNS)
    assert v.needs_review and not v.passed and v.sensitive == ["goat-relay/src/routing.rs"]


def test_review_label_counts_only_from_an_owner():
    base = dict(changed=["goat-relay/src/routing.rs"], label_present=True, label_actor="x")
    assert ct.decide(facts(**base, label_actor_permission="admin"), PATTERNS).passed
    assert ct.decide(facts(**base, label_actor_permission="maintain"), PATTERNS).passed
    assert not ct.decide(facts(**base, label_actor_permission="write"), PATTERNS).passed
    assert not ct.decide(facts(**base, label_actor_permission=None), PATTERNS).passed


def test_render_names_tier_and_paths():
    f = facts(changed=["goat-relay/src/routing.rs"])
    text = ct.render(ct.decide(f, PATTERNS), f)
    assert "REVIEW NEEDED" in text and "unknown" in text and "goat-relay/src/routing.rs" in text


def test_owner_with_private_membership_is_maintainer_by_permission():
    for perm in ("admin", "maintain", "write"):
        f = facts(association="CONTRIBUTOR", repo_permission=perm, changed=[".github/workflows/x.yml"])
        assert ct.tier_of(f)[0] == "maintainer" and ct.decide(f, PATTERNS).passed
    assert ct.tier_of(facts(association="CONTRIBUTOR", repo_permission="read"))[0] == "unknown"
