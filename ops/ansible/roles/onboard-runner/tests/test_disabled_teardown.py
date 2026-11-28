#!/usr/bin/env python3
"""Exercise the production disabled-runner task against a disposable stack."""

import argparse
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import tempfile
from typing import Dict, List, Optional, Tuple


TEST_DIR = Path(__file__).resolve().parent
PLAYBOOK = TEST_DIR / "playbook.yml"
TAG = "onboard_runner_disabled_teardown"


class Runner:
    def __init__(self, log_dir: Optional[Path]):
        self.log_dir = log_dir
        self.sequence = 0

    def run(
        self,
        label: str,
        command: List[str],
        *,
        env: Dict[str, str],
        expected: int = 0,
        capture: Optional[Path] = None,
    ) -> subprocess.CompletedProcess:
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            env=env,
            timeout=180,
        )
        output = result.stdout + result.stderr
        if self.log_dir:
            self.sequence += 1
            self.log_dir.mkdir(parents=True, exist_ok=True)
            log = self.log_dir / f"{self.sequence:02d}-{label}.log"
            log.write_text(
                "$ " + shlex.join(command) + "\n"
                + f"exit={result.returncode}\n"
                + output
            )
            if capture is not None and capture.is_file():
                shutil.copyfile(capture, self.log_dir / f"{self.sequence:02d}-{capture.name}")
        if result.returncode != expected:
            raise AssertionError(
                f"{label}: expected exit {expected}, got {result.returncode}\n{output}"
            )
        return result


def require_command(name: str) -> str:
    command = shutil.which(name)
    if command is None:
        raise SystemExit(f"required command is unavailable: {name}")
    return command


