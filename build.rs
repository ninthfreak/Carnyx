//! Carnyx's build script: the Slint compile, and — for Android targets only —
//! the Java this library loads at run time, turned into a dex that gets embedded
//! in the shared library.
//!
//! THERE ARE TWO JAVA TREES AND THIS ONE COMPILES ONLY ITS OWN. `java/` is what
//! is dexed and embedded here: the NWD tuner bridge, HTTPS, location, and the
//! starter for the foreground service. `android/app/src/main/java/` is compiled
//! by AGP into the APK's own dex instead, and holds exactly one class,
//! `CarnyxService`.
//!
//! A manifest-declared component has to be constructible by the APPLICATION's
//! class loader, which has never heard of the `InMemoryDexClassLoader` this
//! file's output is loaded into. Under the GRADLE build that alone does not rule
//! out `java/`, because `android/app/build.gradle.kts` puts that tree in the
//! Gradle java source set as well and AGP compiles it into the APK dex too — so
//! a service there would work, at the cost of being compiled twice and of being
//! invisible under cargo-apk, which packages no Java at all. Gradle-only code
//! stays with the Gradle build; that is a choice, not an impossibility.
//!
//! ## Why a build script compiles Java at all
//!
//! Carnyx is a pure NativeActivity built by cargo-apk, and cargo-apk packages no
//! Java and no dex — neither it nor ndk-build mentions dex anywhere. The NWD
//! tuner is an AIDL binder service, and there is no NDK path to binder, so that
//! looked like a hard blocker.
//!
//! It is not, and the answer is not invented here. Slint's own Android backend
//! solves the identical problem and this is a copy of its mechanism: compile the
//! Java with `android_build::JavaBuild` (javac), convert it with
//! `android_build::Dexer` (d8), write `$OUT_DIR/classes.dex`, and have the Rust
//! side `include_bytes!` it and load it at run time with
//! `InMemoryDexClassLoader`. The reference is
//! `i-slint-backend-android-activity-1.17.1`'s own `build.rs` and
//! `javahelper.rs`, which ship this exact pattern on this exact device.
//!
//! One step is ours rather than Slint's: the AIDL. The vendor interface is
//! declared in `.aidl` files, so the `aidl` compiler from the SDK build-tools
//! runs first and generates the `Stub`/`Proxy` Java that javac then sees.
//!
//! ## What this needs installed
//!
//! An Android SDK with a `platforms/android-NN/android.jar`, a `build-tools/NN`
//! containing both `aidl` and `lib/d8.jar`, and a JDK. All are located the way
//! `android-build` locates them — `ANDROID_HOME`/`ANDROID_SDK_ROOT`, then the
//! default install path — with `ANDROID_AIDL` available as a direct override for
//! the one binary `android-build` does not know about.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use android_build::{Dexer, JavaBuild};

/// The Java sources that are ours, relative to `java/` — the RUNTIME dex only;
/// see the header for the tree this one is not. The two `com/nwd/...`
/// parcelables are vendor interface code under the interop scope rule recorded
/// in their own headers; they are listed here because javac needs them, not
/// because they are ours.
const JAVA_SOURCES: &[&str] = &[
    "com/ninthfreak/carnyx/NwdBridge.java",
    "com/ninthfreak/carnyx/CarnyxLocation.java",
    "com/ninthfreak/carnyx/CarnyxNet.java",
    "com/ninthfreak/carnyx/CarnyxProcess.java",
    "com/ninthfreak/carnyx/CarnyxAlert.java",
    "com/ninthfreak/carnyx/CarnyxKeepAlive.java",
    "com/ninthfreak/carnyx/CarnyxWake.java",
    "com/nwd/radio/service/data/Frequency.java",
    "com/nwd/radio/service/data/RadioPoint.java",
];

/// The AIDL interfaces to generate Java from. The two `parcelable` declarations
/// beside them (`data/Frequency.aidl`, `data/RadioPoint.aidl`) generate no code
/// — they only tell `aidl` that those types are parcelable — so they are inputs
/// to the include path rather than arguments.
const AIDL_INTERFACES: &[&str] = &[
    "com/nwd/radio/service/RadioFeature.aidl",
    "com/nwd/radio/service/RadioCallback.aidl",
];

/// Escape hatch for a checkout with no Android SDK.
///
/// Set it and an Android-target build writes an EMPTY `classes.dex` instead of
/// failing, so the Rust half still compiles and `cargo check
/// --target armv7-linux-androideabi` is usable on a machine that has no SDK —
/// which is the only way this code gets compile-checked in CI or a container.
///
/// The resulting library CANNOT drive the tuner: `src/android/dex.rs` refuses an
/// empty dex with a message naming this variable, so the failure is a legible
/// one at start-up instead of a `ClassNotFoundException` in a car.
const ALLOW_MISSING_DEX: &str = "CARNYX_ALLOW_MISSING_DEX";

