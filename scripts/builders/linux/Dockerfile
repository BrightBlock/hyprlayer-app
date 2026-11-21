# Cross-builds the Linux deb/rpm at native arm64 speed against a bookworm amd64
# sysroot: no Rosetta, and the binaries need only glibc 2.34.
ARG RUST_VERSION=1.99.0
FROM rust:${RUST_VERSION}-slim-bookworm

ARG NODE_VERSION=24.21.0
ARG PNPM_VERSION=12.8.1

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      gcc-x86-64-linux-gnu pkg-config libarchive-tools mmdebstrap curl xz-utils \
 && rm -rf /var/lib/apt/lists/*

# Extracted, never run, so the sysroot's amd64-only build tools need no
# emulation. ld cannot follow its absolute symlinks out of /sysroot.
RUN mmdebstrap --variant=extract --architectures=amd64 \
      --include=libc6-dev,libwebkit2gtk-4.1-dev,libgtk-3-dev,librsvg2-dev \
      bookworm /sysroot http://deb.debian.org/debian \
 && find /sysroot -type l -lname '/*' -exec sh -c 'ln -sfn "/sysroot$(readlink "$1")" "$1"' _ {} \;

RUN arch="$(dpkg --print-architecture | sed 's/amd64/x64/')" \
 && file="node-v$NODE_VERSION-linux-$arch.tar.xz" \
 && curl -fsSLO "https://nodejs.org/dist/v$NODE_VERSION/$file" \
 && curl -fsSL "https://nodejs.org/dist/v$NODE_VERSION/SHASUMS256.txt" | grep " $file\$" | sha256sum -c - \
 && tar -xJf "$file" -C /usr/local --strip-components=1 --no-same-owner \
 && rm "$file" \
 && npm install -g "pnpm@$PNPM_VERSION" \
 && rustup target add x86_64-unknown-linux-gnu

# hyprlayer-core's libghostty-vt is built by Zig, pinned in the repo's mise.toml.
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

ENV PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_SYSROOT_DIR=/sysroot \
    PKG_CONFIG_LIBDIR=/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig:/sysroot/usr/share/pkgconfig \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=--sysroot=/sysroot" \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CFLAGS_x86_64_unknown_linux_gnu=--sysroot=/sysroot \
    AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar

COPY build.sh /usr/local/bin/hyprlayer-build
COPY build-server.sh /usr/local/bin/hyprlayer-build-server
ENTRYPOINT ["bash", "/usr/local/bin/hyprlayer-build"]
