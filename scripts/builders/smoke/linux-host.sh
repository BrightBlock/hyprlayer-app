#!/bin/sh
# One distro's leg of scripts/builders/smoke/linux.sh: packages in /pkg, results in /out.
set -eu

: "${VERSION:?}"
DEB="/pkg/Hyprlayer_${VERSION}_amd64.deb"
SERVER="/pkg/hyprlayer-server-x86_64-unknown-linux-gnu"

check_installed() {
  echo "server --version: $(hyprlayer-server --version)"
  missing="$(ldd /usr/bin/Hyprlayer | grep 'not found' || true)"
  [ -z "$missing" ] || { echo "FAIL unresolved libraries: $missing"; exit 1; }
  echo "PASS every library resolves"
  [ "$(sha256sum < /usr/bin/hyprlayer-server)" = "$(sha256sum < "$SERVER")" ] \
    || { echo "FAIL installed server differs from the release asset"; exit 1; }
  echo "PASS installed server matches the release asset"
}

case "$1" in
  deb)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq >/dev/null
    apt-get install -y -qq "$DEB" >/tmp/install.log 2>&1 || { tail -30 /tmp/install.log; exit 1; }
    . /etc/os-release
    echo "PASS installed $(dpkg-query -W -f='${Package} ${Version}' hyprlayer) on $PRETTY_NAME"
    check_installed ;;
  rpm)
    RPM="$(ls /pkg/Hyprlayer-"$VERSION"-*.x86_64.rpm | head -n1)"
    dnf install -y -q "$RPM" >/tmp/install.log 2>&1 || { tail -30 /tmp/install.log; exit 1; }
    . /etc/os-release
    echo "PASS installed $(rpm -q hyprlayer) on $PRETTY_NAME"
    check_installed ;;
  aur)
    # Rosetta rejects the x86_64 seccomp filter pacman's download sandbox installs.
    sed -i 's/^#DisableSandboxSyscalls/DisableSandboxSyscalls/' /etc/pacman.conf
    pacman -Syu --noconfirm --needed sudo dbus xorg-server-xvfb xorg-xwd imagemagick ttf-dejavu >/dev/null 2>&1
    useradd -m tester && echo 'tester ALL=(ALL) NOPASSWD: ALL' >/etc/sudoers.d/tester
    cp -r /pkg/aur /home/tester/aur
    # A local file named like the PKGBUILD's source skips the download but not the sha256 check.
    cp "$DEB" /home/tester/aur/
    chown -R tester: /home/tester/aur
    su tester -c 'cd ~/aur && makepkg -si --noconfirm' >/tmp/install.log 2>&1 || { tail -40 /tmp/install.log; exit 1; }
    echo "PASS makepkg verified the deb against the PKGBUILD and installed $(pacman -Q hyprlayer-desktop-bin)"
    check_installed
    su tester -c '
      export XDG_RUNTIME_DIR=/tmp/xdg-tester DISPLAY=:99
      mkdir -p -m 700 "$XDG_RUNTIME_DIR"
      Xvfb :99 -screen 0 1440x900x24 >/dev/null 2>&1 &
      sleep 2
      eval "$(dbus-launch --sh-syntax)"
      WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 \
        /usr/bin/Hyprlayer >/tmp/app.log 2>&1 &
      sleep 60
      xwd -root -silent | magick xwd:- /out/arch-screenshot.png
    '
    # Under Rosetta the process name is rosetta, so match the command line.
    pgrep -f /usr/bin/Hyprlayer >/dev/null || { tail -20 /tmp/app.log; echo "FAIL the app exited"; exit 1; }
    grep -q "connected to this machine" /tmp/app.log || { tail -20 /tmp/app.log; echo "FAIL the app never reached its server"; exit 1; }
    echo "PASS the app launched, started its server and connected: $(grep -o 'connected to this machine[^:]*: server [^,]*' /tmp/app.log | head -n1)" ;;
  *) echo "unknown mode $1" >&2; exit 1 ;;
esac
