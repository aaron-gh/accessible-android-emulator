plugins {
    id("com.android.application")
}

android {
    namespace = "io.github.aaron_gh.aae.helper"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.aaron_gh.aae.helper"
        // As old as AAE supports, so the helper works on every device.
        minSdk = 21
        targetSdk = 36
        // Raise versionCode with every change: AAE updates devices whose helper is older.
        versionCode = 17
        versionName = "0.17.0"
    }

    // AAE's own key, so every build of AAE, here or on GitHub, signs its apps
    // the same way, and devices accept each new helper as an update. Its
    // password comes from AAE_ANDROID_KEY_PASSWORD; macos/build.sh and
    // windows/build.sh read it from the Keychain. Without the key, builds are
    // signed with this computer's debug key instead.
    val aaeKey = file(System.getenv("AAE_ANDROID_KEYSTORE") ?: "${System.getProperty("user.home")}/.android/aae.keystore")
    val aaeKeyPassword: String? = System.getenv("AAE_ANDROID_KEY_PASSWORD")
    signingConfigs {
        create("aae") {
            storeFile = aaeKey
            storePassword = aaeKeyPassword
            keyAlias = "aae"
            keyPassword = aaeKeyPassword
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // It only ever runs on AAE's own virtual devices.
            signingConfig = if (aaeKey.isFile && aaeKeyPassword != null) {
                signingConfigs.getByName("aae")
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
