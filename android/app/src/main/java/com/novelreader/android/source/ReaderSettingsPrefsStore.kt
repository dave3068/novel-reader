package com.novelreader.android.source

import com.novelreader.android.reader.ReaderSettingsSnapshot
import com.novelreader.android.reader.ReaderSettingsStore

/**
 * [ReaderSettingsStore] 的 Android 实现：复用 SAF 层同一份键值存储（`AndroidSafKeyValue` → `SharedPreferences`）。
 *
 * 存储格式：单键，值形如 `v1;fontSizeSp=18.0;lineSpacing=1.35;hMargin=20.0;vMargin=16.0;dark=0`。
 * 任一必需字段缺失或不可解析即视为无记录（返回 `null`，调用方保持默认值）——
 * 宁可回到默认设置，也不采用半截记录。
 */
class ReaderSettingsPrefsStore(private val kv: SafKeyValueStore) : ReaderSettingsStore {

    override fun load(): ReaderSettingsSnapshot? {
        val raw = kv.get(KEY) ?: return null
        val fields = LinkedHashMap<String, String>()
        for (part in raw.split(FIELD_SEPARATOR)) {
            val index = part.indexOf(PAIR_SEPARATOR)
            if (index > 0) fields[part.substring(0, index)] = part.substring(index + 1)
        }
        return ReaderSettingsSnapshot(
            fontSizeSp = fields[FONT_SIZE]?.toFloatOrNull() ?: return null,
            lineSpacingMultiplier = fields[LINE_SPACING]?.toFloatOrNull() ?: return null,
            horizontalMarginDp = fields[H_MARGIN]?.toFloatOrNull() ?: return null,
            verticalMarginDp = fields[V_MARGIN]?.toFloatOrNull() ?: return null,
            darkTheme = fields[DARK] == "1",
        )
    }

    override fun save(snapshot: ReaderSettingsSnapshot) {
        val dark = if (snapshot.darkTheme) "1" else "0"
        kv.put(
            KEY,
            "$VERSION$FIELD_SEPARATOR" +
                "$FONT_SIZE$PAIR_SEPARATOR${snapshot.fontSizeSp}$FIELD_SEPARATOR" +
                "$LINE_SPACING$PAIR_SEPARATOR${snapshot.lineSpacingMultiplier}$FIELD_SEPARATOR" +
                "$H_MARGIN$PAIR_SEPARATOR${snapshot.horizontalMarginDp}$FIELD_SEPARATOR" +
                "$V_MARGIN$PAIR_SEPARATOR${snapshot.verticalMarginDp}$FIELD_SEPARATOR" +
                "$DARK$PAIR_SEPARATOR$dark",
        )
    }

    private companion object {
        const val KEY = "reader.settings.v1"
        const val VERSION = "v1"
        const val FONT_SIZE = "fontSizeSp"
        const val LINE_SPACING = "lineSpacing"
        const val H_MARGIN = "hMargin"
        const val V_MARGIN = "vMargin"
        const val DARK = "dark"
        const val FIELD_SEPARATOR = ';'
        const val PAIR_SEPARATOR = '='
    }
}
