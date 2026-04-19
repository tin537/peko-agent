// Root build file for the SMS-shim project. Per-module config is in app/build.gradle.kts.
// Deliberately minimal: this is a headless receiver APK, no Compose, no dependencies beyond
// the platform SDK, so the build stays small and fast.
plugins {
    id("com.android.application")      version "8.7.3"  apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
