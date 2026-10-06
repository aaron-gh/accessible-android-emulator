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
        versionCode = 15
        versionName = "0.15.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Signed with the debug key until AAE has a release key. It only
            // ever runs on AAE's own virtual devices.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
