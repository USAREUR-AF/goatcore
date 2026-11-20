#!/usr/bin/env python3
"""check-alertmanager-inhibits-name-alerts: every alert an inhibit rule names exists.

An Alertmanager inhibit rule that names an alert no Prometheus rule defines does
nothing, and nothing says so. Three such rules suppressed SpockSubscriptionLag in
favour of other alerts long after that alert stopped being able to fire, and they
kept the removed alert looking deliberate.

What it reads:
  - the Alertmanager configs: ops/compose/mgmt-host/monitoring/alertmanager.yml,
    and the observability and obs-host roles' templates/alertmanager.yml.j2,
    rendered as check-alertmanager-renders.py renders them (ansible's
    trim_blocks, its `bool` and `regex_escape` filters stubbed);
  - the alert rules: every `- alert: <name>` in the rule files under
    ops/compose/ and ops/ansible/roles/ (their files/ and templates/), the
    sandbox ops/monitoring/ and every tests/ directory excluded.

What it asserts: every `alertname` in an inhibit rule's source_matchers and
target_matchers (`alertname = "X"`, `alertname="X"`, and the older source_match /
target_match maps) names an alert some rule defines. A regex matcher (`=~`) must
match at least one defined alert. A negative matcher (`!=`, `!~`) is not checked.

  check-alertmanager-inhibits-name-alerts.py              check this repository
  check-alertmanager-inhibits-name-alerts.py --self-test  plant a missing name and
                                                          prove it is refused
Exit 0 clean, 1 a missing alert, 2 a source could not be read or rendered.
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONFIGS = [
    "ops/compose/mgmt-host/monitoring/alertmanager.yml",
    "ops/ansible/roles/observability/templates/alertmanager.yml.j2",
    "ops/ansible/roles/obs-host/templates/alertmanager.yml.j2",
]
RULE_ROOTS = ["ops/compose", "ops/ansible/roles"]
ALERT = re.compile(r"^\s*-\s*alert:\s*['\"]?([A-Za-z_:][A-Za-z0-9_:]*)['\"]?\s*$", re.M)
MATCHER = re.compile(r"""^\s*alertname\s*(=~|!~|!=|=)\s*"?([^"]*?)"?\s*$""")


class Unreadable(Exception):
    pass


def rule_files(repo: Path):
    for root in RULE_ROOTS:
        base = repo / root
        if not base.is_dir():
            continue
        for p in sorted(base.rglob("*")):
            rel = p.relative_to(repo).as_posix()
            if not p.is_file() or p.suffix not in (".yml", ".yaml", ".j2"):
                continue
            if "/tests/" in rel or rel.startswith("ops/monitoring/"):
                continue
            if root == "ops/ansible/roles" and "/files/" not in rel and "/templates/" not in rel:
                continue
            yield p


def defined_alerts(repo: Path) -> set[str]:
    names = set()
    for p in rule_files(repo):
        try:
            names.update(ALERT.findall(p.read_text(encoding="utf-8")))
        except (OSError, UnicodeDecodeError):
            continue
    return names


def render(repo: Path, rel: str) -> dict:
    import yaml
    path = repo / rel
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as e:
        raise Unreadable(f"{rel}: {e}")
    if rel.endswith(".j2"):
        from jinja2 import Environment, FileSystemLoader, select_autoescape
        # The output is YAML, never HTML: escaping would rewrite `<`, `&` and quotes in the
        # config. select_autoescape escapes only an .html/.htm/.xml template, so an .yml.j2
        # renders as ansible renders it.
        env = Environment(loader=FileSystemLoader(str(path.parent)), trim_blocks=True, lstrip_blocks=False,
                          autoescape=select_autoescape(enabled_extensions=("html", "htm", "xml"),
                                                       default_for_string=False, default=False))
        env.filters["bool"] = lambda v: str(v).lower() in ("true", "yes", "1", "on")
        env.filters["regex_escape"] = lambda s: s
        try:
            text = env.get_template(path.name).render(
                pagerduty_integration_key="TEST", deadman_webhook_url="http://example/",
                deadman_webhook_token="t", site_id="ci", obs_site="ci")
        except Exception as e:  # noqa: BLE001 - any render failure is the finding
            raise Unreadable(f"{rel}: does not render: {type(e).__name__}: {e}")
    try:
        return yaml.safe_load(text) or {}
    except Exception as e:  # noqa: BLE001
        raise Unreadable(f"{rel}: not YAML: {e}")


def inhibit_names(doc: dict):
    """(side, op, value) for every alertname matcher in the inhibit rules."""
    for i, rule in enumerate(doc.get("inhibit_rules") or []):
        for side in ("source", "target"):
            for m in rule.get(f"{side}_matchers") or []:
                hit = MATCHER.match(str(m))
                if hit:
                    yield i, side, hit.group(1), hit.group(2)
            old = rule.get(f"{side}_match") or {}
            if "alertname" in old:
                yield i, side, "=", str(old["alertname"])
            old_re = rule.get(f"{side}_match_re") or {}
            if "alertname" in old_re:
                yield i, side, "=~", str(old_re["alertname"])


