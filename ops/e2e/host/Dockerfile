# A hermetic "management host" for the deployment e2e harness
# (docs/design/deployment-e2e.md §Hermetic): what the push converger needs from
# a host and nothing else — sshd with a key-only sudoer, python3 for Ansible,
# the docker CLI and compose plugin talking to the runner's daemon through the
# mounted socket. The ledger role writes /var/lib/goat/<component>/ here exactly
# as on a Thor.
FROM ubuntu:26.04@sha256:6a06111c3b0a8600feadb270aec771dc239bf407b25625f93f44ab00e11f4d3c
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server sudo python3 ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /run/sshd /var/lib/goat \
 && useradd -m -s /bin/bash goat \
 && echo 'goat ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/goat \
 && chmod 0440 /etc/sudoers.d/goat \
 && sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication no/' /etc/ssh/sshd_config
COPY --from=docker:27.3.1-cli@sha256:328eb399a065780c2cebe9224de003aa14084cf69efae882ac27430f921819b7 /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker/compose-bin:v2.29.7@sha256:25beb180af2138ac58697b2c229bd7df9992b8d47a8272008c1f1a32d3130253 /docker-compose /usr/local/lib/docker/cli-plugins/docker-compose
# The harness mounts the operator's public key at /pubkey at run time.
COPY entrypoint.sh /entrypoint.sh
RUN chmod 0755 /entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
