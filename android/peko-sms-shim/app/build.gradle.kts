plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace  = "com.peko.shim.sms"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.peko.shim.sms"
        // API 26 covers every Android 8+ device. ISms transaction codes are
        // stable enough across 26→35 that we don't need version-specific
        // handling in the app itself.
        minSdk        = 26
        targetSdk     = 35
        versionCode   = 1
        versionName   = "0.1.0"
    }

    buildTypes {
        release {
            // No minification — there's basically nothing to shrink. A single
            // receiver class + result writer. Keeping ProGuard off also means
            // the SmsManager reflection we do in SmsCommandReceiver can't be
            // broken by R8 renaming the androidx shim.
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
}
