// 顶层构建脚本。版本组合已按本机可用工具链锁定：
//   Gradle 8.11.1  +  AGP 8.10.1  +  Kotlin 2.0.21（含 Compose 编译器插件）
// 说明：AGP 8.10.x 最低要求 Gradle 8.11.1，并支持 compileSdk 36。
plugins {
    id("com.android.application") version "8.10.1" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
}
