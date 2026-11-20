#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# build_with_mirror_fallback <docker build arguments...>
#
# Sourced by the scripts that build an ubuntu fixture image with apt in CI
# (docs/design/ci-apt-failover.md, "Docker builds on the normal path").
#
# .github/actions/apt-failover exports the runner's Ubuntu mirror as
# UBUNTU_MIRROR once it has answered a probe, and each such build points its
# apt sources at it. A mirror that answered the probe can still stop answering
# mid-install: in run 44643250287 a fixture build fetched its indexes from the
# mirror, then could not connect for any package about two minutes later.
#
# So this runs `docker build --build-arg UBUNTU_MIRROR=<mirror> <arguments>`.
# If that fails while a mirror was set, it prints a ::warning:: naming the
# mirror and builds once more with UBUNTU_MIRROR empty, which leaves the
# image's own sources (archive.ubuntu.com, security.ubuntu.com). It fails only
# when both builds fail, and says so. With UBUNTU_MIRROR unset (a local run, or
# the failover path) it builds once. An inline Dockerfile on stdin (a `-`
# context) is kept so both builds get it whole. The warning and the failure
# go to stderr, which the runner reads for annotations too, so a caller that
# discards the build's stdout still shows them.
#
# Not covered: the mirror dying while the image's own sources are also
# unreachable. Both builds then fail; that needs the CI apt cache started on
# demand, which apt-failover does only when its probe fails.

build_with_mirror_fallback() {
  local mirror="${UBUNTU_MIRROR:-}" stdin_file="" arg rc
  for arg in "$@"; do
    if [ "$arg" = "-" ]; then
      stdin_file="$(mktemp)"
      cat > "$stdin_file"
      break
    fi
  done

  _bwmf_build() {
    if [ -n "$stdin_file" ]; then
      docker build --build-arg UBUNTU_MIRROR="$1" "${@:2}" < "$stdin_file"
    else
      docker build --build-arg UBUNTU_MIRROR="$1" "${@:2}"
    fi
  }

  rc=0
  _bwmf_build "$mirror" "$@" || rc=$?
  if [ "$rc" -ne 0 ] && [ -n "$mirror" ]; then
    echo "::warning::docker build with UBUNTU_MIRROR=$mirror failed (exit $rc); building once more with the image's own Ubuntu sources" >&2
    rc=0
    _bwmf_build "" "$@" || rc=$?
    if [ "$rc" -ne 0 ]; then
      echo "docker build failed twice: with UBUNTU_MIRROR=$mirror and with the image's own Ubuntu sources (exit $rc)" >&2
    fi
  fi
  if [ -n "$stdin_file" ]; then
    rm -f "$stdin_file"
  fi
  return "$rc"
}
