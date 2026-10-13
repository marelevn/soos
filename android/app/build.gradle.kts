plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The one place the version is written is the workspace's Cargo.toml: the
// desktop apps, the CLI and this app all say the same thing.
val (major, minor, patch) = Regex("""(?m)^version = "(\d+)\.(\d+)\.(\d+)"""")
    .find(rootDir.parentFile.resolve("Cargo.toml").readText())!!
    .destructured
    .toList()
    .map { it.toInt() }

android {
    namespace = "io.github.marelevn.soos"
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "io.github.marelevn.soos"
        minSdk = 26
        targetSdk = 37
        versionCode = major * 10_000 + minor * 100 + patch
        versionName = "$major.$minor.$patch"
        // soos-core is built for this one, and AndroidX brings libraries for
        // the others: without this, a phone of another kind is offered an
        // app that closes at launch.
        ndk { abiFilters += "arm64-v8a" }
    }

    signingConfigs {
        // Set by the release workflow; a local release build is unsigned.
        System.getenv("ANDROID_KEYSTORE_PATH")?.let { path ->
            create("release") {
                storeFile = file(path)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release")
        }
    }

    lint {
        // 64-bit ARM only is on purpose; see abiFilters.
        disable += "ChromeOsAbiSupport"
    }

    buildFeatures {
        compose = true
    }

    sourceSets["main"].assets.directories.add("../../assets/fonts")
}

// soos-core is built for the phone's CPU only. Always in release mode, also
// for a debug app: a debug fend is too slow for the 2-second line limit.
val cargoNdk = tasks.register<Exec>("cargoNdk") {
    workingDir = rootDir.parentFile
    commandLine(
        "cargo", "ndk", "-t", "arm64-v8a", "--platform", "26",
        "-o", "android/app/src/main/jniLibs",
        "build", "--release", "-p", "soos-android",
    )
}
tasks.named("preBuild") { dependsOn(cargoNdk) }

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")

    // Android's own org.json is a stub in unit tests.
    testImplementation("org.json:json:20260814")
    testImplementation("junit:junit:4.13.2")
}
