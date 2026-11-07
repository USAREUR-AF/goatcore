# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
#
# onboard-runner container image — the pull-model onboarding auto-executor
# (docs/design/operator-onboard-intake.md §"Fast-follow"), run as a CONTAINER
# with its OWN scoped portal identity rather than a host process reusing the
# operator's login (the earlier shape).
#
# Two-stage build:
#   1. build   — static Go binary (CGO disabled), no runtime deps of its own.
#   2. runtime — the runtime-shell base (glibc, a shell, apt-get or apk; ADR
#      1220). The runner SHELLS OUT to `goat
#      admin onboard --native`, so the runtime carries:
#        - glibc         — the site `goat` binary is glibc-linked
#                          (aarch64-unknown-linux-gnu); the base's glibc must
#                          be >= the Ubuntu-24.04 build target (2.39): Debian 13
#                          (2.41) and Wolfi's rolling glibc both are.
#        - sh + curl     — the native onboard's Ory leg runs `sh -c 'curl …'`
#                          in-container against the sibling Kratos admin API
#                          (GOAT_ONBOARD_KRATOS_LOCAL=1).
#        - python 3.13   — lifecycle-sweep.py is the single hold release engine.
#      The `goat` binary itself is NOT baked in — it is bind-mounted from the
#      host at deploy time (/usr/local/bin/goat, role-managed, one version
#      source), so the container tracks the host's goat with no second copy to
#      drift. The image carries NO identity material; the minimal goat profile
#      (portal endpoint + the runner's own portal token) is materialized at
#      startup by docker-entrypoint.sh from the deploy env-file.

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_RUNTIME_SHELL=cgr.dev/chainguard/wolfi-base:latest@sha256:8509ee4351a2e501b7f98201f2e2642396e82020c61a589cacdcfe74b9de46c1

FROM golang:1.26-bookworm@sha256:23ce71337b726608afb1bc00986e630a05034a87a87ea1fc2cd1ae5cf33387eb AS build
WORKDIR /src
# Cache module downloads first for layer reuse.
COPY go.mod go.sum ./
RUN go mod download
COPY *.go ./
ENV CGO_ENABLED=0
RUN go build -trimpath -ldflags="-s -w" -o /onboard-runner .

FROM ${BASE_RUNTIME_SHELL}
# curl (+ ca-certificates) for the native onboard's local Kratos-admin leg, and
# python 3.13. The public base (Chainguard wolfi-base) takes both from apk. A
# Debian base takes curl from apt-get and must already carry python3
# (python:3.13-slim does). The RUN fails if python3 is not on PATH afterwards.
RUN if command -v apk >/dev/null 2>&1; then \
      apk add --no-cache curl ca-certificates python-3.13; \
    else \
      apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
      && rm -rf /var/lib/apt/lists/*; \
    fi \
    && python3 --version
# A writable home for the materialized goat profile — non-root uid, no passwd
# entry needed (goat reads $HOME, not /etc/passwd).
RUN mkdir -p /home/runner && chown 65532:65532 /home/runner
COPY --from=build /onboard-runner /usr/local/bin/onboard-runner
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh
USER 65532:65532
ENV HOME=/home/runner
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
