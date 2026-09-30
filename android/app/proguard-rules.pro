# Release 混淆规则（当前不开启 minify，保留占位与 JNI 保护规则）
#
# JNI 声明必须保留：Rust 侧按 "Java_com_novelreader_android_core_NrCore_<入口>" 解析符号，
# 若类名/方法名被混淆，运行期会 UnsatisfiedLinkError。
-keepclasseswithmembernames,includedescriptorclasses class com.novelreader.android.core.** {
    native <methods>;
}
-keep class com.novelreader.android.core.NrCore { *; }