def check(repo: Path, configs=CONFIGS) -> tuple[list[str], int]:
    alerts = defined_alerts(repo)
    if not alerts:
        raise Unreadable("no alert rules found under " + ", ".join(RULE_ROOTS))
    problems, seen = [], 0
    for rel in configs:
        for i, side, op, value in inhibit_names(render(repo, rel)):
            seen += 1
            if op == "=" and value not in alerts:
                problems.append(f"{rel}: inhibit rule {i + 1} {side} names alert {value!r}, which no rule defines")
            elif op == "=~" and not any(re.fullmatch(value, a) for a in alerts):
                problems.append(f"{rel}: inhibit rule {i + 1} {side} matches alert {value!r}, which no rule's name matches")
    return problems, seen


def self_test() -> int:
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp)
        (repo / "ops/compose/x/rules").mkdir(parents=True)
        (repo / "ops/compose/x/rules/r.yml").write_text(
            "groups:\n  - name: g\n    rules:\n      - alert: Present\n        expr: up == 0\n"
            "      - alert: AlsoPresent\n        expr: up == 0\n")
        (repo / "ops/monitoring/rules").mkdir(parents=True)
        (repo / "ops/monitoring/rules/sandbox.yml").write_text("      - alert: OnlyInSandbox\n")
        cfg = repo / "am.yml"

        def run(inhibits: str):
            cfg.write_text("route: {receiver: r}\nreceivers: [{name: r}]\ninhibit_rules:\n" + inhibits)
            return check(repo, ["am.yml"])

        probs, n = run('  - source_matchers: [\'alertname = "Present"\']\n'
                       '    target_matchers: [\'alertname="AlsoPresent"\']\n')
        if probs or n != 2:
            failures.append(f"two defined alerts passed: {probs} ({n} read)")
        probs, _ = run('  - source_matchers: [\'alertname = "Present"\']\n'
                       '    target_matchers: [\'alertname = "Missing"\']\n')
        if not (len(probs) == 1 and "'Missing'" in probs[0] and "target" in probs[0]):
            failures.append(f"a missing target was not refused: {probs}")
        probs, _ = run('  - source_match: {alertname: Gone}\n    target_matchers: [\'alertname = "Present"\']\n')
        if not (len(probs) == 1 and "'Gone'" in probs[0]):
            failures.append(f"a missing name in an old source_match was not refused: {probs}")
        probs, _ = run('  - source_matchers: [\'alertname = "OnlyInSandbox"\']\n'
                       '    target_matchers: [\'alertname = "Present"\']\n')
        if not (len(probs) == 1 and "'OnlyInSandbox'" in probs[0]):
            failures.append(f"a name defined only in the sandbox counted: {probs}")
        probs, _ = run('  - source_matchers: [\'alertname =~ "Also.*"\']\n'
                       '    target_matchers: [\'alertname =~ "Nope.*"\']\n')
        if not (len(probs) == 1 and "'Nope.*'" in probs[0]):
            failures.append(f"a regex matching no alert was not refused, or one matching was: {probs}")
        probs, _ = run('  - source_matchers: [\'alertname != "Whatever"\']\n'
                       '    target_matchers: [\'alertname = "Present"\']\n')
        if probs:
            failures.append(f"a negative matcher was checked: {probs}")
        # A .yml.j2 renders unescaped, as ansible renders it: `<`, `&` and quotes stay as written.
        (repo / "am.yml.j2").write_text(
            "route: {receiver: r}\nreceivers: [{name: r, note: \"{{ '<a & \\'b\\'>' }}\"}]\n"
            "inhibit_rules:\n  - source_matchers: ['alertname = \"Present\"']\n"
            "    target_matchers: ['alertname = \"AlsoPresent\"']\n")
        doc = render(repo, "am.yml.j2")
        if doc.get("receivers", [{}])[0].get("note") != "<a & 'b'>":
            failures.append(f"a .yml.j2 rendered escaped: {doc.get('receivers')}")
    for f in failures:
        print(f"  FAIL {f}")
    if failures:
        print(f"check-alertmanager-inhibits-name-alerts: self-test FAILED ({len(failures)})")
        return 1
    print("check-alertmanager-inhibits-name-alerts: self-test passed (7 cases)")
    return 0


def main(argv: list[str]) -> int:
    try:
        import yaml  # noqa: F401
        import jinja2  # noqa: F401
    except ImportError as e:
        print(f"check-alertmanager-inhibits-name-alerts: missing dependency: {e}", file=sys.stderr)
        return 2
    if argv[1:] == ["--self-test"]:
        return self_test()
    if argv[1:]:
        print(__doc__.strip().splitlines()[-4], file=sys.stderr)
        return 2
    try:
        problems, seen = check(REPO)
    except Unreadable as e:
        print(f"check-alertmanager-inhibits-name-alerts: {e}", file=sys.stderr)
        return 2
    if seen == 0:
        print("check-alertmanager-inhibits-name-alerts: no inhibit rule names an alert; refusing to report clean on an empty read",
              file=sys.stderr)
        return 2
    for p in problems:
        print(p)
    if problems:
        return 1
    print(f"check-alertmanager-inhibits-name-alerts: PASS, {seen} alertname matcher(s) in "
          f"{len(CONFIGS)} Alertmanager config(s) each name a defined alert")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
