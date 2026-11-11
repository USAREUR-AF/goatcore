#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Which ci.yml jobs a pull request's changed files can affect.

Reads changed paths (one per line) on stdin and decides, per job, whether
any of them is an input that job reads. The map follows what each job's
tests read, not only what it builds, as docs/design/ci-results-follow-inputs.md
asks: declare broadly, by directory.

REPORT ONLY. ci.yml prints this plan to the job summary; no job is skipped
by it yet. A week of plans against real results shows whether the map is
complete before any job is gated on it.

Fail open: a changed file that no area claims, any change under .github/,
or a change to this classifier marks every job as needed.

  ci_areas.py            print one `job=true|false` line per job
  ci_areas.py --summary  print a Markdown table for $GITHUB_STEP_SUMMARY
"""
import fnmatch
import sys

# Jobs that read the whole tree today, or that must run on every pull
# request whatever it changes. Never gated:
#   no-identifiers  any file can carry an identifier
#   bats-bash32     tests/build.bats scans every tracked and untracked file
#                   for credentials; it can be gated only after that scan
#                   moves to an always-on job
ALWAYS = ("no-identifiers", "bats-bash32")

GO_MODULE_DIRS = (
    "signal-server", "ops/admit-audit", "ops/enrollment", "ops/federation/reference",
    "ops/goat-id-console", "ops/goat-zenoh-portal", "ops/grant-crypto",
    "ops/mgmt-host-bootstrap", "ops/onboard-runner", "ops/relay-probe",
    "ops/signal-federation-probe", "ops/smoketest", "ops/snitch", "ops/time-hygiene",
    "ops/wg-cp0-mesh-reach", "ops/zenoh/loadgen",
)

# fnmatch's `*` crosses `/`, so `dir/*` is everything below dir.
AREAS = {
    "rust": [
        "Cargo.toml", "Cargo.lock", "rust-toolchain*", "goat-relay/*", "goat-relay-protocol/*",
        "crates/*", "ops/grant-crypto/sdk/core/*", "ops/grant-crypto/keyd/*", "ops/errors/*",
        "ops/warrant/*", "ops/identity/*", "ops/goatbuilder/*", "ops/federation/vectors/*",
        "ops/federation/standup/*", "ops/ansible/*", "ops/scripts/*", "ops/pins/*",
        "ops/release/*", "ops/validation/*", "ops/consistency/*", "ops/secrets/*",
        "ops/enrollment/*", "docs/reference/goatbuilder/*", "tests/fixtures/configs/*",
    ],
    "go": [
        "*.go", "*go.mod", "*go.sum", "go.work*",
        *(d + "/*" for d in GO_MODULE_DIRS),
        "ops/federation/vectors/*", "ops/identity/*", "ops/permissions/*", "ops/lifecycle/*",
        "docs/operators/*", "docs/operations/zenoh-trial-onboarding.md",
    ],
    "go-vuln": ["*.go", "*go.mod", "*go.sum", "go.work*"],
    # compileall reads every .py, the go.mod coverage guard every go.mod and
    # bash-portability all of ops/, so this job's inputs stay wide until those
    # whole-tree checks move to an always-on job.
    "python-and-ansible": [
        "*.py", "*go.mod", "ops/*", "tests/*", "terraform/modules/*",
        "crates/goat-federation/src/*", "crates/goatbuilder/src/federate/*",
    ],
    "peering-forwarder": [
        "ops/ci/test_peering*", "ops/ci/test-peering*", "ops/ci/test_validate_partners_*",
        "ops/ci/check-peering-forwarder-holds-nothing.py", "ops/ci/fixtures/peering-forwarder-*",
        "ops/ci/test-host-firewall-role.sh", "ops/federation/validate-partners.py",
        "ops/ansible/peering-forwarder.yml", "ops/ansible/peering-carrier-keys.yml",
        "ops/ansible/roles/goat-peering-forwarder/*", "ops/ansible/roles/goat-peering-wg/*",
        "ops/ansible/roles/goat-host-firewall/*", "ops/ansible/ansible.cfg",
        "ops/ansible/filter_plugins/*", "ops/ansible/group_vars/*",
    ],
    "peering-host-firewall": [
        "ops/ansible/roles/goat-host-firewall/*", "ops/ansible/peering-republisher.yml",
        "ops/ci/test-host-firewall-role.sh",
    ],
}
JOBS = (*AREAS, *ALWAYS)

FAIL_OPEN = (".github/*", "ops/ci/ci_areas.py")
# Read by tests even though they are docs: never treated as documentation.
DOCS_READ_BY_TESTS = ("docs/reference/*", "docs/operators/*", "docs/operations/zenoh-trial-onboarding.md")
DOC_EXT = ("md", "png", "jpg", "jpeg", "gif", "svg", "webp")


def _match(path, patterns):
    return any(fnmatch.fnmatchcase(path, p) for p in patterns)


def is_doc(path):
    """The same rule as the `changes` job's `code` flag."""
    if _match(path, DOCS_READ_BY_TESTS):
        return False
    ext = path.rsplit(".", 1)[-1].lower() if "." in path else ""
    if path.startswith("docs/") and ext in DOC_EXT:
        return True
    return "/" not in path and ext == "md"


def plan(paths):
    """Return ({job: bool}, reason) for the changed paths."""
    paths = [p.strip() for p in paths if p.strip()]
    needed = {job: False for job in JOBS}
    for job in ALWAYS:
        needed[job] = True
    for path in paths:
        if _match(path, FAIL_OPEN):
            return {job: True for job in JOBS}, f"fail open: {path} changes CI itself"
    unclaimed = []
    for path in paths:
        if is_doc(path):
            continue
        hits = [job for job, pats in AREAS.items() if _match(path, pats)]
        if not hits:
            unclaimed.append(path)
        for job in hits:
            needed[job] = True
    if unclaimed:
        return {job: True for job in JOBS}, f"fail open: no area claims {unclaimed[0]}"
    return needed, "by area"


def main(argv):
    needed, reason = plan(sys.stdin.read().splitlines())
    if argv[1:] == ["--summary"]:
        skip = [job for job, need in needed.items() if not need]
        print("### Per-area plan (report only, nothing is skipped)\n")
        print(f"Decision: {reason}. Would skip: {', '.join(skip) if skip else 'nothing'}.\n")
        print("| Job | Needed |\n| --- | --- |")
        for job, need in needed.items():
            print(f"| `{job}` | {'yes' if need else 'no'} |")
    else:
        for job, need in needed.items():
            print(f"{job}={'true' if need else 'false'}")
        print(f"reason={reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
