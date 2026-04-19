// Top-level build file. Per-module config lives in app/build.gradle.kts.
//
// AGP 8.11.x is the first release that officially supports Gradle 9.x;
// 8.7.3 (what we started with) is capped at Gradle 8.11. Kotlin 2.1.0
// matches AGP 8.11's expected Kotlin range and keeps the Compose
// compiler plugin on a version that's known to build cleanly against
// Compose BOM 2024.12.01 without forcing a dual bump.
plugins {
    id("com.android.application")                    version "8.11.1" apply false
    id("org.jetbrains.kotlin.android")               version "2.1.0"  apply false
    id("org.jetbrains.kotlin.plugin.compose")        version "2.1.0"  apply false
    id("org.jetbrains.kotlin.plugin.serialization")  version "2.1.0"  apply false
}
