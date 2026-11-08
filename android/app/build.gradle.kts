import com.android.build.api.dsl.ApplicationExtension

plugins {
    id("com.android.application")
}

// Version comes from the repository's VERSION file (e.g. 0.11.4), or from
// VIVANT_VERSION (v0.11.4) in CI, so the APK always matches the server.
val rawVersion = (System.getenv("VIVANT_VERSION") ?: rootProject.file("../VERSION").readText())
    .trim().removePrefix("v")
val versionNums = Regex("""^(\d+)\.(\d+)\.(\d+)""").find(rawVersion)?.destructured
val code = versionNums?.let { (a, b, c) -> a.toInt() * 10000 + b.toInt() * 100 + c.toInt() } ?: 1

configure<ApplicationExtension> {
    namespace = "io.github.freecode100year.vivant"
    // tor-android is built against API 37.1
    compileSdk {
        version = release(37) {
            minorApiLevel = 2
        }
    }

    defaultConfig {
        applicationId = "io.github.freecode100year.vivant"
        minSdk = 26
        targetSdk = 35
        versionCode = code
        versionName = rawVersion
    }

    // Release signing: set VIVANT_KEYSTORE (path), VIVANT_KEYSTORE_PASSWORD,
    // VIVANT_KEY_ALIAS, VIVANT_KEY_PASSWORD. Without them the release APK is
    // signed with the debug key (fine for trying it out; updates need the
    // same key every time).
    val ks = System.getenv("VIVANT_KEYSTORE")
    signingConfigs {
        create("release") {
            if (!ks.isNullOrEmpty()) {
                storeFile = file(ks)
                storePassword = System.getenv("VIVANT_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("VIVANT_KEY_ALIAS")
                keyPassword = System.getenv("VIVANT_KEY_PASSWORD") ?: System.getenv("VIVANT_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName(if (ks.isNullOrEmpty()) "debug" else "release")
        }
    }

    // Two editions: "full" (Tor onion service, EasyTier, FRP) and "easytier"
    // (EasyTier only: smaller, simpler). They install side by side.
    flavorDimensions += "edition"
    productFlavors {
        create("full") {
            dimension = "edition"
        }
        create("easytier") {
            dimension = "edition"
            applicationIdSuffix = ".easytier"
            versionNameSuffix = "-easytier"
        }
    }

    // One APK per CPU type keeps downloads small; the universal one works everywhere.
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "armeabi-v7a", "x86_64")
            isUniversalApk = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The server and tunnel clients are real executables (lib*.so) started as
    // child processes; they must be extracted to nativeLibraryDir, the only
    // place an app may execute files from.
    packaging {
        jniLibs {
            useLegacyPackaging = true
            excludes += "lib/x86/**" // tor-android ships 32-bit x86 too; nobody needs it
        }
    }

    lint {
        abortOnError = false
        checkReleaseBuilds = false
    }
}

dependencies {
    // full edition only: Tor itself (the same build Orbot uses) and its control-port library
    add("fullImplementation", "info.guardianproject:tor-android:0.4.9.14")
    add("fullImplementation", "info.guardianproject:jtorctl:0.4.5.7")
    // Snowflake / obfs4 / webtunnel bridges for censored networks (as in Orbot)
    add("fullImplementation", "com.netzarchitekten:IPtProxy:5.7.0")
}
