# Builds hyprlayer-server for the E2E container tier: Rust, plus the Zig that
# builds libghostty-vt. The harness passes a private per-platform tag of this base.
ARG BASE=rust:1-bookworm
FROM ${BASE}

RUN arch="$(uname -m)" \
 && case "$arch" in \
      aarch64) sha=ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17 ;; \
      x86_64) sha=70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00 ;; \
    esac \
 && file="zig-$arch-linux-0.16.0.tar.xz" \
 && curl -fsSLO "https://ziglang.org/download/0.16.0/$file" \
 && echo "$sha  $file" | sha256sum -c - \
 && mkdir -p /opt/zig \
 && tar -xJf "$file" -C /opt/zig --strip-components=1 --no-same-owner \
 && rm "$file"
ENV PATH=/opt/zig:$PATH
