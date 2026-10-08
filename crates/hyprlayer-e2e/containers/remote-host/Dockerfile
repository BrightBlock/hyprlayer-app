# A remote host for the E2E container tier: sshd, git, and one key-only user.
# The server binary and the authorized key are installed per run.
# The harness passes a private per-platform tag of this base, so --platform never re-points it.
ARG BASE=debian:bookworm-slim
FROM ${BASE}

RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server git procps ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --shell /bin/bash e2e \
 && mkdir -p /run/sshd \
 && printf 'PasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin no\nAllowUsers e2e\n' \
      > /etc/ssh/sshd_config.d/e2e.conf

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
