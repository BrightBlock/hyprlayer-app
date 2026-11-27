# The Linux builder plus the dev libraries GPUI links, in a second amd64 sysroot.
FROM hyprlayer-builder/linux:latest
RUN mmdebstrap --variant=extract --architectures=amd64 \
      --include=libc6-dev,libxkbcommon-dev,libxkbcommon-x11-dev,libwayland-dev,libxcb1-dev,libxcb-xkb-dev,libx11-xcb-dev,libfontconfig-dev,libfreetype-dev,libssl-dev,libvulkan-dev,libdrm-dev \
      bookworm /sysroot-gpui http://deb.debian.org/debian \
 && find /sysroot-gpui -type l -lname '/*' -exec sh -c 'ln -sfn "/sysroot-gpui$(readlink "$1")" "$1"' _ {} \;
ENV PKG_CONFIG_SYSROOT_DIR=/sysroot-gpui \
    PKG_CONFIG_LIBDIR=/sysroot-gpui/usr/lib/x86_64-linux-gnu/pkgconfig:/sysroot-gpui/usr/share/pkgconfig \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=--sysroot=/sysroot-gpui" \
    CFLAGS_x86_64_unknown_linux_gnu=--sysroot=/sysroot-gpui
ENTRYPOINT []
