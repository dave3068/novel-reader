package com.novelreader.android.core

/**
 * 头文件冻结的 12 项错误码（`NR_ERR_*`，数值即 0..=11）。
 *
 * 名字与 `include/novel_reader.h` 逐字相同，数值由
 * `scripts/verify-android-boundary.py` 与头文件宏逐项比对。
 */
object NrErrors {
    const val NR_ERR_OK = 0
    const val NR_ERR_INVALID_ARGUMENT = 1
    const val NR_ERR_OUT_OF_RANGE = 2
    const val NR_ERR_UTF8_BOUNDARY = 3
    const val NR_ERR_BOOK_NOT_FOUND = 4
    const val NR_ERR_NO_OPEN_BOOK = 5
    const val NR_ERR_IO = 6
    const val NR_ERR_CANCELLED = 7
    const val NR_ERR_LAYOUT_NOT_FOUND = 8
    const val NR_ERR_ENGINE_DESTROYED = 9
    const val NR_ERR_INTERNAL = 10
    const val NR_ERR_CHAPTER_OUT_OF_RANGE = 11

    /** 冻结的 12 项错误码（顺序即数值 0..=11）。 */
    val ALL: List<Int> = listOf(
        NR_ERR_OK,
        NR_ERR_INVALID_ARGUMENT,
        NR_ERR_OUT_OF_RANGE,
        NR_ERR_UTF8_BOUNDARY,
        NR_ERR_BOOK_NOT_FOUND,
        NR_ERR_NO_OPEN_BOOK,
        NR_ERR_IO,
        NR_ERR_CANCELLED,
        NR_ERR_LAYOUT_NOT_FOUND,
        NR_ERR_ENGINE_DESTROYED,
        NR_ERR_INTERNAL,
        NR_ERR_CHAPTER_OUT_OF_RANGE,
    )

    /** 头文件冻结的错误码上界（`NR_ERR_CHAPTER_OUT_OF_RANGE`）。 */
    const val MAX_FROZEN_CODE = NR_ERR_CHAPTER_OUT_OF_RANGE

    /** 是否是头文件冻结过的错误码。 */
    fun isFrozen(rawCode: Int): Boolean = rawCode in NR_ERR_OK..MAX_FROZEN_CODE
}

/**
 * 错误码的**业务分类**：只依据错误码数值做映射，**绝不解析错误文本**
 * （头文件 §2「错误文本只用于展示，禁止用文本判断业务分支」）。
 *
 * 分类用于决定 UI 行为（提示 / 重试 / 重新导入 / 重新登记排版世代），
 * 不复制核心语义。
 */
enum class NrErrorKind {
    /** 成功（`NR_ERR_OK`）。 */
    NONE,

    /** 参数与契约错误（调用方 bug，不重试）。 */
    CALLER,

    /** 偏移 / 索引越界：可用核心回传的坐标纠正后重试。 */
    RANGE,

    /** 偏移不在合法 UTF-8 边界：按核心修正后的偏移重试。 */
    ENCODING,

    /** 书籍不存在：需重新导入或重新关联。 */
    NOT_FOUND,

    /** 尚未打开书籍：先 `nr_open_txt`。 */
    SESSION,

    /** 数据源 I/O 失败（平台层问题，可重试 / 提示）。 */
    IO,

    /** 协作式取消：长任务的部分结果（`partial = 1`）。 */
    CANCELLED,

    /** 排版世代或请求标识过期：重新 BEGIN_LAYOUT。 */
    LAYOUT,

    /** 引擎已销毁：句柄不可再用。 */
    ENGINE_STATE,

    /** 内部错误（含 panic 拦截）；`null` 返回但无记录错误码也归此类。 */
    INTERNAL,

    /** 章节越界 / 偏移与章节不一致。 */
    CHAPTER,

    /** 头文件未冻结的错误码：**不得当作成功**。 */
    UNKNOWN,
    ;

    companion object {
        /** 错误码 → 业务分类；未知码返回 [UNKNOWN]（绝不静默当作 [NONE]）。 */
        fun of(rawCode: Int): NrErrorKind = when (rawCode) {
            NrErrors.NR_ERR_OK -> NONE
            NrErrors.NR_ERR_INVALID_ARGUMENT -> CALLER
            NrErrors.NR_ERR_OUT_OF_RANGE -> RANGE
            NrErrors.NR_ERR_UTF8_BOUNDARY -> ENCODING
            NrErrors.NR_ERR_BOOK_NOT_FOUND -> NOT_FOUND
            NrErrors.NR_ERR_NO_OPEN_BOOK -> SESSION
            NrErrors.NR_ERR_IO -> IO
            NrErrors.NR_ERR_CANCELLED -> CANCELLED
            NrErrors.NR_ERR_LAYOUT_NOT_FOUND -> LAYOUT
            NrErrors.NR_ERR_ENGINE_DESTROYED -> ENGINE_STATE
            NrErrors.NR_ERR_INTERNAL -> INTERNAL
            NrErrors.NR_ERR_CHAPTER_OUT_OF_RANGE -> CHAPTER
            else -> UNKNOWN
        }
    }
}

/**
 * 跨边界失败的统一异常：只携带 `NR_ERR_*` 数值与由它推导的分类。
 *
 * `message` 仅用于展示（可为 `null`）；**不得**用 `message` 判断业务分支。
 */
class NrException(
    val rawCode: Int,
    val kind: NrErrorKind = NrErrorKind.of(rawCode),
    message: String? = null,
) : RuntimeException(message ?: "NR_ERR_$rawCode (${kind.name})") {

    /** 是否为协作式取消（长任务部分成功）。 */
    val isCancelled: Boolean
        get() = kind == NrErrorKind.CANCELLED
}

/** 非 0 错误码即抛出 [NrException]（错误码 → 分类不做文本解析）。 */
internal fun Int.requireOk(): Int {
    if (this != NrErrors.NR_ERR_OK) {
        throw NrException(this)
    }
    return this
}
