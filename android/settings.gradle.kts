// Novel Reader V1 — Android 正式工程（AN1 建立）
//
// 版本锁（本机已验证组合，禁止浮动）：
//   Gradle 8.11.1  +  AGP 8.10.1  +  Kotlin/Compose 插件 2.0.21
//   JDK 17（见 gradle.properties 的 org.gradle.java.home）
//   compileSdk/targetSdk 36、minSdk 31、NDK 27.3.13750724、首发 ABI arm64-v8a
//
// JNI 不使用 CMake / externalNativeBuild：Rust 静态库由 scripts/build-android-libs.sh
// 链接为自包含 libnr_jni.so，Gradle 只负责打包（详见 android/jni/README.md）。

import org.gradle.api.initialization.resolve.RepositoriesMode

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "novel-reader-android"
include(":app")
