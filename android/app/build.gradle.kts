plugins {
    id("com.android.application")
}

android {
    // AGP 8 took `package` out of the manifest; this is where it lives now. It
    // must stay `com.ninthfreak.carnyx` — the same id cargo-apk emits — or the
    // unit treats a Gradle APK as a different app and installs it alongside the
    // cargo-apk one instead of over it.
    namespace = "com.ninthfreak.carnyx"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.ninthfreak.carnyx"
        // 26 is skia-bindings' hardcoded floor, not a preference — see the long
        // note in ../../Cargo.toml. The manifest and the .so have to agree on it.
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        // Tracks `version` in ../../Cargo.toml. Two places, deliberately: Gradle
        // cannot read Cargo.toml without a parser, and a wrong string here is
        // cosmetic where a wrong `minSdk` is not.
        versionName = "0.1.0"

        // THE HEAD UNIT IS 32-BIT ARM. Dropping armeabi-v7a produces an APK that
        // will not install at all (INSTALL_FAILED_NO_MATCHING_ABIS), which the
        // on-device installer reports only as "App not installed".
        ndk {
            abiFilters += listOf("armeabi-v7a", "arm64-v8a")
        }
    }

    // The vendor tuner is an AIDL binder service. Gradle compiles `.aidl`
    // natively, which is the job `build.rs` currently does by hand with the SDK's
    // `aidl` binary and `d8`.
    buildFeatures {
        aidl = true
    }

    sourceSets["main"].apply {
        // The Java tree stays where it is, at the repository root, so the
        // cargo-apk build keeps working unchanged while this is a spike. Both
        // srcDirs point at the same folder on purpose: Gradle takes `.aidl` from
        // the aidl set and `.java` from the java set, and `java/com/nwd/radio/`
        // holds both — `Frequency.aidl` declares the parcelable, `Frequency.java`
        // implements it.
        java.srcDirs("../../java")
        aidl.srcDirs("../../java")
        // `assets/db/stations.sqlite` → the APK asset path `db/stations.sqlite`,
        // which is where `stations::install` looks for it.
        assets.srcDirs("../../assets")
        // Written by `cargo ndk -o`, not by hand, and gitignored. See
        // tools/build-apk-gradle.sh.
        jniLibs.srcDirs("src/main/jniLibs")
    }

    androidResources {
        // The station table must be STORED, not deflated. A deflated asset has no
        // file descriptor to hand SQLite. `stations::install` copies it out to
        // sidestep that anyway, but the copy is cheaper from a stored file and
        // this is the bug cargo-apk hits only in release builds.
        noCompress += "sqlite"
    }

    // d8 desugars Java 8; anything newer risks a class-file version the SDK's
    // dexer in an older build-tools cannot read. Matches what build.rs asks javac
    // for today.
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }

    packaging {
        jniLibs {
            // Extract the .so at install time rather than loading it from a
            // compressed APK. NativeActivity resolves the library through
            // System.loadLibrary, and the extracted path is the one that has
            // always worked here — cargo-apk leaves the attribute unset, which
            // gives the same behaviour.
            useLegacyPackaging = true
        }
    }

    buildTypes {
        debug {
            // Debug builds are signed with the standard debug keystore, which is
            // what makes `./gradlew assembleDebug` produce something installable
            // with no key material anywhere near this repository.
            isJniDebuggable = true
        }
        release {
            isMinifyEnabled = false
            // NO signingConfig HERE ON PURPOSE. Release signing reads a keystore
            // that lives OUTSIDE the repository, through the environment, and is
            // wired up when there is a release to make — see #68. Naming a
            // keystore path or a password in a tracked file is the thing this
            // project has refused from the start.
        }
    }
}
