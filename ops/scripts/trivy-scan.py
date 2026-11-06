#!/usr/bin/env python3
"""Run Trivy against the checkout, the curated image set, and the fleet pins.

Three lanes, labeled in every summary:
  - the local repository (filesystem scan),
  - the head of each curated image line (trivy-images.txt, :latest),
  - the deployed set: the immutable digests ops/pins/fleet-pins.yaml pins,
    which is what the fleet actually runs. The head lane answers "is the
    newest build clean"; the pin lane answers "is what is deployed clean".
"""

from __future__ import annotations

import argparse
import html
import json
import os
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


SEVERITIES = ("CRITICAL", "HIGH", "MEDIUM", "LOW", "UNKNOWN")
ACCEPTED_RISK_ISSUE = "https://github.com/usareur-af/goatcore/issues/1482"
ACCEPTED_RISK_NOTE = (
    "The risk has been accepted for this version. Do not update as it may break the application."
)
# Keep these annotations scoped to the advisory, pinned version, and component
# covered by the existing decision. A later advisory or different component
# must remain visible as an ordinary finding.
ACCEPTED_RISK_FINDINGS = {
    ("GHSA-65p9-r9h6-22vj", "aws-lc-fips-sys", "0.13.11", "ops/zenoh/pqc-image/fips-probe/Cargo.lock"),
    ("GHSA-9f94-5g5w-gf6r", "aws-lc-fips-sys", "0.13.11", "ops/zenoh/pqc-image/fips-probe/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "crates/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/consistency/rust/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/federation/republisher/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/flashlight/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/goat-topology/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/goat-trashgoat/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/grant-crypto/keyd/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/panoscope/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/serve/trust-sandbox/orchestrator/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-ingress-gateway/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-shims/rest-pub-bridge/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/audit-subscriber/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/crypto-posture/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/fabric-crawler/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/fabric-health/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/loadgen/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/mesh-health/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/node-net-health/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/ntp-health-pub/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/peering-health/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/presence-health/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/surface-visit-audit/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/synth-vendor/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "ops/zenoh-trial-peers/time-health/Cargo.lock"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "usr/local/bin/goat-audit-subscriber"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "usr/local/bin/goat-mesh-health"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "app/panoscope"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "usr/local/bin/goat-consistency-console"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "usr/local/bin/goat-node-net-health"),
    ("CVE-2026-32829", "lz4_flex", "0.10.0", "usr/local/bin/goat-consistency-health"),
    ("GHSA-2mjx-qc3c-rqvc", "rustls", "0.23.40", "ops/zenoh-trial-peers/surface-visit-audit/Cargo.lock"),
    ("CVE-2025-64702", "github.com/quic-go/quic-go", "v0.55.0", "ops/relay/wedge-repro/go.mod"),
    ("CVE-2026-40898", "github.com/quic-go/quic-go", "v0.55.0", "ops/relay/wedge-repro/go.mod"),
}
FINDING_SECTIONS = (
    ("Vulnerabilities", None),
    ("Misconfigurations", "FAIL"),
    ("Secrets", None),
)
# A curated image that cannot be pulled is a lifecycle state, not a scanner
# fault: the registry answers DENIED or not-found for a package that was never
# published, is not public, or was deleted. Report it as a warning so a red
# run always means the scanner itself or a report failed. A network or
# registry outage does not match these markers and stays a failure.
UNPULLABLE_MARKERS = (
    "unable to find the specified image",
    "manifest unknown",
    "name unknown",
    "denied",
    "unauthorized",
)


def image_is_unpullable(output: str) -> bool:
    text = output.lower()
    return any(marker in text for marker in UNPULLABLE_MARKERS)


def warn_unpullable(label: str, lane: str) -> None:
    if lane == "pin":
        message = (
            f"{label} is not pullable: fleet-pins.yaml deploys this digest, but "
            "this registry does not serve it. Either the pin was minted against "
            "another registry instance, or the package is not public. A fleet "
            "host cannot re-pull this image from here."
        )
    else:
        message = (
            f"{label} is not pullable: it has never been published, or its registry "
            "package is not public. Its image workflow publishes it on a qualifying "
            "push to main; once the package exists and is public, this scan includes "
            "it automatically."
        )
    print(f"  {message}", file=sys.stderr)
    if os.environ.get("GITHUB_ACTIONS") == "true":
        print(f"::warning title=Curated image not pullable::{message}")