fn main() {
    slint_build::compile("ui/app.slint").expect("Slint compilation failed");

    println!("cargo:rerun-if-env-changed={ALLOW_MISSING_DEX}");
    println!("cargo:rerun-if-changed=java");

    // The dex is only meaningful on Android; on the host there is no JVM to load
    // it into and the module that embeds it is `cfg`'d out entirely.
    if !env::var("TARGET").unwrap_or_default().contains("android") {
        return;
    }

    let out_dir: PathBuf = env::var_os("OUT_DIR").expect("OUT_DIR is always set").into();
    let dex_path = out_dir.join("classes.dex");

    let sdk_missing = android_build::android_jar(None).is_none();
    if sdk_missing && env::var_os(ALLOW_MISSING_DEX).is_some() {
        println!(
            "cargo:warning=No Android SDK found and {ALLOW_MISSING_DEX} is set: writing an EMPTY \
             classes.dex. This library will refuse to start the NWD tuner."
        );
        std::fs::write(&dex_path, []).expect("cannot write the placeholder classes.dex");
        return;
    }

    let android_jar = android_build::android_jar(None).unwrap_or_else(|| {
        panic!(
            "No Android platform found. Install an SDK platform and set ANDROID_HOME, or set \
             {ALLOW_MISSING_DEX}=1 to build a library that cannot drive the tuner."
        )
    });

    let java_root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("java");
    let gen_dir = out_dir.join("aidl-java");
    let class_dir = out_dir.join("classes");

    // Both trees are rebuilt from scratch. javac and d8 are additive — a stale
    // `.class` from a renamed source would be dexed straight into the library
    // and only show up as a wrong method at run time.
    for dir in [&gen_dir, &class_dir] {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|e| panic!("cannot create {}: {e}", dir.display()));
    }

    let aidl = find_aidl().unwrap_or_else(|| {
        panic!(
            "The `aidl` compiler was not found in the SDK build-tools. Set ANDROID_AIDL to it, or \
             set {ALLOW_MISSING_DEX}=1 to build a library that cannot drive the tuner."
        )
    });

    for iface in AIDL_INTERFACES {
        let status = Command::new(&aidl)
            .arg(format!("-I{}", java_root.display()))
            .arg(format!("-o{}", gen_dir.display()))
            .arg(java_root.join(iface))
            .status()
            .unwrap_or_else(|e| panic!("could not run {}: {e}", aidl.display()));
        if !status.success() {
            panic!("aidl failed on {iface}");
        }
    }

    let mut sources: Vec<PathBuf> = JAVA_SOURCES.iter().map(|s| java_root.join(s)).collect();
    collect_java(&gen_dir, &mut sources);

    let release = env::var("PROFILE").as_deref() == Ok("release");

    // Java 8 source/target, matching Slint's helper: d8 desugars Java 8, and
    // anything newer risks a class-file version d8 in an older build-tools
    // refuses.
    let out = JavaBuild::new()
        .files(&sources)
        .class_path(&android_jar)
        .classes_out_dir(&class_dir)
        .java_source_version(8)
        .java_target_version(8)
        .debug_info(android_build::DebugInfo {
            line_numbers: !release,
            variables: !release,
            source_files: !release,
        })
        .command()
        .unwrap_or_else(|e| panic!("could not build the javac command: {e}"))
        .args(["-encoding", "UTF-8"])
        .output()
        .unwrap_or_else(|e| panic!("could not run javac: {e}"));
    if !out.status.success() {
        panic!("Java compilation failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    let out = Dexer::new()
        .android_jar(&android_jar)
        .class_path(&class_dir)
        .collect_classes(&class_dir)
        .expect("no .class files were produced")
        .release(release)
        // 20 disables multidex, which is what guarantees a single `classes.dex`
        // for `include_bytes!` to pick up. It is not an API floor for the app.
        .android_min_api(20)
        .out_dir(&out_dir)
        .command()
        .unwrap_or_else(|e| panic!("could not build the D8 command: {e}"))
        .output()
        .unwrap_or_else(|e| panic!("could not run D8: {e}"));
    if !out.status.success() {
        eprintln!("Dex conversion failed: {}", String::from_utf8_lossy(&out.stderr));
        // Slint's build script carries this same diagnosis, and it is worth
        // repeating because the error d8 prints does not point at the cause.
        if let Some(home) = android_build::java_home() {
            if let Ok(ver) = android_build::check_javac_version(&home) {
                if ver >= 21 {
                    eprintln!(
                        "JDK {ver} is known to fail with d8 from build-tools older than 35.0.0 \
                         when the sources contain anonymous classes — which these do. Use JDK 17, \
                         or install build-tools 35."
                    );
                }
            }
        }
        panic!("Dex conversion failed");
    }

    assert!(dex_path.is_file(), "d8 did not write {}", dex_path.display());
}

/// Locate the `aidl` binary.
///
/// `android-build` knows how to find `android.jar` and `d8.jar` but not this —
/// AIDL is not part of what Slint's backend needs — so the same search is done
/// here: an explicit override first, then the highest build-tools directory that
/// actually contains the binary.
fn find_aidl() -> Option<PathBuf> {
    println!("cargo:rerun-if-env-changed=ANDROID_AIDL");
    if let Some(p) = env::var_os("ANDROID_AIDL").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = if cfg!(windows) { "aidl.exe" } else { "aidl" };
    let build_tools = android_build::android_sdk()?.join("build-tools");
    if let Ok(requested) = env::var("ANDROID_BUILD_TOOLS_VERSION") {
        let p = build_tools.join(requested).join(exe);
        if p.is_file() {
            return Some(p);
        }
    }
    std::fs::read_dir(&build_tools)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join(exe).is_file())
        .map(|e| e.file_name())
        .max()
        .map(|name| build_tools.join(name).join(exe))
}

/// Every `.java` under `dir`, recursively. The AIDL compiler lays its output out
/// by package, so the generated files are several directories down.
fn collect_java(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_java(&path, out);
        } else if path.extension().is_some_and(|e| e == "java") {
            out.push(path);
        }
    }
}
