# A remote host for the E2E container tier: sshd, git, and one key-only user.
# The server binary and the authorized key are installed per run.
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server git procps ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --shell /bin/bash e2e \
 && mkdir -p /run/sshd \
 && printf 'PasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin no\nAllowUsers e2e\n' \
      > /etc/ssh/sshd_config.d/e2e.conf

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