# fleet-pins.yaml is machine-written and every `fleet:` value is one reference
# on one line, so a line scan needs no YAML dependency on the scan runner. The
# {{ ghcr_org }} template renders from GHCR_ORG, defaulting to the org the
# curated list names.
GHCR_ORG_DEFAULT = "usareur-af"
# The value itself carries spaces while templated ({{ ghcr_org }}), so capture
# the whole line value and validate after rendering.
FLEET_PIN_RE = re.compile(r"^\s*fleet:\s*(.+?)\s*$", re.MULTILINE)


def load_fleet_pins(path: Path) -> list[str]:
    if not path.is_file():
        return []
    org = os.environ.get("GHCR_ORG", GHCR_ORG_DEFAULT)
    refs: list[str] = []
    for raw in FLEET_PIN_RE.findall(path.read_text(encoding="utf-8")):
        # A component with no fleet image carries `fleet: null`. Left in, Trivy
        # would ask Docker Hub for an image called "null" and report it denied.
        if raw.strip("\"'") in ("null", "~", ""):
            continue
        ref = re.sub(r"\{\{\s*ghcr_org\s*\}\}", org, raw.strip("\"'"))
        if re.search(r"\s", ref):
            # A variable this scan does not render; a garbage ref would reach
            # Trivy as a parse error wearing the pin's name.
            print(f"Skipping fleet pin with an unrendered variable: {raw}", file=sys.stderr)
            continue
        if ref not in refs:
            refs.append(ref)
    return refs


def pin_label(ref: str) -> str:
    name, _, digest = ref.partition("@")
    if digest:
        return f"{name}@{digest[:19]} (deployed pin)"
    return f"{ref} (deployed pin)"


