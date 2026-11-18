#!/usr/bin/env python3
"""Exercise the production readiness task with deterministic Docker states."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


SCRIPT_DIR = Path(__file__).resolve().parent
ROLE_DIR = SCRIPT_DIR.parent
ROLES_DIR = ROLE_DIR.parent
DOCKER_DIR = SCRIPT_DIR / "fixtures"
DOCKER_STANDIN = DOCKER_DIR / "docker"
TASK_NAME = "ory-narrow-repset | wait for kratos + hydra containers healthy"
KRATOS = "kratos-test"
HYDRA = "hydra-test"
ORIGINAL_FORMAT = "{{.State.Health.Status}}"
CORRECTED_FORMAT = (
    "{{.State.Status}} "
    "{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}"
)
EXPECTED_FORMATS = {
    "original": ORIGINAL_FORMAT,
    "corrected": CORRECTED_FORMAT,
}
HEALTHY = {"status": "running", "health": "healthy"}


SCENARIOS = [
    {
        "name": "both-running-healthy",
        "states": {KRATOS: [HEALTHY], HYDRA: [HEALTHY]},
        "success": True,
        "calls": [KRATOS, HYDRA],
    },
    {
        "name": "first-unhealthy-second-healthy",
        "states": {
            KRATOS: [{"status": "running", "health": "unhealthy"}],
            HYDRA: [HEALTHY],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{KRATOS}: running unhealthy"],
    },
    {
        "name": "first-healthy-second-unhealthy",
        "states": {
            KRATOS: [HEALTHY],
            HYDRA: [{"status": "running", "health": "unhealthy"}],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{HYDRA}: running unhealthy"],
    },
    {
        "name": "starting",
        "states": {
            KRATOS: [HEALTHY],
            HYDRA: [{"status": "running", "health": "starting"}],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{HYDRA}: running starting"],
    },
    {
        "name": "stopped-with-stale-healthy",
        "states": {
            KRATOS: [{"status": "exited", "health": "healthy"}],
            HYDRA: [HEALTHY],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{KRATOS}: exited healthy"],
    },
    {
        "name": "synthetic-paused-with-healthy-field",
        "states": {
            KRATOS: [{"status": "paused", "health": "healthy"}],
            HYDRA: [HEALTHY],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{KRATOS}: paused healthy"],
    },
    {
        "name": "synthetic-restarting-with-healthy-field",
        "states": {
            KRATOS: [HEALTHY],
            HYDRA: [{"status": "restarting", "health": "healthy"}],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{HYDRA}: restarting healthy"],
    },
    {
        "name": "running-without-healthcheck",
        "states": {
            KRATOS: [HEALTHY],
            HYDRA: [{"status": "running", "health": None}],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{HYDRA}: running none"],
    },
    {
        "name": "missing-container",
        "states": {
            KRATOS: [
                {"rc": 1, "stderr": f"Error: No such object: {KRATOS}"}
            ],
            HYDRA: [HEALTHY],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [
            f"{KRATOS}: inspection error (rc=1): Error: No such object: {KRATOS}"
        ],
    },
    {
        "name": "docker-daemon-inspection-error",
        "states": {
            KRATOS: [HEALTHY],
            HYDRA: [
                {
                    "rc": 1,
                    "stderr": (
                        "Cannot connect to the Docker daemon at "
                        "unix:///var/run/docker.sock. Is the docker daemon running?"
                    ),
                }
            ],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [
            f"{HYDRA}: inspection error (rc=1): Cannot connect to the Docker "
            "daemon at unix:///var/run/docker.sock. Is the docker daemon running?"
        ],
    },
    {
        "name": "synthetic-inspection-exit-125",
        "states": {
            KRATOS: [HEALTHY],
            HYDRA: [{"rc": 125, "stderr": "synthetic inspection failure"}],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [
            f"{HYDRA}: inspection error (rc=125): synthetic inspection failure"
        ],
    },
    {
        "name": "both-failing",
        "states": {
            KRATOS: [
                {"rc": 1, "stderr": f"Error: No such object: {KRATOS}"}
            ],
            HYDRA: [
                {
                    "rc": 1,
                    "stderr": (
                        "Cannot connect to the Docker daemon at "
                        "unix:///var/run/docker.sock. Is the docker daemon running?"
                    ),
                }
            ],
        },
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [
            f"{KRATOS}: inspection error (rc=1): Error: No such object: {KRATOS}",
            f"{HYDRA}: inspection error (rc=1): Cannot connect to the Docker "
            "daemon at unix:///var/run/docker.sock. Is the docker daemon running?",
        ],
    },
    {
        "name": "unhealthy-to-healthy-retry",
        "states": {
            KRATOS: [
                {"status": "running", "health": "unhealthy"},
                HEALTHY,
            ],
            HYDRA: [HEALTHY, HEALTHY],
        },
        "wait_seconds": 10,
        "success": True,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA],
        "retried": True,
    },
    {
        "name": "persistent-failure-exhaustion",
        "states": {
            KRATOS: [HEALTHY, HEALTHY],
            HYDRA: [
                {"status": "running", "health": "starting"},
                {"status": "running", "health": "starting"},
            ],
        },
        "wait_seconds": 10,
        "success": False,
        "calls": [KRATOS, HYDRA, KRATOS, HYDRA, KRATOS, HYDRA],
        "diagnostics": [f"{HYDRA}: running starting"],
        "retried": True,
    },
]


def calls_from(state_dir):
    calls_path = state_dir / "calls.jsonl"
    if not calls_path.exists():
        return []
    return [
        json.loads(line)
        for line in calls_path.read_text(encoding="utf-8").splitlines()
    ]


def run_scenario(ansible_playbook, scenario):
    with tempfile.TemporaryDirectory(prefix="ory-health-") as temporary:
        temporary_path = Path(temporary)
        scenario_path = temporary_path / "scenario.json"
        state_dir = temporary_path / "state"
        scenario_path.write_text(
            json.dumps(scenario["states"], sort_keys=True), encoding="utf-8"
        )

        env = os.environ.copy()
        env.update(
            {
                "ANSIBLE_NOCOLOR": "1",
                "ANSIBLE_ROLES_PATH": str(ROLES_DIR),
                "ORY_HEALTH_TEST_SCENARIO": str(scenario_path),
                "ORY_HEALTH_TEST_STATE_DIR": str(state_dir),
                "PATH": f"{DOCKER_DIR}{os.pathsep}{env['PATH']}",
                "ZAGROS_REPLICATOR_PASS": "fixture-only-password",
            }
        )
        command = [
            ansible_playbook,
            "-i",
            "localhost,",
            "--extra-vars",
            json.dumps(
                {
                    "ory_spock_preflight_wait_seconds": scenario.get(
                        "wait_seconds", 5
                    )
                }
            ),
            str(SCRIPT_DIR / "container-health.yml"),
        ]
        completed = subprocess.run(
            command,
            cwd=ROLE_DIR,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
        return completed, calls_from(state_dir), command


def check_scenario(completed, calls, scenario, expected_format):
    errors = []
    assertions = 0
    output = completed.stdout
    expected_success = scenario["success"]
    assertions += 1
    if (completed.returncode == 0) != expected_success:
        errors.append(
            f"expected {'success' if expected_success else 'task failure'}, "
            f"got rc={completed.returncode}"
        )
    assertions += 1
    if TASK_NAME not in output:
        errors.append("production readiness task did not execute")
    assertions += 1
    if "unreachable=1" in output or "UNREACHABLE!" in output:
        errors.append("fixture host was unreachable")
    containers = [call["container"] for call in calls]
    formats = [call["format"] for call in calls]
    assertions += 1
    if containers != scenario["calls"]:
        errors.append(
            f"inspect calls were {containers!r}, expected {scenario['calls']!r}"
        )
    assertions += 1
    if formats != [expected_format] * len(calls):
        errors.append(
            f"inspect formats were {formats!r}, expected only {expected_format!r}"
        )
    for diagnostic in scenario.get("diagnostics", []):
        assertions += 1
        if diagnostic not in output:
            errors.append(f"missing diagnostic: {diagnostic!r}")
    retry_line = f"FAILED - RETRYING: [localhost]: {TASK_NAME}"
    if scenario.get("retried"):
        assertions += 1
        if retry_line not in output:
            errors.append("expected retry was not observed")
    return assertions, errors


def check_standin_format_contract():
    assertions = 0
    errors = []
    with tempfile.TemporaryDirectory(prefix="ory-health-format-") as temporary:
        temporary_path = Path(temporary)
        scenario_path = temporary_path / "scenario.json"
        scenario_path.write_text(
            json.dumps({KRATOS: [HEALTHY]}, sort_keys=True), encoding="utf-8"
        )
        for name, inspect_format, expected_rc, expected_stdout in (
            ("original", ORIGINAL_FORMAT, 0, "healthy\n"),
            ("corrected", CORRECTED_FORMAT, 0, "running healthy\n"),
            ("unknown", "{{.State.Status}}", 2, ""),
        ):
            state_dir = temporary_path / name
            env = os.environ.copy()
            env.update(
                {
                    "ORY_HEALTH_TEST_SCENARIO": str(scenario_path),
                    "ORY_HEALTH_TEST_STATE_DIR": str(state_dir),
                }
            )
            completed = subprocess.run(
                [str(DOCKER_STANDIN), "inspect", "--format", inspect_format, KRATOS],
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            assertions += 1
            if completed.returncode != expected_rc:
                errors.append(
                    f"stand-in {name} format returned {completed.returncode}, "
                    f"expected {expected_rc}"
                )
            assertions += 1
            if completed.stdout != expected_stdout:
                errors.append(
                    f"stand-in {name} stdout was {completed.stdout!r}, "
                    f"expected {expected_stdout!r}"
                )
    return assertions, errors


def main():
    ansible_playbook = shutil.which("ansible-playbook")
    if not ansible_playbook:
        print("ERROR: ansible-playbook is not available", file=sys.stderr)
        return 2

    expected_format_name = os.environ.get(
        "ORY_HEALTH_EXPECT_FORMAT", "corrected"
    )
    if expected_format_name not in EXPECTED_FORMATS:
        print(
            "ERROR: ORY_HEALTH_EXPECT_FORMAT must be original or corrected",
            file=sys.stderr,
        )
        return 2
    expected_format = EXPECTED_FORMATS[expected_format_name]

    format_assertions, format_errors = check_standin_format_contract()
    print(f"stand-in format contract: {'PASS' if not format_errors else 'FAIL'}")
    for error in format_errors:
        print(f"ASSERTION FAILED: {error}")

    failures = 0
    assertion_total = format_assertions
    assertion_failures = len(format_errors)
    print(f"ansible-playbook={ansible_playbook}")
    print(f"expected_inspect_format={expected_format!r}")
    for scenario in SCENARIOS:
        completed, calls, command = run_scenario(ansible_playbook, scenario)
        assertions, errors = check_scenario(
            completed, calls, scenario, expected_format
        )
        verdict = "PASS" if not errors else "FAIL"
        print(f"\n=== {scenario['name']}: {verdict} ===")
        print(f"command={' '.join(command)}")
        print(f"returncode={completed.returncode}")
        print(f"inspect_calls={[call['container'] for call in calls]!r}")
        print(f"inspect_formats={[call['format'] for call in calls]!r}")
        print(completed.stdout.rstrip())
        for error in errors:
            print(f"ASSERTION FAILED: {error}")
        failures += bool(errors)
        assertion_total += assertions
        assertion_failures += len(errors)

    passed = len(SCENARIOS) - failures
    print(f"\ncontainer-health scenarios: passed={passed} failed={failures}")
    print(
        "container-health assertions: "
        f"passed={assertion_total - assertion_failures} "
        f"failed={assertion_failures} total={assertion_total}"
    )
    return 1 if failures or format_errors else 0


if __name__ == "__main__":
    sys.exit(main())
