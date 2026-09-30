plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.novelreader.probe"
    compileSdk = 36
    // 固定为已安装的 build-tools，避免 AGP 尝试联网自动下载其它版本
    buildToolsVersion = "36.0.0"
    // 指定已安装的 NDK：AGP 才能找到 llvm-strip 对预编译 .so 做符号剥离（显著减小 APK）
    ndkVersion = "27.3.13750724"

    // 说明：JNI **不**通过 CMake/externalNativeBuild 构建（本机 Android SDK 未安装 CMake）。
    // 共享库由 scripts/build-android-libs.sh 交叉编译为自包含 .so，以「预编译库」形式放在
    // app/src/main/jniLibs/arm64-v8a/libnr_jni.so，Gradle 只负责打包。
    defaultConfig {
        applicationId = "com.novelreader.probe"
        minSdk = 31
        targetSdk = 36
        versionCode = 1
        versionName = "0.1"
        // 构建标记：真机上一眼确认装的是哪一次构建（探针用途）
        buildConfigField("String", "BUILD_MARKER", "\"v3-letterSpacing-fix\"")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // 当前版本仅支持 arm64
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
        debug {
            applicationIdSuffix = ".debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true; buildConfig = true }

    packaging {
        jniLibs { useLegacyPackaging = false }
        resources { excludes += "/META-INF/{AL2.0,LGPL2.1}" }
    }
    lint {
        abortOnError = false
        checkReleaseBuilds = false
    }
    testOptions { unitTests.isReturnDefaultValues = true }
}

kotlin { jvmToolchain(17) }

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.compose.ui:ui:1.7.8")
    implementation("androidx.compose.ui:ui-tooling-preview:1.7.8")
    implementation("androidx.compose.material3:material3:1.3.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    testImplementation("junit:junit:4.13.2")

    // 真机 UI 交互测试（验证「点击/手势有反应」这一回归点）
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4:1.7.8")
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.7.8")
}
