plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// 首发 ABI（子规格 NFR-A1）。此常量同时用于 ndk.abiFilters 与 JNI 库构建任务，
// 避免两处各写一份导致 APK 里出现没有原生库的 ABI。
val nrAbi = "arm64-v8a"
// 仓库根目录（android/ 的上一级）。
val repoRoot: File = rootProject.projectDir.parentFile
// 由 scripts/build-android-libs.sh 生成的预编译 JNI 库（自包含，非手工提交）。
val nrJniLib: File = file("src/main/jniLibs/$nrAbi/libnr_jni.so")

android {
    namespace = "com.novelreader.android"
    compileSdk = 36
    // 固定为已安装的 build-tools，避免 AGP 尝试联网自动下载其它版本。
    buildToolsVersion = "36.0.0"
    // 必须指定与已装 NDK 一致的版本：AGP 才能找到 llvm-strip 对预编译 .so 做符号剥离
    // （省略会导致包体异常，见 docs/novel-reader-v1-development-plan.md）。
    ndkVersion = "27.3.13750724"

    // JNI **不**通过 CMake/externalNativeBuild 构建（本机 Android SDK 无 CMake）：
    // 自包含 libnr_jni.so 由 scripts/build-android-libs.sh 交叉编译 + 静态链接产出
    // （见 android/jni/README.md），Gradle 只负责打包。

    defaultConfig {
        applicationId = "com.novelreader.android"
        minSdk = 31
        targetSdk = 36
        versionCode = 2
        versionName = "0.5.0"
        // 构建标记：真机/APK 上一眼确认装的是哪一次构建（子规格 A-T7 / 检查项 C8）。
        // AN6 起由 `scripts/build-android-apk.sh` 产出 dist/ 分发包，标记值同步换代，
        // 使 `scripts/verify-android-apk.py` 能证明「本次构建的代码确实进了 APK」。
        buildConfigField("String", "BUILD_MARKER", "\"v1-v11-keep-awake-2026-10-09\"")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters += listOf(nrAbi)
        }
    }

    // 正式签名（L1）：仅当 android/local.properties 提供签名参数时才启用。
    // 注意：signingConfigs 必须定义在 `android { }` 这一层（AGP DSL 的 signingConfigs
    // 不属于 buildTypes 的作用域；写进 buildTypes.release 里会被忽略 → signingReport 显示
    // `Config: null`，Release 产物永远是 app-release-unsigned.apk）。
    // 密钥与口令**一律不入版本库**（local.properties 已被 .gitignore 忽略；
    // 配置模板见 docs/android/device-runbook.md「Release 正式签名」）。
    signingConfigs {
        create("releaseSigned") {
            // AGP **不会**把 local.properties 的任意键暴露为 project property，
            // 因此优先读环境变量（NR_SIGNING_STORE_FILE / _PASSWORD / _KEY_ALIAS / _KEY_PASSWORD），
            // 其次读 gradle.properties 的 nrSigning.* 键，最后读 local.properties 的
            // nrSigning.* 键（后者便于本机开发，需 build-android-apk.sh 显式转成环境变量）。
            fun nrSigningProp(name: String): String? =
                System.getenv("NR_SIGNING_" + name)
                    ?: (project.findProperty("nrSigning." + name) as String?)

            val nrStore = nrSigningProp("STORE_FILE")
            val nrStorePass = nrSigningProp("STORE_PASSWORD")
            val nrAlias = nrSigningProp("KEY_ALIAS")
            val nrKeyPass = nrSigningProp("KEY_PASSWORD")
            if (nrStore != null && nrStorePass != null && nrAlias != null && nrKeyPass != null) {
                storeFile = file(nrStore)
                storePassword = nrStorePass
                keyAlias = nrAlias
                keyPassword = nrKeyPass
                // v1+v2+v3：minSdk 31 实际上 v2/v3 足够，但同时启用 v1 以兼容旧校验工具。
                enableV1Signing = true
                enableV2Signing = true
                enableV3Signing = true
            }
        }
    }

buildTypes {
        debug {
            // Debug 与 Release 使用同一 applicationId，便于按同一包名校验两个产物。
            isMinifyEnabled = false
        }
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            // 仅在参数齐备**且** keystore 文件真实存在时才挂上签名配置；
            // 否则保持未签名（产物名 app-release-unsigned.apk，属预期，不得当发布包分发）。
            // 注意：不能写 `nrSigned.storeFile.isFile` —— storeFile 是可变属性，Kotlin 不允许智能转换。
            val nrSigned = signingConfigs.findByName("releaseSigned")
            val nrSignedFile = nrSigned?.storeFile
            if (nrSigned != null && nrSignedFile != null && nrSignedFile.isFile) {
                signingConfig = nrSigned
            }
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
        buildConfig = true
    }
    packaging {
        jniLibs {
            // .so 以未压缩方式存储：APK 内容校验可直接读大小，且安装时无需解压拷贝。
            useLegacyPackaging = false
        }
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
    lint {
        // 允许登记过的非阻断告警（检查项 C10：不得有 Kotlin 编译错误）。
        abortOnError = false
        checkReleaseBuilds = false
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

kotlin {
    jvmToolchain(17)
}

// ---------------------------------------------------------------------------
// JNI 预编译库构建：产物是构建生成物，必须可由脚本重建（子规格「禁止事项」）。
// 只有在 .so 缺失或 Rust/C 源变化时才真正执行脚本（Gradle up-to-date 检查），
// 因此 `./gradlew :app:assembleDebug` 从干净目录一次即可产出带 JNI 库的完整 APK。
// ---------------------------------------------------------------------------
val buildNrJniLibs = tasks.register<Exec>("buildNrJniLibs") {
    group = "nr"
    description = "交叉编译 Rust 静态库并链接自包含 libnr_jni.so（scripts/build-android-libs.sh）"
    workingDir = repoRoot
    commandLine("bash", "scripts/build-android-libs.sh", "--abi", nrAbi)

    inputs.files(
        fileTree(repoRoot.resolve("crates")) {
            include("**/*.rs", "**/Cargo.toml")
        }
    )
    inputs.files(
        repoRoot.resolve("Cargo.toml"),
        repoRoot.resolve("Cargo.lock"),
        repoRoot.resolve("rust-toolchain.toml"),
        repoRoot.resolve("include/novel_reader.h"),
        repoRoot.resolve("scripts/build-android-libs.sh"),
    )
    inputs.files(fileTree(repoRoot.resolve("android/jni")) { include("**/*.c", "**/*.h") })
    outputs.file(nrJniLib)
}

// preBuild 早于 mergeNativeLibs / packageDebug，保证打包时 .so 已就位。
tasks.named("preBuild") { dependsOn(buildNrJniLibs) }

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.compose.ui:ui:1.7.8")
    implementation("androidx.compose.ui:ui-tooling-preview:1.7.8")
    implementation("androidx.compose.material3:material3:1.3.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    // JVM 单元测试（A-T2/A-T3/A-T4/A-T5 的自动检查项都跑这一组任务）。
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")

    // 真机 UI 测试（延期门禁用，构建期不影响 JVM 单测）。
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4:1.7.8")
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.7.8")
}
