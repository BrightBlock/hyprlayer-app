# Cross-builds the Windows NSIS installer at native arm64 speed: cargo-xwin
# supplies the MSVC CRT and Windows SDK, and Debian's makensis works where the
# macOS arm64 one crashes.
ARG RUST_VERSION=1.99.0
FROM rust:${RUST_VERSION}-slim-trixie

ARG LLVM_VERSION=19
ARG NODE_VERSION=24.21.0
ARG PNPM_VERSION=12.8.1
ARG CARGO_XWIN_VERSION=0.23.1

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      "clang-$LLVM_VERSION" "lld-$LLVM_VERSION" "llvm-$LLVM_VERSION" nsis curl xz-utils \
 && rm -rf /var/lib/apt/lists/*
# The unversioned clang-cl, lld-link, llvm-rc and llvm-lib exist only here.
ENV PATH=/usr/lib/llvm-$LLVM_VERSION/bin:$PATH

RUN arch="$(dpkg --print-architecture | sed 's/amd64/x64/')" \
 && file="node-v$NODE_VERSION-linux-$arch.tar.xz" \
 && curl -fsSLO "https://nodejs.org/dist/v$NODE_VERSION/$file" \
 && curl -fsSL "https://nodejs.org/dist/v$NODE_VERSION/SHASUMS256.txt" | grep " $file\$" | sha256sum -c - \
 && tar -xJf "$file" -C /usr/local --strip-components=1 --no-same-owner \
 && rm "$file" \
 && npm install -g "pnpm@$PNPM_VERSION"

RUN file="cargo-xwin-v$CARGO_XWIN_VERSION.$(uname -m)-unknown-linux-musl.tar.gz" \
 && base="https://github.com/rust-cross/cargo-xwin/releases/download/v$CARGO_XWIN_VERSION" \
 && curl -fsSLO "$base/$file" \
 && curl -fsSL "$base/$file.sha256" | sha256sum -c - \
 && tar -xzf "$file" -C /usr/local/cargo/bin \
 && rm "$file" \
 && rustup target add x86_64-pc-windows-msvc

# Baked into the image so a release neither downloads the SDK nor silently
# picks up a newer one.
ENV XWIN_CACHE_DIR=/opt/xwin
RUN cargo xwin cache xwin

COPY build.sh /usr/local/bin/hyprlayer-build
ENTRYPOINT ["bash", "/usr/local/bin/hyprlayer-build"]
