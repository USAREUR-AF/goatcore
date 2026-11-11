# CI apt failover: our own package cache, used only when Ubuntu's mirror does not answer

Status: built (`.github/actions/apt-failover/action.yml`, wired into the jobs below). Status: WIP — gate: a run with archive.ubuntu.com, security.ubuntu.com and azure.archive.ubuntu.com black-holed in which every wired job's install or image build succeeds through the cache.

## The job

A GitHub runner installs packages from Ubuntu's archive, through its Azure mirror. When that mirror and the archive do not answer, every job that installs a package fails for a reason that has nothing to do with the change under test. The failover keeps those jobs working on our own copy of exactly the packages CI installs, and uses it only when the mirror is down.

## How it works

`apt-failover` runs right after checkout in each job that installs a package or builds an image that does.

1. It finds the mirror apt is configured with: the first `URIs:` of `/etc/apt/sources.list.d/ubuntu.sources`, following the runner's `mirror+file:` list to its first entry, else the first `deb` line of `/etc/apt/sources.list`.
2. It asks that mirror for `dists/<codename>/Release`, with `bash` alone, and allows `probe-seconds` (10) for the answer.
3. If the mirror answers, nothing changes, and one line says so. That is the normal path.
4. If it does not answer:
   - it logs in to ghcr.io with the job token, or falls back to an anonymous pull;
   - it pulls the `cache-image` input (`ghcr.io/usareur-af/ci-apt-cache:<date>`, an apt-cacher-ng that serves its baked cache offline). A tag is pulled with a warning that it is not pinned by digest. A pull denied after a successful token login is retried anonymously, for a public package;
   - it runs that image on the host network on port 3142, and waits for it to listen;
   - it writes `/etc/apt/apt.conf.d/00ci-failover` (`Acquire::http::Proxy "http://127.0.0.1:3142"`);
   - it sets `proxies.default` in `~/.docker/config.json`, with `httpProxy` at the default bridge's gateway (inside a container, 127.0.0.1 is the container), and `noProxy` covering loopback, the private ranges, `.local` and `.internal`. It sets no `httpsProxy`, so the https fetches of image builds (PyPI, galaxy, registries) stay direct;
   - it runs `apt-get update` once through the cache, so every later install, with or without its own update, resolves against the cache's indexes. A runner's lists are as old as its image (peering-host-firewall installs nftables without an update). If an Ubuntu source fails that refresh, the step exits 69, "environment", in one line. A warning from the runner image's own third-party https sources is not counted;
   - it exports `APT_FAILOVER=1` to the later steps;
   - one line names the path taken.
5. If the image cannot be pulled, or does not listen, the step fails and says why. The job had no source for its packages either way.

`ops/ci/apt-install-fallback.sh` sees `Acquire::http::Proxy` once this file is written, and then uses the proxy and never rewrites sources. So the two never fight.

Nothing restarts.

## Where the image is named

The image is named in two places. Swap the pushed digest into both:

1. the `cache-image` input default in `.github/actions/apt-failover/action.yml`;
2. `services: apt-cache: image` of `fresh-ubuntu` in `.github/workflows/goatlite-workstation.yml`.

## Limits of the design

- The probe checks only the first mirror apt names, the runner's Azure mirror. If archive.ubuntu.com or security.ubuntu.com is down while Azure answers, the action does not fail over. That matters to windows-portability, which reads those two hosts directly.
- The cache image must listen on all addresses, not only 127.0.0.1. Docker builds and containers reach it at the bridge gateway, and a service at its own hostname.
- fresh-ubuntu's service image is pulled on every run, not only on failover. The docker CLI reads its config on each command, and apt reads its config on each run.

## Job containers

`goatlite-workstation.yml` `fresh-ubuntu` runs in `container: ubuntu:24.04`. Its steps have no docker CLI, and 127.0.0.1 there is the job container. So the cache runs as the job's `services:` entry `apt-cache` (the same image, named in two places: change both together). The action, given `service-host: apt-cache`, only probes, and on failover points the container's apt at `http://apt-cache:3142` and refreshes the lists through it.

The cost: GitHub fails a job whose service image cannot be pulled, so this one job now pulls our image on every run, also when the mirror is fine. It does so from ghcr.io, with the job token. This is accepted because ghcr.io is GitHub's own registry, which the runner already depends on for actions, and the job reaches the same Docker Hub and GitHub hosts in any case. It needs the package to grant this repository read access, or to be public.

## Sources the proxy cannot serve

An `https://` apt source bypasses `Acquire::http::Proxy`. One job writes its own:

- `goat-cli-pr.yml` `windows-portability` installs `gcc-mingw-w64-x86-64` from https archive.ubuntu.com and security.ubuntu.com. With `APT_FAILOVER=1`, it writes the same lines with `http://`. The archive keyring verifies every index either way.

The runner image's own extra sources (for example packages.microsoft.com) are not installed from by any job. An unreachable one is only a warning to `apt-get update`.

## What the cache cannot hold

`ci.yml` `python-and-ansible` installs `linux-modules-extra-$(uname -r)` for the runner's running Azure kernel, for its scsi_debug flash test. A cache built on one day holds only that day's kernel. On a newer runner kernel with the archive down, that install misses, and the test fails rather than skips. The cache must be refreshed when the runner kernel changes. The design of that test is the owner's decision, not part of the failover. `federation-e2e.yml` has the same limit for wireguard, and only on a kernel without its own module.

## Wired jobs

- `ci.yml`: go, python-and-ansible, peering-forwarder, peering-host-firewall, bats-bash32 and go-vuln.
- One job each:
  - `federation-bundle.yml` bundle;
  - `federation-republisher.yml` test;
  - `deploy-e2e.yml` hermetic;
  - `goatbuilder-pr.yml` host;
  - `goat-cli-pr.yml` windows-portability;
  - `signal-server.yml` test;
  - `enrollment.yml` unit;
  - `grant-crypto.yml` portal-mint-conformance;
  - `wg-cp0-accessor.yml` unit;
  - `wg-cp0-mesh-reach.yml` unit;
  - `docs-i18n-currency.yml` currency;
  - `docs-picker-e2e.yml` picker;
  - `_release-binary.yml` docs;
  - `sovereign-pki-registry.yml` check-sovereign-pkis;
  - `zero-saas-recipes.yml` local-registry;
  - `goatlite-vm-image.yml` build;
  - `netbird-mgmt.yml` build;
  - `goatlite-workstation.yml` fresh-ubuntu;
  - `id-console-web-e2e.yml` e2e (Playwright's `install --with-deps`);
  - `portal-web-e2e.yml` e2e (the same).
- `federation-e2e.yml` is maintained by the test environment, which adds the step there.

Each job that pulls the image has `packages: read`. Where its workflow set only `contents: read` at the top, the job now sets `contents: read` and `packages: read`, and nothing else is widened. Where a workflow has no permissions block, the wired job keeps its defaults, except the two Playwright e2e jobs, which now set `contents: read` and `packages: read`.

## Not built

- The cache refresh when the runner image's kernel changes.
- An arm64 build of the cache image for `netbird-mgmt`'s arm64 runner. On that runner a failover pull of an amd64-only image fails, and only when the mirror is down.
