// Modified by Hyprlayer from herdr e8d7d970: Zig builds a copy of the source in
// OUT_DIR with its cache there too, and a cargo-xwin SDK becomes Zig's Windows
// libc when LIBGHOSTTY_VT_WINDOWS_LIBC is unset.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn zig_target(target: &str) -> &str {
    match target {
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "x86_64-unknown-linux-musl" => "x86_64-linux-musl",
        "aarch64-unknown-linux-musl" => "aarch64-linux-musl",
        "x86_64-apple-darwin" => "x86_64-macos",
        "aarch64-apple-darwin" => "aarch64-macos",
        "aarch64-apple-ios" => "aarch64-ios",
        "aarch64-apple-ios-sim" => "aarch64-ios-simulator",
        "x86_64-pc-windows-msvc" => "x86_64-windows-msvc",
        "aarch64-pc-windows-msvc" => "aarch64-windows-msvc",
        other => panic!("unsupported target for libghostty-vt build: {other}"),
    }
}

fn env_bool(name: &str) -> Option<bool> {
    match env::var(name) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            other => panic!("invalid boolean value for {name}: {other}"),
        },
        Err(env::VarError::NotPresent) => None,
        Err(err) => panic!("failed to read {name}: {err}"),
    }
}

// Zig ships no MSVC headers, so a cross build needs them from the SDK that
// cargo-xwin downloads, at its XWIN_CACHE_DIR or default cache path.
fn xwin_libc_file(target: &str, out_dir: &Path) -> Option<PathBuf> {
    let cache = match env::var_os("XWIN_CACHE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = PathBuf::from(env::var_os("HOME")?);
            let caches = if cfg!(target_os = "macos") {
                home.join("Library/Caches")
            } else {
                home.join(".cache")
            };
            caches.join("cargo-xwin")
        }
    };
    let xwin = cache.join("xwin");
    if !xwin.join("sdk").is_dir() {
        return None;
    }
    let arch = target.split('-').next()?;
    let dir = |path: &str| xwin.join(path).display().to_string();
    let contents = format!(
        "include_dir={}\nsys_include_dir={}\ncrt_dir={}\nmsvc_lib_dir={}\nkernel32_lib_dir={}\ngcc_dir=\n",
        dir("sdk/include/ucrt"),
        dir("crt/include"),
        dir(&format!("sdk/lib/ucrt/{arch}")),
        dir(&format!("crt/lib/{arch}")),
        dir(&format!("sdk/lib/um/{arch}")),
    );
    let file = out_dir.join("xwin-libc.txt");
    fs::write(&file, contents).expect("failed to write the xwin libc file for Zig");
    Some(file)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("failed to create the libghostty-vt build copy");
    for entry in fs::read_dir(from).expect("failed to read the vendored libghostty-vt") {
        let entry = entry.expect("failed to read the vendored libghostty-vt");
        let name = entry.file_name();
        if name == "zig-pkg" || name == ".zig-cache" || name == "zig-out" {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            copy_tree(&src, &dst);
        } else {
            fs::copy(&src, &dst).expect("failed to copy the vendored libghostty-vt");
        }
    }
}

fn main() {
    // The vendored source stays at the repository root, shared with its maintenance scripts.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor_dir = manifest_dir.join("../../vendor");
    let vendored_dir = vendor_dir.join("libghostty-vt");

    println!("cargo:rerun-if-changed=build.rs");
    for path in [
        vendor_dir.join("libghostty-vt.vendor.json"),
        vendored_dir.join("build.zig"),
        vendored_dir.join("build.zig.zon"),
        vendored_dir.join("include"),
        vendored_dir.join("pkg"),
        vendored_dir.join("src"),
        vendored_dir.join("VERSION"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_OPTIMIZE");
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_SIMD");
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_ZIG_SYSTEM_DIR");
    println!("cargo:rerun-if-env-changed=ZIG");
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_WINDOWS_LIBC");
    println!("cargo:rerun-if-env-changed=XWIN_CACHE_DIR");

    let optimize = env::var("LIBGHOSTTY_VT_OPTIMIZE").unwrap_or_else(|_| "ReleaseFast".into());
    let simd = env_bool("LIBGHOSTTY_VT_SIMD").unwrap_or(true);
    let target = env::var("TARGET").expect("TARGET");
    let zig_target = zig_target(&target);
    let version_string = fs::read_to_string(vendored_dir.join("VERSION"))
        .expect("failed to read vendored libghostty-vt VERSION")
        .trim()
        .to_string();

    // Install into this build's OUT_DIR so concurrent builds for different targets
    // from one checkout never link each other's archive.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let zig_prefix = out_dir.join("zig-out");

    // Zig 0.16 unpacks dependencies into <build root>/zig-pkg, so build a copy:
    // the checkout stays clean and may be mounted read-only.
    let build_root = out_dir.join("libghostty-vt");
    if build_root.exists() {
        fs::remove_dir_all(&build_root).expect("failed to clear the libghostty-vt build copy");
    }
    copy_tree(&vendored_dir, &build_root);

    let zig = env::var("ZIG").unwrap_or_else(|_| "zig".into());
    let mut command = Command::new(&zig);
    command
        .arg("build")
        .arg("--prefix")
        .arg(&zig_prefix)
        .arg("--cache-dir")
        .arg(out_dir.join("zig-cache"))
        .arg("-Demit-lib-vt")
        .arg(format!("-Doptimize={optimize}"))
        .arg(format!("-Dsimd={simd}"))
        .arg(format!("-Dtarget={zig_target}"))
        .arg(format!("-Dversion-string={version_string}"))
        .arg("-Demit-xcframework=false");
    if target.ends_with("windows-msvc") {
        let libc_file = match env::var_os("LIBGHOSTTY_VT_WINDOWS_LIBC") {
            Some(file) => {
                println!("cargo:rerun-if-changed={}", PathBuf::from(&file).display());
                Some(PathBuf::from(file))
            }
            None => xwin_libc_file(&target, &out_dir),
        };
        if let Some(libc_file) = libc_file {
            command.arg("--libc").arg(libc_file);
        }
    }
    if let Ok(system_dir) = env::var("LIBGHOSTTY_VT_ZIG_SYSTEM_DIR") {
        command.arg("--system").arg(system_dir);
    }

    let status = command
        .current_dir(&build_root)
        .status()
        .unwrap_or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                panic!(
                    "zig executable not found (looked for {zig:?}; set the ZIG \
                     environment variable to point at the zig binary). Building \
                     the vendored libghostty-vt requires Zig 0.16.0: `mise install` \
                     in the repository installs the pinned one, then retry the build"
                );
            }
            panic!("failed to execute zig build for vendored libghostty-vt: {err}");
        });
    assert!(
        status.success(),
        "zig build for vendored libghostty-vt failed: {status}. \
         It requires Zig 0.16.0; check `zig version` \
         or set ZIG to the path of a Zig 0.16.0 binary, then retry"
    );

    let mut lib_dir = zig_prefix.join("lib");
    if target.contains("-apple-") {
        // Apple's linker prefers the sibling dylib for `-l`, so search a directory
        // holding only the static archive.
        let static_dir = out_dir.join("lib");
        fs::create_dir_all(&static_dir).expect("failed to create libghostty-vt link directory");
        fs::copy(
            lib_dir.join("libghostty-vt.a"),
            static_dir.join("libghostty-vt.a"),
        )
        .expect("failed to copy libghostty-vt static archive");
        lib_dir = static_dir;
    }
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-lib=static=ghostty-vt-static");
    } else {
        println!("cargo:rustc-link-lib=static=ghostty-vt");
    }
}
