# A hermetic host for the federation end-to-end run (docs/design/deployment-e2e.md,
# docs/design/cross-sovereign-federation.md): systemd, sshd, its own container engine,
# WireGuard and the operator's tools. Each host runs its own dockerd because the edge
# role bind-mounts host paths and reads the container's PID, and neither survives a
# socket shared with the runner. Needs --privileged, and the wireguard kernel module on
# the runner. Built through a proxy when the runner's docker sets one (the CI failover cache):
# apt honours http_proxy for the Ubuntu archive (plain http, signed indexes); pip and
# ansible-galaxy reach PyPI and Galaxy over https directly, which an apt cache does not serve.
# python3-apt (Ansible's apt module) and unzip (goat-zenoh) are in the image, so
# phase 2's apt tasks find them installed and a run reaches no mirror for them.
FROM ubuntu:26.04@sha256:6a06111c3b0a8600feadb270aec771dc239bf407b25625f93f44ab00e11f4d3c
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get -o Acquire::Retries=5 update \
 && apt-get -o Acquire::Retries=5 install -y --no-install-recommends \
      systemd systemd-sysv dbus openssh-server openssh-client sudo python3 python3-pip python3-yaml \
      docker.io docker-compose-v2 iptables wireguard-tools nftables iproute2 iputils-ping \
      curl openssl jq git ca-certificates util-linux \
      python3-apt unzip \
 && env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy \
      python3 -m pip install --quiet --break-system-packages 'ansible-core>=2.16,<2.21' \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -m -s /bin/bash -G docker goat \
 && echo 'goat ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/goat \
 && chmod 0440 /etc/sudoers.d/goat \
 && sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication no/' /etc/ssh/sshd_config \
 && git config --system --add safe.directory '*' \
 && for attempt in 1 2 3 4 5; do \
      env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy \
      ansible-galaxy collection install --collections-path /usr/share/ansible/collections \
        'ansible.posix:>=1.5.0' 'community.general:>=8.0.0' 'community.docker:>=3.4.0' && break; \
      [ "$attempt" != 5 ] || exit 1; sleep 20; \
    done \
 && systemctl enable ssh docker
COPY daemon.json /etc/docker/daemon.json
# The harness mounts the run's key directory at /etc/fed-e2e and this unit installs it,
# so a host is reachable as soon as sshd is.
COPY fed-e2e-keys.service /etc/systemd/system/fed-e2e-keys.service
RUN systemctl enable fed-e2e-keys
STOPSIGNAL SIGRTMIN+3
ENTRYPOINT ["/sbin/init"]
