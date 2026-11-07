#!/usr/bin/env python3
"""Fail if a tracked Go module has no blocking vulnerability scan or exemption."""
from collections import Counter
from pathlib import Path
import subprocess
import sys

import yaml

# Tracked Go modules deliberately left out of the go-vuln matrix, each with
# its reason. An entry naming a module this repository does not track fails,
# so the list holds only modules that are here.
EXEMPTIONS: dict[str, str] = {}


def blocking(node):
    return "if" not in node and str(node.get("continue-on-error", "false")).lower() == "false"


def scans(workflow, directory):
    for job in workflow.get("jobs", {}).values():
        if not blocking(job):
            continue
        default_dir = job.get("defaults", {}).get("run", {}).get("working-directory")
        for step in job.get("steps", []):
            if not blocking(step) or step.get("working-directory", default_dir) != directory:
                continue
            # Accept the ordinary blocking command, not comments, echoes,
            # conditionals or commands that mask its return code.
            lines = [line.strip() for line in str(step.get("run", "")).splitlines()
                     if line.strip() and not line.lstrip().startswith("#")]
            allowed = {"govulncheck ./...",
                       "go install golang.org/x/vuln/cmd/govulncheck@latest",
                       "set -e", "set -euo pipefail"}
            if (lines and lines[-1] == "govulncheck ./..."
                    and lines.count("govulncheck ./...") == 1 and set(lines) <= allowed):
                return True
    return False


def validate(modules, ci, signal):
    job = ci["jobs"]["go-vuln"]
    matrix = job["strategy"]["matrix"]["module"]
    counts = Counter(matrix + ["signal-server"] + list(EXEMPTIONS))
    errors = []
    if job["strategy"]["matrix"].get("exclude"):
        errors.append("go-vuln matrix exclusions can leave declared modules unscanned")
    for module in sorted(modules | counts.keys()):
        if module not in modules:
            errors.append(f"declared module has no tracked go.mod: {module}")
        elif counts[module] != 1:
            errors.append(f"module needs exactly one scan or exemption: {module} ({counts[module]})")
    if not blocking(job) or not scans({"jobs": {"go-vuln": job}}, "${{ matrix.module }}"):
        errors.append("go-vuln matrix has no unconditional blocking govulncheck ./... step")
    if not scans(signal, "signal-server"):
        errors.append("signal-server has no unconditional blocking govulncheck ./... step")
    return errors


def main():
    root = Path(subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"], text=True).strip())
    files = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
    mods = {str(Path(f).parent) for f in files if Path(f).name == "go.mod"}
    for module in mods:
        if not (root / module / "go.mod").is_file():
            raise ValueError(f"missing tracked input: {module}/go.mod")
    workflows = []
    for name in ("ci.yml", "signal-server.yml"):
        with (root / ".github/workflows" / name).open() as source:
            workflows.append(yaml.load(source, Loader=yaml.BaseLoader))
    errors = validate(mods, *workflows)
    for error in errors:
        print(f"FAIL: {error}", file=sys.stderr)
    if errors:
        return 1
    print(f"Go vulnerability coverage: all {len(mods)} tracked modules classified; scan steps verified")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError, yaml.YAMLError,
            subprocess.CalledProcessError) as error:
        print(f"FAIL: cannot verify Go vulnerability coverage: {error}", file=sys.stderr)
        sys.exit(2)
