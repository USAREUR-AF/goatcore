#!/usr/bin/env python3
"""build_with_mirror_fallback builds once with the runner's Ubuntu mirror and, when
that build fails, once more with the image's own sources.

A runner's mirror can pass apt-failover's probe and then stop answering mid-install
(peering-forwarder and peering-host-firewall, run 44643250287: the indexes came from
the mirror, then every package failed to connect about two minutes later). The helper
in ops/ci/docker-build-mirror-fallback.sh takes the build's own arguments, passes
UBUNTU_MIRROR as a build argument, and on a failure warns, naming the mirror, and
builds again with UBUNTU_MIRROR empty. It fails only when both builds fail, and says
so. With UBUNTU_MIRROR unset it builds once.

Each case runs the helper under bash with a stand-in `docker` that records its
arguments and stdin and fails on demand.

Run: python3 -m pytest ops/ci/test_docker_build_mirror_fallback.py
"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
HELPER = REPO / "ops/ci/docker-build-mirror-fallback.sh"
MIRROR = "http://mirror.example.org/ubuntu"
DOCKERFILE = "FROM ubuntu:24.04\nRUN apt-get update\n"

# Records each call's arguments and stdin; fails when the call carries a non-empty
# UBUNTU_MIRROR and FAIL_WITH_MIRROR=1, or an empty one and FAIL_WITHOUT_MIRROR=1.
STANDIN = r'''#!/usr/bin/env bash
n=$(( $(cat "$LOG/n" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$LOG/n"
printf '%s\n' "$*" > "$LOG/args.$n"
cat > "$LOG/stdin.$n"
case " $* " in
  *" UBUNTU_MIRROR= "*) [ "${FAIL_WITHOUT_MIRROR:-0}" = 1 ] && exit 1 ;;
  *" UBUNTU_MIRROR="*) [ "${FAIL_WITH_MIRROR:-0}" = 1 ] && exit 1 ;;
esac
exit 0
'''


class BuildWithMirrorFallback(unittest.TestCase):

    def run_helper(self, mirror, fail_with=False, fail_without=False):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            bindir, log = d / "bin", d / "log"
            bindir.mkdir()
            log.mkdir()
            (bindir / "docker").write_text(STANDIN)
            (bindir / "docker").chmod(0o755)
            env = dict(os.environ, PATH=f"{bindir}:{os.environ['PATH']}", LOG=str(log),
                       FAIL_WITH_MIRROR="1" if fail_with else "0",
                       FAIL_WITHOUT_MIRROR="1" if fail_without else "0")
            env.pop("UBUNTU_MIRROR", None)
            if mirror is not None:
                env["UBUNTU_MIRROR"] = mirror
            script = f'. "{HELPER}"; build_with_mirror_fallback --progress=plain -t fixture -'
            r = subprocess.run(["bash", "-c", script], input=DOCKERFILE, env=env,
                               capture_output=True, text=True, timeout=60)
            calls = int((log / "n").read_text()) if (log / "n").exists() else 0
            args = [(log / f"args.{i}").read_text().strip() for i in range(1, calls + 1)]
            stdin = [(log / f"stdin.{i}").read_text() for i in range(1, calls + 1)]
            return r, args, stdin

    def test_a_good_mirror_builds_once(self):
        r, args, stdin = self.run_helper(MIRROR)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(args), 1)
        self.assertIn(f"--build-arg UBUNTU_MIRROR={MIRROR}", args[0])
        self.assertIn("-t fixture -", args[0])
        self.assertEqual(stdin, [DOCKERFILE])
        self.assertNotIn("::warning::", r.stdout + r.stderr)

    def test_a_failed_build_with_the_mirror_is_built_again_without_it(self):
        r, args, stdin = self.run_helper(MIRROR, fail_with=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(args), 2)
        self.assertIn(f"--build-arg UBUNTU_MIRROR={MIRROR}", args[0])
        self.assertIn("--build-arg UBUNTU_MIRROR= ", args[1] + " ")
        # The inline Dockerfile reaches both builds whole.
        self.assertEqual(stdin, [DOCKERFILE, DOCKERFILE])
        out = r.stdout + r.stderr
        self.assertIn("::warning::", out)
        self.assertIn(MIRROR, out)

    def test_both_builds_failing_fails_and_says_so(self):
        r, args, _ = self.run_helper(MIRROR, fail_with=True, fail_without=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(len(args), 2)
        self.assertIn("failed twice", r.stdout + r.stderr)
        self.assertIn(MIRROR, r.stdout + r.stderr)

    def test_no_mirror_builds_once_and_keeps_its_result(self):
        for fail in (False, True):
            with self.subTest(fails=fail):
                r, args, _ = self.run_helper(None, fail_without=fail)
                self.assertEqual(len(args), 1)
                self.assertIn("--build-arg UBUNTU_MIRROR= ", args[0] + " ")
                self.assertEqual(r.returncode != 0, fail)
                self.assertNotIn("::warning::", r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
