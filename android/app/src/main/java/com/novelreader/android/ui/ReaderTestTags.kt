package com.novelreader.android.ui

/**
 * 三个页面的测试标签（`Modifier.testTag`）。
 *
 * 真机 UI 测试（延期的 instrumentation 门禁）与 AN6 的 APK 校验脚本按这些标签定位控件：
 * 同名文案会在语义树上产生多节点歧义，标签是稳定锚点（见 novel-reader-ui-env 技能）。
 */
object ReaderTestTags {

    // 书架页
    const val SHELF_SCREEN = "nr-shelf-screen"
    const val SHELF_EMPTY_STATE = "nr-shelf-empty"
    const val SHELF_IMPORT_BUTTON = "nr-shelf-import"
    const val SHELF_REMOVE_BUTTON = "nr-shelf-remove"
    const val SHELF_REMOVE_CONFIRM = "nr-shelf-remove-confirm"
    const val SHELF_REMOVE_CANCEL = "nr-shelf-remove-cancel"
    const val SHELF_REMOVE_DIALOG = "nr-shelf-remove-dialog"
    const val SHELF_IMPORT_BUTTON_BOTTOM = "nr-shelf-import-bottom"
    const val SHELF_BOOK_PREFIX = "nr-shelf-book-"
    const val SHELF_OPEN_READER_BUTTON = "nr-shelf-open-reader"
    const val SHELF_OPEN_SETTINGS_BUTTON = "nr-shelf-open-settings"

    // 阅读页
    const val READER_SCREEN = "nr-reader-screen"
    const val READER_BODY = "nr-reader-body"
    const val READER_STATUS = "nr-reader-status"
    const val READER_PREVIOUS_PAGE = "nr-reader-previous-page"
    const val READER_NEXT_PAGE = "nr-reader-next-page"
    const val READER_PREVIOUS_CHAPTER = "nr-reader-previous-chapter"
    const val READER_NEXT_CHAPTER = "nr-reader-next-chapter"
    const val READER_SAVE_POSITION = "nr-reader-save-position"
    const val READER_OPEN_SETTINGS = "nr-reader-open-settings"
    const val READER_BACK = "nr-reader-back"
    const val READER_CHAPTER_LIST_BUTTON = "nr-reader-chapters"
    const val READER_ENCODING_BUTTON = "nr-reader-encoding"
    const val READER_ENCODING_DIALOG = "nr-reader-encoding-dialog"
    const val READER_ENCODING_AUTO = "nr-reader-encoding-auto"
    const val READER_CHAPTER_LIST = "nr-reader-chapter-list"
    const val READER_CHAPTER_LIST_CLOSE = "nr-reader-chapter-list-close"

    // 设置页
    const val SETTINGS_SCREEN = "nr-settings-screen"
    const val SETTINGS_FONT_SIZE_SLIDER = "nr-settings-font-size"
    const val SETTINGS_LINE_SPACING_SLIDER = "nr-settings-line-spacing"
    const val SETTINGS_HORIZONTAL_MARGIN_SLIDER = "nr-settings-horizontal-margin"
    const val SETTINGS_VERTICAL_MARGIN_SLIDER = "nr-settings-vertical-margin"
    const val SETTINGS_DARK_THEME_SWITCH = "nr-settings-dark-theme"
    const val SETTINGS_RELAYOUT_BUTTON = "nr-settings-relayout"
    const val SETTINGS_SAVE_POSITION_BUTTON = "nr-settings-save-position"
    const val SETTINGS_REBUILD_INDEX_BUTTON = "nr-settings-rebuild-index"
    const val SETTINGS_CANCEL_REQUEST_BUTTON = "nr-settings-cancel-request"
    const val SETTINGS_DIAGNOSTICS = "nr-settings-diagnostics"
    const val SETTINGS_BACK = "nr-settings-back"

    /** 编码选项标签（编码码唯一；0 = 自动）。 */
    fun readerEncodingOption(code: Int): String = "$READER_ENCODING_BUTTON-option-$code"

    /** 目录条目标签（章序号唯一）。 */
    fun readerChapterItem(chapterIndex: Int): String = "$READER_CHAPTER_LIST_BUTTON-item-$chapterIndex"

    /** 书架条目「移除」按钮标签（来源标识唯一）。 */
    fun shelfRemoveButton(sourceIdentity: String): String = "$SHELF_REMOVE_BUTTON-$sourceIdentity"

    /** 书架条目标签（`bookId` 唯一）。 */
    fun shelfBook(bookId: Long): String = "$SHELF_BOOK_PREFIX$bookId"
}
