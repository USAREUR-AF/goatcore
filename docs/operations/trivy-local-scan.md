# Scan the local repository with Trivy

This guide shows developers how to run Trivy against a local goat checkout.
The scan is manual and runs on your workstation. It does not add a CI job or
scan deployed hosts or container images.

## Install Trivy

Use the official [Trivy installation guide](https://trivy.dev/latest/getting-started/installation/)
for your operating system. On macOS or Linux with Homebrew:

```bash
brew install trivy
trivy --version
```

The Trivy version is managed by your local package manager; this repository
does not currently pin a developer CLI version.

## Scan the checkout

From the root of the goat repository, run:

```bash
trivy fs --scanners vuln,misconfig,secret .
```

This scans supported dependency manifests and lockfiles for known
vulnerabilities, recognized configuration files for misconfigurations, and
files for secret patterns. Trivy fetches its vulnerability database as needed,
so the first scan requires network access. See the [filesystem scan reference](https://trivy.dev/latest/guide/target/filesystem/)
for supported files and scanner options.

By default, findings are printed but do not make the command return a failure
status. To have high or critical findings return status `1`, run:

```bash
trivy fs \
  --scanners vuln,misconfig,secret \
  --severity HIGH,CRITICAL \
  --exit-code 1 \
  .
```

Use this option when you want a local command or script to stop on findings.
The severity filter applies to the findings included in that run.

## Save a JSON report

To write the results to a file outside the checkout:

```bash
trivy fs \
  --scanners vuln,misconfig,secret \
  --format json \
  --output /tmp/goat-trivy.json \
  .
```

Change the output path if `/tmp` is not available on your platform. Treat the
report as security-sensitive: review it before sharing, and avoid committing
it to the repository.

## Scan a container image

An image scan examines packages and files inside a built image. It is a
separate scan from `trivy fs`, which examines files in the checkout. Trivy
can scan images from your local container engine or from a registry. See the
[container image scan reference](https://trivy.dev/latest/guide/target/container_image/)
for details.

### Build and scan an image locally

The following example builds the `goat-mesh-health` image using the same
Dockerfile context as its image workflow. Docker must be installed and its
daemon running:

```bash
docker build \
  --tag goat-mesh-health:local \
  ops/zenoh-trial-peers/mesh-health

trivy image goat-mesh-health:local
```

Trivy checks the local image store before attempting to pull an image from a
registry. The default image scan checks for vulnerabilities and secrets. To
return status `1` when high or critical findings are present:

```bash
trivy image \
  --severity HIGH,CRITICAL \
  --exit-code 1 \
  goat-mesh-health:local
```

### Scan an image from GHCR

Images built by this repository's workflows are published under
`ghcr.io/usareur-af/`. For example, to scan the published `goat-mesh-health`
image:

```bash
trivy image ghcr.io/usareur-af/goat-mesh-health:latest
```

Use the tag or digest you intend to examine; `latest` can change over time.
Public images can be scanned without logging in. For a private package, first
authenticate Docker to GHCR with an account that has package read access. See
GitHub's [Container registry authentication instructions](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry#authenticating-in-the-container-registry).

### Save an image scan report

Use `--format json` and `--output` as with a filesystem scan, for example:

```bash
trivy image \
  --format json \
  --output /tmp/goat-mesh-health-trivy.json \
  goat-mesh-health:local
```

Keep reports outside the checkout unless you intend to add them to a change
after reviewing their contents.

## Run the checkout and curated image scans together

The repository includes a convenience script that runs the filesystem scan
once, then scans the images listed in
[`ops/scripts/trivy-images.txt`](../../ops/scripts/trivy-images.txt):

```bash
python3 ops/scripts/trivy-scan.py
```

The curated list covers commonly deployed first-party services and is based
on the fleet pins and image build workflows. The script scans the published
`:latest` tag for each image, which shows the newest build of that image. It
then scans each digest in
[`ops/pins/fleet-pins.yaml`](../../ops/pins/fleet-pins.yaml), which shows what a
goatnet deploys. Those rows end in `(deployed pin)`. Compare the two when they
differ: a fix in `:latest` doesn't reach a goatnet until its pin moves. The
script doesn't build images. It scans `linux/amd64` by default. To scan the
arm64 variants instead, run:

```bash
python3 ops/scripts/trivy-scan.py --platform linux/arm64
```

The final table shows finding counts by severity for the checkout and each
image. Trivy scan failures appear as errors, separate from a scan with zero
findings. An image the registry refuses or can't find appears as `NOT PULLABLE`
and raises a warning instead of an error. A large image can exceed Trivy's
default five-minute limit on a slow connection, which appears as an error. The
script saves the full JSON reports under
`.envrc.local.d/trivy-reports/` in a unique, UTC-timestamped directory for
each run and prints the path. Folder names begin with
`run-YYYYMMDDTHHMMSS.ffffffZ-`; they sort chronologically and include a unique
suffix. `.envrc.local.d/` is ignored by Git, so these local reports are not
included in commits or pushed to the remote. The script returns a nonzero
status if any scan fails; findings alone do not make it fail. Review the
findings and reports as part of triage. Each run folder also contains a
standalone `summary.html`; open it in a browser to filter findings by severity
and review target, finding ID, package, versions, and summary. Secret match
text and raw JSON are not copied into the HTML page.

### Open the local HTML report

The script prints the full path on the line beginning `HTML summary:`. Open
that file in your browser. On macOS, for example, copy the printed path into:

```bash
open /path/printed/by/the/script/summary.html
```

On Linux, use `xdg-open` with the printed path, or navigate in your file
manager to `.envrc.local.d/trivy-reports/`, open the newest timestamped
`run-...` folder, and open `summary.html`.

The script requires Python 3 and Trivy. It reads GHCR images using the
credentials available to Trivy/Docker, so authenticate to GHCR first if your
account needs access to these packages.

## GitHub Actions advisory scan

The `Trivy advisory scan` workflow runs on pull requests that change supported
dependency, container, Terraform, or YAML inputs; it also runs weekly and can
be started manually from the Actions tab. It scans the checkout, the same
curated set of published GHCR images, and the digests in the fleet pins.

The workflow scans vulnerabilities and repository misconfigurations. It skips
secret scanning so uploaded reports do not contain secret matches. The job
summary shows finding counts and scan errors; the JSON reports and generated
HTML page are available as a workflow artifact for 14 days. Findings, scanner
errors, and setup/report errors do not fail the workflow or block pull
requests. Treat an error as an incomplete scan, not a clean result. This is
visibility during rollout, not a security gate.

### Download and open the GitHub Actions HTML report

1. Open the repository's **Actions** tab and select the **Trivy advisory
   scan** workflow.
2. Open the run you want to review. In its **Artifacts** section, download
   `trivy-reports-<run id>`.
3. Extract the downloaded archive, find `summary.html` inside the timestamped
   `run-...` folder, and open it in a browser. The severity controls in the
   page filter the findings interactively.

Artifacts are retained for 14 days. If the run has no report artifact, check
the workflow summary and logs: the scan may have stopped before it wrote the
reports, so missing output does not mean there were no findings.

## Scope and limitations

- `trivy fs` scans the local source tree and supported files; it does not
  build the project or inspect image layers. Use `trivy image` for an image
  that you have built or pulled.
- Trivy's results depend on the files and package formats it recognizes and
  the vulnerability database available at scan time. A clean result is not a
  guarantee that the repository or a deployed system is vulnerability-free.
- The advisory workflow scans the published image tags in the curated list;
  it does not build or scan the images produced by each image workflow.
- Per-image blocking Trivy/Grype build gates and recurring scans of deployed
  fleet images remain separate work.