def load_targets(path: Path) -> list[str]:
    return [
        line.strip()
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


def scan(command: list[str]) -> tuple[int, str]:
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return result.returncode, result.stdout


def read_report(report_path: Path, label: str) -> tuple[dict[str, int], list[dict[str, str]]]:
    report = json.loads(report_path.read_text(encoding="utf-8"))
    counts = {severity: 0 for severity in SEVERITIES}
    counts.update({"TOTAL": 0})
    findings: list[dict[str, str]] = []

    for result in report.get("Results") or []:
        for section, required_status in FINDING_SECTIONS:
            for finding in result.get(section) or []:
                if required_status and finding.get("Status", "FAIL").upper() != required_status:
                    continue
                severity = str(finding.get("Severity", "UNKNOWN")).upper()
                if severity not in counts:
                    severity = "UNKNOWN"
                counts[severity] += 1
                counts["TOTAL"] += 1
                cause = finding.get("CauseMetadata") or {}
                if not isinstance(cause, dict):
                    cause = {}
                location = str(finding.get("Target") or result.get("Target") or label)
                line = finding.get("StartLine") or cause.get("StartLine")
                if line:
                    location = f"{location}:{line}"
                # Use an explicit field allowlist: Trivy secret results can
                # include matched source text under fields such as Code/Match.
                findings.append(
                    {
                        "source": label,
                        "severity": severity,
                        "target": location,
                        "kind": {
                            "Vulnerabilities": "Vulnerability",
                            "Misconfigurations": "Misconfiguration",
                            "Secrets": "Secret",
                        }[section],
                        "id": str(
                            finding.get("VulnerabilityID")
                            or finding.get("AVDID")
                            or finding.get("ID")
                            or finding.get("RuleID")
                            or ""
                        ),
                        "package": str(finding.get("PkgName") or ""),
                        "installed": str(finding.get("InstalledVersion") or ""),
                        "fixed": str(finding.get("FixedVersion") or ""),
                        "summary": str(finding.get("Title") or finding.get("Category") or ""),
                        "accepted_risk": "",
                    }
                )
                finding_record = findings[-1]
                accepted_key = (
                    finding_record["id"],
                    finding_record["package"],
                    finding_record["installed"],
                    finding_record["target"].split(":", 1)[0],
                )
                if accepted_key in ACCEPTED_RISK_FINDINGS:
                    finding_record["accepted_risk"] = ACCEPTED_RISK_NOTE

    return counts, findings


def print_table(rows: list[tuple[str, dict[str, int] | None, str]]) -> None:
    columns = ("TARGET", "CRITICAL", "HIGH", "MEDIUM", "LOW", "UNKNOWN", "TOTAL", "STATUS")
    formatted = []
    for target, counts, status in rows:
        formatted.append(
            (
                target,
                *(str(counts[key]) if counts is not None else "-" for key in SEVERITIES),
                str(counts["TOTAL"]) if counts is not None else "-",
                status,
            )
        )

    widths = [max(len(columns[i]), *(len(row[i]) for row in formatted)) for i in range(len(columns))]
    print("  ".join(name.ljust(widths[i]) for i, name in enumerate(columns)))
    print("  ".join("-" * width for width in widths))
    for row in formatted:
        print("  ".join(value.ljust(widths[i]) for i, value in enumerate(row)))


def write_html_summary(
    path: Path,
    rows: list[tuple[str, dict[str, int] | None, str]],
    findings: list[dict[str, str]],
) -> None:
    severity_counts = {
        severity: sum(item["severity"] == severity for item in findings)
        for severity in SEVERITIES
    }
    filters = "\n".join(
        f'<label><input type="checkbox" name="severity" value="{severity}" checked> '
        f'{severity.title()} (<span id="severity-count-{severity}">{severity_counts[severity]}</span>)</label>'
        for severity in SEVERITIES
    )
    sources = [label for label, _counts, _status in rows]
    source_options = "\n".join(
        f'<option value="{html.escape(source, quote=True)}">{html.escape(source)}</option>'
        for source in sources
    )
    findings_by_source: dict[str, list[dict[str, str]]] = {source: [] for source in sources}
    for item in findings:
        findings_by_source.setdefault(item["source"], []).append(item)

    def render_finding_rows(source: str, status: str) -> str:
        source_findings = sorted(
            findings_by_source.get(source, []),
            key=lambda item: SEVERITIES.index(item["severity"]),
        )
        if not source_findings:
            if status == "SCANNED":
                message = "No findings were reported for this target."
            elif status == "NOT PULLABLE":
                message = (
                    "This image is not pullable (never published, or its package "
                    "is not public), so there was nothing to scan."
                )
            else:
                message = "No findings are available because this scan did not complete."
            return f'<tr><td colspan="9">{html.escape(message)}</td></tr>'
        return "\n".join(
            '<tr class="finding-row" data-severity="'
            + html.escape(item["severity"], quote=True)
            + '" data-source="'
            + html.escape(item["source"], quote=True)
            + '" data-pinned="'
            + ("true" if item["accepted_risk"] else "false")
            + '"><td>'
            + html.escape(item["severity"])
            + "</td><td>"
            + html.escape(item["source"])
            + "</td><td>"
            + html.escape(item["target"])
            + "</td><td>"
            + html.escape(item["kind"])
            + "</td><td>"
            + html.escape(item["id"])
            + "</td><td>"
            + html.escape(item["package"])
            + "</td><td>"
            + html.escape(item["installed"])
            + "</td><td>"
            + html.escape(item["fixed"])
            + "</td><td"
            + (' class="accepted-risk"' if item["accepted_risk"] else "")
            + ">"
            + html.escape(item["summary"])
            + (
                '<div class="accepted-risk-note">'
                + html.escape(item["accepted_risk"])
                + ' <a href="'
                + html.escape(ACCEPTED_RISK_ISSUE, quote=True)
                + '">Issue #1482</a></div>'
                if item["accepted_risk"]
                else ""
            )
            + "</td></tr>"
            for item in source_findings
        )

    summary_rows: list[str] = []
    for index, (label, counts, status) in enumerate(rows):
        source = html.escape(label, quote=True)
        target_id = f"target-findings-{index}"
        values = [str(counts[key]) if counts is not None else "-" for key in SEVERITIES]
        total = str(counts["TOTAL"]) if counts is not None else "-"
        summary_rows.append(
            '<tr class="target-row" data-source="'
            + source
            + '"><td><button class="target-toggle" type="button" aria-expanded="false" aria-controls="'
            + target_id
            + '">'
            + html.escape(label)
            + '</button></td><td>'
            + "</td><td>".join(values)
            + "</td><td>"
            + total
            + "</td><td>"
            + html.escape(status)
            + "</td></tr><tr class=\"target-details\" data-source=\""
            + source
            + '" hidden><td colspan="8" id="'
            + target_id
            + '"><div class="table-wrap"><table class="finding-table"><thead><tr>'
            + "".join(
                f"<th>{heading}</th>"
                for heading in (
                    "Severity",
                    "Image / scan",
                    "Target / location",
                    "Type",
                    "Finding ID",
                    "Package",
                    "Installed",
                    "Fixed",
                    "Summary",
                )
            )
            + "</tr></thead><tbody>"
            + render_finding_rows(label, status)
            + "</tbody></table></div></td></tr>"
        )
    summary_table_rows = "\n".join(summary_rows)

    document = f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
  <title>Trivy scan summary</title>
  <style>
    :root {{ color-scheme: light dark; font: 16px/1.45 system-ui, sans-serif; }}
    body {{ margin: 1rem auto; padding: 0 .5rem; max-width: 1800px; }}
    h1, h2 {{ line-height: 1.2; }}
    .accepted-risk {{ background: #fff2b2; color: #302600; }}
    .accepted-risk-note {{ margin-top: .4rem; padding: .45rem; background: #ffe27a; border-radius: .2rem; }}
    .filters {{ display: flex; flex-wrap: wrap; gap: .5rem 1.25rem; margin: 1rem 0; }}
    label {{ white-space: nowrap; }}
    .table-wrap {{ overflow-x: auto; }}
    table {{ border-collapse: collapse; width: 100%; }}
    th, td {{ border: 1px solid #8886; padding: .45rem .6rem; text-align: left; vertical-align: top; }}
    th {{ position: sticky; top: 0; background: Canvas; }}
    td {{ overflow-wrap: anywhere; }}
    .target-toggle {{ border: 0; padding: 0; color: inherit; background: transparent; font: inherit; text-align: left; cursor: pointer; }}
    .target-row {{ cursor: pointer; }}
    .target-row:hover {{ background: #8882; }}
    .target-details > td {{ padding: .5rem; }}
    .finding-table {{ min-width: 1100px; table-layout: fixed; }}
    .finding-table th:first-child, .finding-table td:first-child {{ width: 8rem; white-space: nowrap; }}
    .finding-table th:nth-child(8), .finding-table td:nth-child(8) {{ width: 7rem; }}
    .target-row[hidden], .target-details[hidden], .finding-row[hidden] {{ display: none !important; }}
    #findings-count {{ font-weight: 600; }}
  </style>
</head>
<body>
  <h1>Trivy scan summary</h1>
  <div class="filters">
    <label><input type="checkbox" id="severity-all" checked> All</label>
    {filters}
    <label><input type="checkbox" id="pinned-dependencies"> Pinned Dependencies</label>
  </div>
  <label for="source-filter">Target:</label>
  <select id="source-filter">
    <option value="">All targets</option>
    {source_options}
  </select>
  <p id="findings-count" aria-live="polite"></p>
  <p>Select a target row to expand its findings. Findings are ordered by severity.</p>
  <div class="table-wrap"><table id="scan-summary">
    <thead><tr><th>Target</th><th>Critical</th><th>High</th><th>Medium</th><th>Low</th><th>Unknown</th><th>Total</th><th>Status</th></tr></thead>
    <tbody>{summary_table_rows}</tbody>
  </table></div>
  <script>
    const controls = [...document.querySelectorAll('input[name="severity"]')];
    const allControl = document.getElementById('severity-all');
    const pinnedControl = document.getElementById('pinned-dependencies');
    const sourceFilter = document.getElementById('source-filter');
    const findingRows = [...document.querySelectorAll('.finding-row')];
    const targetRows = [...document.querySelectorAll('.target-row')];
    const detailRows = [...document.querySelectorAll('.target-details')];
    const count = document.getElementById('findings-count');
    function filterFindings() {{
      const selected = new Set(controls.filter(control => control.checked).map(control => control.value));
      const sourceCounts = new Map(controls.map(control => [control.value, 0]));
      let visible = 0;
      for (const row of targetRows) {{
        row.hidden = Boolean(sourceFilter.value && row.dataset.source !== sourceFilter.value);
      }}
      for (const row of detailRows) {{
        const toggle = document.querySelector(`.target-toggle[aria-controls="${{row.cells[0].id}}"]`);
        const sourceMatches = !sourceFilter.value || row.dataset.source === sourceFilter.value;
        row.hidden = !sourceMatches || !toggle || toggle.getAttribute('aria-expanded') !== 'true';
      }}
      for (const row of findingRows) {{
        const sourceMatches = !sourceFilter.value || row.dataset.source === sourceFilter.value;
        const pinnedMatches = pinnedControl.checked || row.dataset.pinned !== 'true';
        if (sourceMatches && pinnedMatches) {{
          sourceCounts.set(row.dataset.severity, sourceCounts.get(row.dataset.severity) + 1);
        }}
        const severityMatches = selected.has(row.dataset.severity);
        row.hidden = !(severityMatches && sourceMatches && pinnedMatches);
        if (!row.hidden) visible += 1;
      }}
      for (const control of controls) {{
        document.getElementById('severity-count-' + control.value).textContent = sourceCounts.get(control.value);
      }}
      const selectedCount = controls.filter(control => control.checked).length;
      allControl.checked = selectedCount === controls.length;
      allControl.indeterminate = selectedCount > 0 && selectedCount < controls.length;
      count.textContent = `Matching findings: ${{visible}} of ${{findingRows.length}}`;
    }}
    for (const row of targetRows) {{
      row.addEventListener('click', () => {{
        const toggle = row.querySelector('.target-toggle');
        toggle.setAttribute('aria-expanded', String(toggle.getAttribute('aria-expanded') !== 'true'));
        filterFindings();
      }});
    }}
    for (const control of controls) control.addEventListener('change', filterFindings);
    allControl.addEventListener('change', () => {{
      controls.forEach(control => control.checked = allControl.checked);
      filterFindings();
    }});
    sourceFilter.addEventListener('change', filterFindings);
    pinnedControl.addEventListener('change', filterFindings);
    filterFindings();
  </script>
</body>
</html>
"""
    path.write_text(document, encoding="utf-8")


def append_github_summary(
    rows: list[tuple[str, dict[str, int] | None, str]],
    report_dir: Path,
    failures: int,
    unpullable: list[str],
) -> None:
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        return

    with Path(summary_path).open("a", encoding="utf-8") as summary:
        summary.write("## Trivy advisory scan\n\n")
        summary.write(
            "Advisory only: findings and scan errors do not fail this workflow or block a pull request.\n\n"
        )
        summary.write(
            "| Target | Critical | High | Medium | Low | Unknown | Total | Status |\n"
        )
        summary.write("| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n")
        for target, counts, status in rows:
            values = [str(counts[key]) if counts is not None else "-" for key in SEVERITIES]
            total = str(counts["TOTAL"]) if counts is not None else "-"
            summary.write(
                f"| {target} | {' | '.join(values)} | {total} | {status} |\n"
            )
        summary.write(
            "\nCounts are findings reported by Trivy, not deduplicated across targets. "
            "Rows marked `(deployed pin)` are the immutable digests `ops/pins/fleet-pins.yaml` "
            "deploys; unmarked image rows are the head of each image line (`:latest`).\n\n"
        )
        summary.write(
            f"HTML summary and JSON reports are included in the `trivy-reports` workflow artifact (`{report_dir.name}`).\n"
        )
        if unpullable:
            summary.write("\n**Curated images not pullable, reported as warnings:**\n\n")
            for label in unpullable:
                summary.write(f"- `{label}`\n")
            summary.write(
                "\nEach of these has never been published, or its registry package is "
                "not public. Its image workflow publishes it on a qualifying push to "
                "main, and this scan includes it automatically once it is public. A "
                "missing curated image is a warning, not a failure, so a failed run "
                "here always means the scanner itself or a report failed.\n"
            )
        if failures:
            summary.write(
                f"\n**Incomplete scan:** {failures} target(s) failed. An error is not a clean scan.\n"
            )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Scan the local goat checkout and the curated set of published images."
    )
    parser.add_argument(
        "--platform",
        default="linux/amd64",
        help="image platform to scan (default: linux/amd64); run again with linux/arm64 for that architecture",
    )
    parser.add_argument(
        "--no-secrets",
        action="store_true",
        help="skip secret scanning (useful when reports will be uploaded as CI artifacts)",
    )
    args = parser.parse_args()

    trivy = shutil.which("trivy")
    if not trivy:
        parser.error("Trivy was not found on PATH; install it using docs/operations/trivy-local-scan.md")

    repo_root = Path(__file__).resolve().parents[2]
    manifest = Path(__file__).with_name("trivy-images.txt")
    targets = load_targets(manifest)
    if not targets:
        parser.error(f"no image targets found in {manifest}")

    report_root = repo_root / ".envrc.local.d" / "trivy-reports"
    report_root.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now(timezone.utc).strftime("%d-%m-%y-%H-%M-%S")
    report_dir = report_root / f"trivy-run-{timestamp}"
    collision = 2
    while True:
        try:
            report_dir.mkdir(mode=0o700)
            break
        except FileExistsError:
            report_dir = report_root / f"trivy-run-{timestamp}-{collision}"
            collision += 1
    pins = load_fleet_pins(repo_root / "ops" / "pins" / "fleet-pins.yaml")
    print(
        f"Scanning checkout, {len(targets)} curated images (head of line) and "
        f"{len(pins)} deployed pins (platform: {args.platform})"
    )
    print(f"JSON reports: {report_dir}")

    rows: list[tuple[str, dict[str, int] | None, str]] = []
    html_findings: list[dict[str, str]] = []
    failures = 0
    unpullable: list[str] = []
    scan_plan = [("Local Repository Scan", "fs", str(repo_root), "fs")] + [
        (image, "image", image, "head") for image in targets
    ]
    curated = {target for _label, _type, target, _lane in scan_plan}
    scan_plan.extend(
        (pin_label(ref), "image", ref, "pin") for ref in pins if ref not in curated
    )

    for number, (label, target_type, target, lane) in enumerate(scan_plan, start=1):
        print(f"[{number}/{len(scan_plan)}] {label}", flush=True)
        report_path = report_dir / f"{number:02d}-{target_type}.json"
        command = [trivy, target_type]
        if target_type == "fs":
            scanners = "vuln,misconfig" if args.no_secrets else "vuln,misconfig,secret"
            command.extend(("--scanners", scanners))
        else:
            scanners = "vuln" if args.no_secrets else "vuln,secret"
            command.extend(("--scanners", scanners, "--platform", args.platform))
        command.extend(("--format", "json", "--output", str(report_path), target))

        returncode, output = scan(command)
        if returncode != 0:
            if target_type == "image" and image_is_unpullable(output):
                unpullable.append(label)
                warn_unpullable(label, lane)
                rows.append((label, None, "NOT PULLABLE"))
                continue
            failures += 1
            print(f"  Scan failed (exit {returncode}):\n{output.rstrip()}", file=sys.stderr)
            rows.append((label, None, f"ERROR ({returncode})"))
            continue

        try:
            counts, target_findings = read_report(report_path, label)
        except (OSError, json.JSONDecodeError, TypeError, ValueError) as error:
            failures += 1
            print(f"  Could not read Trivy report {report_path}: {error}", file=sys.stderr)
            rows.append((label, None, "REPORT ERROR"))
            continue

        rows.append((label, counts, "SCANNED"))
        html_findings.extend(target_findings)

    html_path = report_dir / "summary.html"
    try:
        write_html_summary(html_path, rows, html_findings)
    except OSError as error:
        failures += 1
        print(f"Could not write HTML summary {html_path}: {error}", file=sys.stderr)

    print("\nTrivy findings summary")
    print_table(rows)
    print("\nCounts are findings reported by Trivy, not deduplicated across targets.")
    print(f"Detailed JSON reports are in: {report_dir}")
    print(f"HTML summary: {html_path}")
    append_github_summary(rows, report_dir, failures, unpullable)
    if failures:
        print(f"{failures} scan(s) failed; an error is not a clean scan.", file=sys.stderr)
        return 1
    if unpullable:
        print(
            f"{len(unpullable)} curated image(s) not pullable (never published or not "
            "public); reported as warnings, not failures.",
            file=sys.stderr,
        )
    print("All requested scans completed. Review findings and reports before drawing conclusions.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
