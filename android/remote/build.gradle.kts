plugins {
    id("com.android.application")
}

android {
    namespace = "io.github.aaron_gh.aae.remote"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.aaron_gh.aae.remote"
        // Android 8: AudioTrack's low-latency mode and the vibration effects
        // it uses. Gesture mode's touch passthrough needs Android 11.
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }

    // Signed with AAE's own key, as the helper is, so updates install over
    // each other; without it, this computer's debug key.
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

dependencies {
    implementation("com.squareup.okhttp3:okhttp:5.1.0")
}
