plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

android {
    namespace  = "com.peko.overlay"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.peko.overlay"
        minSdk        = 26                // Android 8 — ample coverage; required for adaptive icons
        targetSdk     = 35
        versionCode   = 1
        versionName   = "0.1.0"
    }

    // Sign release with the Android debug keystore — correction to an
    // earlier wrong assumption. Android 13 / LineageOS 20 PackageManager
    // silently rejects unsigned priv-apps (no logcat, no toast, just no
    // install). The debug keystore is fine because on a Magisk-rooted
    // device the attacker model is already "root can do anything"; the
    // signature exists only to pass PMS validation.
    signingConfigs {
        create("debug-style") {
            storeFile     = file("${System.getProperty("user.home")}/.android/debug.keystore")
            storePassword = "android"
            keyAlias      = "androiddebugkey"
            keyPassword   = "android"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            signingConfig = signingConfigs.getByName("debug-style")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
    }

    packaging {
        resources {
            excludes += setOf(
                "META-INF/AL2.0",
                "META-INF/LGPL2.1",
                "META-INF/DEPENDENCIES",
            )
        }
    }
}

dependencies {
    // Jetpack Compose BoM keeps all Compose libs on matched versions.
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)

    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.lifecycle:lifecycle-service:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.activity:activity-compose:1.9.3")

    // Compose runtime + UI — no Activity host; we use ComposeView in WindowManager.
    implementation("androidx.compose.runtime:runtime")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.material:material-icons-core")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.animation:animation")

    // SavedState support is needed when hosting ComposeView outside an Activity.
    implementation("androidx.savedstate:savedstate-ktx:1.2.1")

    // HTTP + SSE to localhost:8080 (peko-agent)
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("com.squareup.okhttp3:okhttp-sse:4.12.0")

    // Kotlinx serialization for the event payloads
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
}
