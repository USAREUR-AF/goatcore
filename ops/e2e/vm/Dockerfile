FROM ubuntu:24.04@sha256:24737cef17f4e22b1977df7d9a05e77c4e28b7da5b8911f8d8ccda136bf3573c
ARG UBUNTU_MIRROR=
# CI passes the runner's Ubuntu mirror (.github/actions/apt-failover); unset, the image's sources stay.
RUN if [ -n "$UBUNTU_MIRROR" ] && [ -f /etc/apt/sources.list.d/ubuntu.sources ]; then \
      sed -i -E "s#https?://(archive|security)\.ubuntu\.com/ubuntu/?#$UBUNTU_MIRROR/#" /etc/apt/sources.list.d/ubuntu.sources; fi
RUN apt-get update && apt-get install -y --no-install-recommends qemu-system-arm qemu-efi-aarch64 ipxe-qemu && rm -rf /var/lib/apt/lists/*
