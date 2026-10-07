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

ENV PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_SYSROOT_DIR=/sysroot \
    PKG_CONFIG_LIBDIR=/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig:/sysroot/usr/share/pkgconfig \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=--sysroot=/sysroot" \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CFLAGS_x86_64_unknown_linux_gnu=--sysroot=/sysroot \
    AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar

COPY build.sh /usr/local/bin/hyprlayer-build
ENTRYPOINT ["bash", "/usr/local/bin/hyprlayer-build"]
