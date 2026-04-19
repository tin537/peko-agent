// Root build file for the SMS-shim project. Per-module config is in app/build.gradle.kts.
// Deliberately minimal: this is a headless receiver APK, no Compose, no dependencies beyond
// the platform SDK, so the build stays small and fast.
//
// Versions pinned to AGP 8.11.1 + Kotlin 2.1.0 to match the Gradle 9.4.1
// wrapper. AGP <= 8.10 caps at Gradle 8.11.
plugins {
    id("com.android.application")      version "8.11.1" apply false
    id("org.jetbrains.kotlin.android") version "2.1.0"  apply false
}
