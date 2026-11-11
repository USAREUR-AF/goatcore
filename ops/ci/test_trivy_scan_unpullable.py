"""The unpullable classifier reads only the registry's own verdict.

Review finding on PR #9: Trivy probes docker, containerd, podman and the
registry and prints one error line per source. Every miss begins "unable to
find the specified image", and on GitHub's runners the containerd probe always
reports "permission denied", so a classifier that matches anywhere in the
output would call a registry outage or a timeout "not pullable" and warn
instead of failing. Only the "* remote error:" line decides, and on it only
the OCI codes DENIED, MANIFEST_UNKNOWN, NAME_UNKNOWN and UNAUTHORIZED.

The positive fixtures are verbatim Trivy output captured 2026-11-10 against
ghcr.io (a never-published package, and the code lines the registry serves for
private and unknown references). The negative fixtures are the same
multi-source shape with the registry answering 500, a timeout with no remote
line at all, and a TLS failure, which must all stay scanner failures.
"""

import importlib.util
import pathlib

_SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "trivy-scan.py"
_SPEC = importlib.util.spec_from_file_location("trivy_scan", _SCRIPT)
trivy_scan = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(trivy_scan)


MANIFEST_UNKNOWN_BLOCK = """\
2026-11-10T21:04:57+01:00\tFATAL\tFatal error\trun error: image scan error: scan error: unable to initialize a scan service: unable to initialize artifact: unable to initialize container image: unable to find the specified image "ghcr.io/usareur-af/no-such-image-fixture:latest" in ["docker" "containerd" "podman" "remote"]: 4 errors occurred:
\t* docker error: unable to inspect the image (ghcr.io/usareur-af/no-such-image-fixture:latest): Error response from daemon: No such image: ghcr.io/usareur-af/no-such-image-fixture:latest
\t* containerd error: containerd socket not found: /run/containerd/containerd.sock
\t* podman error: unable to initialize Podman client: no podman socket found: stat podman/podman.sock: no such file or directory
\t* remote error: GET https://ghcr.io/v2/usareur-af/no-such-image-fixture/manifests/latest: MANIFEST_UNKNOWN: manifest unknown
"""

DENIED_LINE = (
    "\t* remote error: GET https://ghcr.io/token?scope=repository%3Ausareur-af"
    "%2Fherd%3Apull&service=ghcr.io: DENIED: requested access to the resource "
    "is denied\n"
)

NAME_UNKNOWN_LINE = (
    "\t* remote error: GET https://registry.example/v2/goat/retired/manifests/"
    "sha256:4242424242424242424242424242424242424242424242424242424242424242: "
    "NAME_UNKNOWN: repository name not known to registry\n"
)

UNAUTHORIZED_LINE = (
    "\t* remote error: HEAD https://registry.example/v2/goat/private/manifests/"
    "latest: UNAUTHORIZED: authentication required\n"
)

# The review's exact case: the containerd line says "permission denied" (it
# always does on GitHub's runners) while the registry itself answered 500.
# Before the fix this classified as unpullable; it must stay a failure.
RUNNER_OUTAGE_BLOCK = """\
FATAL\tFatal error\trun error: image scan error: scan error: unable to initialize a scan service: unable to initialize artifact: unable to initialize container image: unable to find the specified image "ghcr.io/usareur-af/herd:latest" in ["docker" "containerd" "podman" "remote"]: 4 errors occurred:
\t* docker error: unable to inspect the image (ghcr.io/usareur-af/herd:latest): Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?
\t* containerd error: failed to list images from containerd: open /run/containerd/containerd.sock: permission denied
\t* podman error: unable to initialize Podman client: no podman socket found: stat podman/podman.sock: no such file or directory
\t* remote error: GET https://ghcr.io/v2/usareur-af/herd/manifests/latest: unexpected status code 500 Internal Server Error
"""

TIMEOUT_OUTPUT = (
    "FATAL\tFatal error\trun error: image scan error: scan error: unable to "
    'initialize a scan service: unable to find the specified image '
    '"ghcr.io/usareur-af/herd:latest": context deadline exceeded\n'
)

TLS_LINE = (
    '\t* remote error: Get "https://ghcr.io/v2/": tls: failed to verify '
    "certificate: x509: certificate signed by unknown authority\n"
)


def test_each_registry_not_served_code_is_unpullable():
    for fixture in (
        MANIFEST_UNKNOWN_BLOCK,
        DENIED_LINE,
        NAME_UNKNOWN_LINE,
        UNAUTHORIZED_LINE,
    ):
        assert trivy_scan.image_is_unpullable(fixture), fixture


def test_runner_permission_denied_with_registry_500_stays_a_failure():
    assert not trivy_scan.image_is_unpullable(RUNNER_OUTAGE_BLOCK)


def test_timeout_and_tls_and_empty_output_stay_failures():
    for fixture in (TIMEOUT_OUTPUT, TLS_LINE, ""):
        assert not trivy_scan.image_is_unpullable(fixture), fixture


def test_codes_outside_the_remote_error_line_do_not_classify():
    # The registry verdict must come from the remote line itself; the same
    # words anywhere else (a finding title, a layer name) must not match.
    assert not trivy_scan.image_is_unpullable(
        "INFO\tdetected misconfiguration: access denied: policy MANIFEST_UNKNOWN: x\n"
        "\t* docker error: No such image\n"
    )



def test_permission_denied_inside_the_remote_line_is_not_a_code():
    # The trailing colon is what separates the OCI code DENIED from the words
    # "permission denied". Here the registry's own line carries those words
    # with a 500, which is an outage, not "not served", so it must stay a
    # failure. Without the colon in the code list this would classify.
    assert not trivy_scan.image_is_unpullable(
        "\t* remote error: GET https://ghcr.io/v2/usareur-af/fixture/manifests/latest: "
        "unexpected status code 500 Internal Server Error: permission denied reading blob\n"
    )

if __name__ == "__main__":
    # Standalone, so the Trivy workflow can run it without installing pytest.
    cases = [fn for name, fn in sorted(globals().items()) if name.startswith("test_") and callable(fn)]
    for case in cases:
        case()
    print(f"ok: trivy unpullable classifier, {len(cases)} cases")
