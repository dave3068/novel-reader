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

    // ---- V1.1 自定义配色（设置页配色区）----

    /** 配色区启用开关。 */
    const val SETTINGS_THEME_ENABLED = "settings-theme-enabled"

    /** 配色区对比度读数。 */
    const val SETTINGS_THEME_CONTRAST = "settings-theme-contrast"

    /** 配色区「未设置」提示。 */
    const val SETTINGS_THEME_EMPTY_HINT = "settings-theme-empty-hint"

    /** 某个基色的输入框（`which` = 0..3）。 */
    fun settingsThemeBase(which: Int): String = "settings-theme-base-$which"

    // ---- V1.1 最近阅读（历史面板）----

    /** 历史面板根。 */
    const val HISTORY_SCREEN = "history-screen"

    /** 从书架进入历史的按钮。 */
    const val SHELF_OPEN_HISTORY_BUTTON = "shelf-open-history-button"

    /** 返回书架。 */
    const val HISTORY_BACK_BUTTON = "history-back-button"

    /** 空态。 */
    const val HISTORY_EMPTY_STATE = "history-empty-state"

    /** 总数文案。 */
    const val HISTORY_TOTAL = "history-total"

    /** 清空全部（带确认）。 */
    const val HISTORY_CLEAR_ALL_BUTTON = "history-clear-all-button"

    /** 某条流水的行。 */
    fun historyRow(entryId: Long): String = "history-row-$entryId"

    /** 某条流水的「清除该条」。 */
    fun historyClearEntry(entryId: Long): String = "history-clear-entry-$entryId"

    // ---- V1.1 阅读背景图（设置页背景区）----

    /** 背景文件名/大小信息行。 */
    const val SETTINGS_BACKGROUND_INFO = "settings-background-info"

    /** 未设置背景时的提示。 */
    const val SETTINGS_BACKGROUND_EMPTY_HINT = "settings-background-empty-hint"

    /** 遮罩强度滑杆。 */
    const val SETTINGS_BACKGROUND_SCRIM = "settings-background-scrim"

    /** 某个缩放口径的选项（0/1/2）。 */
    fun settingsBackgroundFit(mode: Int): String = "settings-background-fit-$mode"

    /** 选择图片…。 */
    const val SETTINGS_BACKGROUND_PICK = "settings-background-pick"

    /** 移除背景。 */
    const val SETTINGS_BACKGROUND_CLEAR = "settings-background-clear"

    // ---- V1.1 书架分类侧栏 ----

    /** 侧栏根。 */
    const val SIDEBAR = "shelf-category-sidebar"

    /** 「全部书籍」行。 */
    const val SIDEBAR_ALL = "shelf-category-all"

    /** 「未分类」行。 */
    const val SIDEBAR_UNCATEGORIZED = "shelf-category-uncategorized"

    /** 某个分类行。 */
    fun sidebarCategory(id: Long): String = "shelf-category-row-$id"

    /** 某个分类的展开/折叠箭头。 */
    fun sidebarToggle(id: Long): String = "shelf-category-toggle-$id"

    /** 新建分类按钮。 */
    const val SIDEBAR_CREATE = "shelf-category-create"

    /** 新建/重命名对话框的输入框。 */
    const val SIDEBAR_NAME_FIELD = "shelf-category-name-field"

    /** 对话框确认。 */
    const val SIDEBAR_NAME_CONFIRM = "shelf-category-name-confirm"

    /** 某个分类的「⋯」操作菜单入口。 */
    fun sidebarMenu(id: Long): String = "shelf-category-menu-$id"

    /** 操作菜单里的三项。 */
    const val SIDEBAR_MENU_RENAME = "shelf-category-menu-rename"
    const val SIDEBAR_MENU_MOVE = "shelf-category-menu-move"
    const val SIDEBAR_MENU_DELETE = "shelf-category-menu-delete"

    /** 目标选择对话框（移动 / 删除共用）。 */
    const val SIDEBAR_TARGET_DIALOG = "shelf-category-target-dialog"

    /** 某个目标行。 */
    fun sidebarTarget(id: Long): String = "shelf-category-target-$id"

    /** 删除去向选项（kind = 1/2/3）。 */
    fun sidebarDeleteDestination(kind: Int): String = "shelf-category-delete-dest-$kind"

    /** 确认按钮。 */
    const val SIDEBAR_CONFIRM = "shelf-category-confirm"
}