def run_mocked_teardown(
    runner: Runner,
    command: List[str],
    callback_env: Dict[str, str],
    root: Path,
    case: str,
    *,
    expected: int,
) -> Tuple[subprocess.CompletedProcess, dict]:
    capture = root / f"{case}-task-result.jsonl"
    env = {
        **callback_env,
        "FAKE_DOCKER_RESULT": case,
        "TASK_RESULT_CAPTURE": str(capture),
    }
    result = runner.run(case, command, env=env, expected=expected, capture=capture)
    assert capture.is_file(), f"{case}: Ansible did not emit callback records"
    records = [json.loads(line) for line in capture.read_text().splitlines() if line.strip()]
    teardown_records = [
        record
        for record in records
        if "Bring the runner DOWN when disabled" in record["task"]
    ]
    assert len(teardown_records) == 1, records
    return result, teardown_records[0]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-dir", type=Path, help="preserve raw command output")
    parser.add_argument(
        "--require-prerequisites",
        action="store_true",
        help="fail instead of reporting a non-Linux platform skip",
    )
    args = parser.parse_args()

    if platform.system() != "Linux":
        message = "disabled teardown lifecycle requires a Linux Docker host"
        if args.require_prerequisites:
            raise SystemExit("required prerequisite is unavailable: " + message)
        print("SKIP: " + message)
        return

    ansible_playbook = require_command("ansible-playbook")
    ansible_galaxy = require_command("ansible-galaxy")
    docker = require_command("docker")
    cc = require_command("cc")
    env = os.environ.copy()

    runner = Runner(args.log_dir)
    versions = [
        runner.run(
            "ansible-version", [ansible_playbook, "--version"], env=env
        ).stdout.splitlines()[0],
        runner.run(
            "collection-version",
            [ansible_galaxy, "collection", "list", "community.docker"],
            env=env,
        ).stdout.strip(),
        runner.run(
            "docker-version",
            [
                docker,
                "version",
                "--format",
                "client={{.Client.Version}} server={{.Server.Version}}",
            ],
            env=env,
        ).stdout.strip(),
        runner.run(
            "compose-version", [docker, "compose", "version"], env=env
        ).stdout.strip(),
    ]
    print("Tested toolchain:\n" + "\n".join(versions), flush=True)

    with tempfile.TemporaryDirectory(prefix="onboard-runner-teardown-") as directory:
        root = Path(directory)
        image_dir = root / "image"
        project_dir = root / "project"
        fake_bin = root / "fake-bin"
        callback_dir = root / "callback_plugins"
        image_dir.mkdir()
        project_dir.mkdir()
        fake_bin.mkdir()
        callback_dir.mkdir()

        project_name = f"goatcore-onboard-runner-teardown-{os.getpid()}"
        image_name = f"{project_name}:test"
        source = image_dir / "idle.c"
        binary = image_dir / "idle"
        source.write_text(
            "#include <unistd.h>\n"
            "int main(void) { for (;;) pause(); }\n"
        )
        runner.run(
            "compile-idle",
            [cc, "-Os", "-static", "-s", "-o", str(binary), str(source)],
            env=env,
        )
        (image_dir / "Dockerfile").write_text(
            "FROM scratch\nCOPY idle /idle\nENTRYPOINT [\"/idle\"]\n"
        )
        compose_file = project_dir / "compose.yaml"
        compose_file.write_text(
            f"name: {project_name}\n"
            "services:\n"
            "  idle:\n"
            f"    image: {image_name}\n"
        )
        callback_file = callback_dir / "task_result_capture.py"
        callback_file.write_text(
            "from __future__ import absolute_import, division, print_function\n"
            "import json\n"
            "import os\n"
            "from ansible.plugins.callback import CallbackBase\n"
            "\n"
            "class CallbackModule(CallbackBase):\n"
            "    CALLBACK_VERSION = 2.0\n"
            "    CALLBACK_TYPE = 'aggregate'\n"
            "    CALLBACK_NAME = 'task_result_capture'\n"
            "    CALLBACK_NEEDS_ENABLED = True\n"
            "\n"
            "    def _write(self, result, event):\n"
            "        record = {\n"
            "            'event': event,\n"
            "            'failed': event == 'failed',\n"
            "            'task': result._task.get_name(),\n"
            "            'result': result._result,\n"
            "        }\n"
            "        with open(os.environ['TASK_RESULT_CAPTURE'], 'a') as stream:\n"
            "            stream.write(json.dumps(record, default=str) + '\\n')\n"
            "\n"
            "    def v2_runner_on_ok(self, result, **kwargs):\n"
            "        self._write(result, 'ok')\n"
            "\n"
            "    def v2_runner_on_failed(self, result, ignore_errors=False):\n"
            "        self._write(result, 'failed')\n"
        )

        role_vars = json.dumps(
            {
                "onboard_runner_dir": str(project_dir),
                "onboard_runner_enabled": False,
            }
        )
        role_command = [
            ansible_playbook,
            "-i",
            "localhost,",
            "-c",
            "local",
            str(PLAYBOOK),
            "--tags",
            TAG,
            "-e",
            role_vars,
            "-v",
        ]

        try:
            runner.run(
                "build-image",
                [
                    docker,
                    "build",
                    "--network",
                    "none",
                    "--tag",
                    image_name,
                    str(image_dir),
                ],
                env=env,
            )

            # Real lifecycle: an absent project is already converged.
            runner.run("absent-teardown", role_command, env=env)
            absent = runner.run(
                "inspect-absent",
                [
                    docker,
                    "ps",
                    "-aq",
                    "--filter",
                    f"label=com.docker.compose.project={project_name}",
                ],
                env=env,
            )
            assert not absent.stdout.strip(), "absent teardown left a project container"

            # Real lifecycle: Compose starts an inert, locally built image; the
            # production role task must stop and remove it.
            runner.run(
                "compose-up",
                [docker, "compose", "--file", str(compose_file), "up", "--detach"],
                env=env,
            )
            container_id = runner.run(
                "running-container-id",
                [
                    docker,
                    "compose",
                    "--file",
                    str(compose_file),
                    "ps",
                    "--quiet",
                    "idle",
                ],
                env=env,
            ).stdout.strip()
            assert container_id, "Compose did not create the synthetic container"
            running = runner.run(
                "inspect-running",
                [docker, "inspect", "--format", "{{.State.Running}}", container_id],
                env=env,
            )
            assert running.stdout.strip() == "true", "synthetic container was not running"

            runner.run("running-teardown", role_command, env=env)
            runner.run(
                "inspect-removed-container",
                [docker, "inspect", container_id],
                env=env,
                expected=1,
            )
            remaining = runner.run(
                "inspect-removed-project",
                [
                    docker,
                    "ps",
                    "-aq",
                    "--filter",
                    f"label=com.docker.compose.project={project_name}",
                ],
                env=env,
            )
            assert not remaining.stdout.strip(), "teardown left a project container"

            # Real lifecycle: a second teardown remains successful and absent.
            runner.run("repeated-teardown", role_command, env=env)
            repeated = runner.run(
                "inspect-repeated",
                [
                    docker,
                    "ps",
                    "-aq",
                    "--filter",
                    f"label=com.docker.compose.project={project_name}",
                ],
                env=env,
            )
            assert not repeated.stdout.strip(), "repeated teardown recreated a container"

            # Mocked warning paths: delegate every Docker call except `compose
            # down` to the real CLI. Compose 2.29+ emits progress as JSON;
            # community.docker supports a level=warning/msg event explicitly.
            fake_docker = fake_bin / "docker"
            fake_docker.write_text(
                "#!/usr/bin/env bash\n"
                "for argument in \"$@\"; do\n"
                "  if [[ \"$argument\" == down ]]; then\n"
                "    echo '{\"level\":\"warning\",\"msg\":\"No resource found to remove for project synthetic.\"}' >&2\n"
                "    if [[ \"${FAKE_DOCKER_RESULT:-mixed}\" == warning-only ]]; then\n"
                "      exit 0\n"
                "    fi\n"
                "    echo 'synthetic unrelated Docker daemon failure' >&2\n"
                "    exit 42\n"
                "  fi\n"
                "done\n"
                "exec \"$REAL_DOCKER\" \"$@\"\n"
            )
            fake_docker.chmod(0o755)
            callback_env = env.copy()
            callback_env["PATH"] = str(fake_bin) + os.pathsep + env["PATH"]
            callback_env["REAL_DOCKER"] = docker
            callback_env["ANSIBLE_CALLBACK_PLUGINS"] = str(callback_dir)
            callback_env["ANSIBLE_CALLBACKS_ENABLED"] = "task_result_capture"

            # A supported Compose warning event with CLI exit zero is not a
            # failure. Assert the captured task result and the warning emitted
            # by community.docker, not merely the playbook's console text.
            warning, warning_teardown = run_mocked_teardown(
                runner, role_command, callback_env, root, "warning-only", expected=0
            )
            warning_result = warning_teardown["result"]
            assert not warning_teardown["failed"], warning_teardown
            assert warning_teardown["event"] == "ok", warning_teardown
            assert "rc" not in warning_result, warning_result
            assert '"level":"warning"' in warning_result["stderr"], warning_result
            assert "No resource found" in warning_result["stderr"], warning_result
            assert any(
                "Docker compose:" in item and "No resource found" in item
                for item in warning_result.get("warnings", [])
            ), warning_result
            print(
                "Warning-only task result: "
                f"event={warning_teardown['event']} playbook_exit={warning.returncode}",
                flush=True,
            )

            # The same supported warning plus an unrelated error and CLI exit
            # 42 must remain a failure.
            failure, teardown = run_mocked_teardown(
                runner, role_command, callback_env, root, "mixed-failure", expected=2
            )
            task_result = teardown["result"]
            assert teardown["failed"], (
                "mixed warning/error result was incorrectly accepted\n"
                + json.dumps(teardown, indent=2)
            )
            assert teardown["event"] == "failed", teardown
            assert task_result["rc"] == 42, task_result
            assert "No resource found" in task_result["stderr"], task_result
            assert (
                "synthetic unrelated Docker daemon failure" in task_result["stderr"]
            ), task_result
            print(
                "Mixed failure task result: "
                f"event={teardown['event']} rc={task_result['rc']} "
                f"playbook_exit={failure.returncode}",
                flush=True,
            )
        finally:
            runner.run(
                "cleanup-compose",
                [docker, "compose", "--file", str(compose_file), "down", "--remove-orphans"],
                env=env,
            )
            runner.run(
                "cleanup-image",
                [docker, "image", "rm", image_name],
                env=env,
            )

    print("Disabled onboard-runner teardown lifecycle passed.")


if __name__ == "__main__":
    main()
