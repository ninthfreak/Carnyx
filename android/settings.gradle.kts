// The Gradle half of the build, and ONLY the packaging half.
//
// Rust still compiles the library — `cargo ndk` cross-compiles `libcarnyx.so`
// for both ABIs and drops them in `app/src/main/jniLibs/`. Gradle's whole job is
// to put that library, the Java, the AIDL and the assets into an APK with a
// manifest it can actually write. See `tools/build-apk-gradle.sh`.
//
// WHY THIS EXISTS AT ALL: cargo-apk cannot declare a `<service>` or a
// `<receiver>`. `ndk_build::manifest::Application` has one activity and no field
// for anything else, and the manifest is generated from `[package.metadata.android]`
// with no escape hatch. A foreground service is the difference between Carnyx
// surviving being switched away from and redrawing its whole face when you come
// back, so the packager had to change. This directory is that change.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "carnyx"
include(":app")
