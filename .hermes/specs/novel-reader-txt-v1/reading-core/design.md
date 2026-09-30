# reading-core 设计细节

## 目录职责

```text
crates/reading-source/   TextSource、编码、窗口和指纹
crates/reading-index/    章节、稀疏索引、布局边界和锚点
crates/reading-core/     阅读会话、位置、进度/书签语义、错误
crates/reading-storage/  SQLite 连接、迁移、事务仓储
crates/reading-ffi/      C ABI、JNI 导出和 Android 外部窗口协议
```

## Android 外部窗口协议

Android SAF 文件由 Kotlin 持有 `ContentResolver`/私有缓存读取能力。Rust 只持有内容长度和坐标空间，不持有 Activity 或 Kotlin 对象。一次请求包含 `book/session/chapter/offset/length/request_id`，返回字节窗口或稳定错误。若 Provider 不支持可靠 seek，Android 先可取消地复制到私有缓存，再让 Rust 使用文件型数据源。

原型 `ExternalTextSource` 返回空字节仅用于坐标语义测试；正式生产路径必须由 Android 适配层提供真实窗口，范围检查仍由 Rust 负责。

## SQLite 版本

初始迁移建立 `books`、`book_sources`、`chapters`、`reading_progress`、`bookmarks`、`reader_settings`、`layout_records`、`app_meta`。正文不入库。索引和分页缓存可删除并重建；进度和书签不能因索引清理丢失。

## 位置规则

`ContentPosition` 以 `book_id + content_unit_id + chapter_index + original_byte_offset + nearby_fingerprint` 表达。V1 一个 TXT 是一个 content unit。页码只属于某个 layout signature，不能单独作为进度身份。所有页面 end byte 必须前进且位于合法字符边界。
