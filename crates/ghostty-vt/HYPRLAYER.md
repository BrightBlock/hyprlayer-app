herdr's `crates/ghostty-vt` at `e8d7d970`, with its vendored libghostty-vt in
`vendor/` at the repository root, laid out as herdr has them, for the server's
per-PTY terminal screen (see `NOTICE` for provenance and licences).

Building it needs Zig 0.16.0. `mise install` in the repository installs the pinned
one; `ZIG` overrides the binary. For `x86_64-pc-windows-msvc`, `build.rs` points
Zig at the SDK cargo-xwin downloaded (`XWIN_CACHE_DIR`, or cargo-xwin's default
cache); `LIBGHOSTTY_VT_WINDOWS_LIBC` overrides that with a Zig libc file. Zig
builds a copy of the source in `OUT_DIR`, because Zig 0.16 unpacks its
dependencies into the build root and the checkout may be read-only. The first
build downloads eight packages (`uucode`, `translate_c`, `aro` and five source
archives) into Zig's global cache.

What changed from herdr:

- Patch 0007 (experimental Kitty PNG retention and host file snapshots) is
  reverted from `vendor/libghostty-vt` and removed from the patch list, with the
  binding code that used it (`native_source.rs`, `native_image_sources.rs`,
  `pane_graphics_files.rs`, their tests, and the encoded-PNG and source-file
  fields of the Kitty placement types). `bindings.rs` loses exactly the items
  0007 added. Patches 0002, 0004, 0005, 0006 and 0008 stay applied.
- The Kitty graphics tests in `lib.rs` are gone, and `base64`, `libc` and
  `tracing` with them. The PNG decoder's two `chunks_exact` loops use
  `as_chunks`, which clippy 1.99 asks for. herdr's `clippy.toml` comes along.
- `build.rs` builds from a copy in `OUT_DIR`, keeps Zig's cache there, and finds
  the cargo-xwin SDK.
- Patch 0009 (Hyprlayer's own) leaves kitty keyboard and XTVERSION queries
  unanswered. `lib.rs` gains `set_default_colors`, `for_each_viewport_row_cell`
  and a test for the patch.
- Patch 0010 (Hyprlayer's own) counts rows scrolled into history and history
  erases, and reports the cursor's DECSCUSR shape. `lib.rs` gains
  `history_pushed`, `history_cleared`, `cursor_shape`, `cursor_x`,
  `cursor_visible`, `for_each_row_cell` (a viewport or history row's cells
  with style and link) and a test; `bindings.rs` gains the three data kinds.
- The vendored tree keeps only what a lib-vt build reads: 508 of its 1,339 files
  are gone (19.6 MB down to 15.2 MB), all of them files that no cold
  `zig build -Demit-lib-vt` for x86_64-linux-gnu, aarch64-linux-gnu,
  aarch64-macos or x86_64-windows-msvc opened (watched with inotify). They are
  Ghostty's docs, CI and Nix files, CMake and Make support, the app's C header,
  shell integration, shaders and GTK UI files, the `+boo` animation frames,
  font and test fixtures, and C headers of the app's libraries. Every `.zig`
  file stays, opened or not, because a build runner on another host can import
  more of them; so do the licence files and `.gitignore`. Ghostty's own
  `zig build test-lib-vt` was not run on the trimmed tree and may miss fixtures.

To regenerate `bindings.rs` after a header change, run herdr's command from the
repository root with bindgen-cli 0.72.1:

    bindgen vendor/libghostty-vt/include/ghostty/vt.h --allowlist-type 'Ghostty.*' \
      --allowlist-function 'ghostty_.*' --allowlist-var 'GHOSTTY_.*' --with-derive-default \
      --output crates/ghostty-vt/src/bindings.rs -- -Ivendor/libghostty-vt/include

A newer clang than herdr's attaches doc comments to the `#define` constants, so
the output differs from the committed file in comments only.
