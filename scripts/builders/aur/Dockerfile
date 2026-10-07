# Renders .SRCINFO beside each PKGBUILD under /out with Arch's own makepkg;
# Debian's 6.0.2 fails on these PKGBUILDs. amd64 only, but a two-second bash
# step, so emulation speed does not matter.
FROM archlinux:base

# makepkg refuses to run as root.
RUN useradd --create-home --uid 1000 builder
USER builder
ENTRYPOINT ["sh", "-c", "for d in /out/aur /out/aur-server; do (cd \"$d\" && makepkg --printsrcinfo > .SRCINFO) || exit 1; done"]
