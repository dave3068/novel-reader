//! 编码识别（BOM / UTF-8 校验 / GB18030·GBK·Big5 统计评分）、可信度与手动覆盖。
//!
//! 契约：`docs/novel-reader-v1-development-plan.md` §7.1、reading-core CR2/AC2、
//! 父级规格 §编码；本模块是 `reading-source` 的一部分，不依赖 `reading-core`。
//!
//! # 识别流程（严格按开发方案顺序）
//!
//! 1. 读**有限**头部窗口；文件较大时补中部/尾部采样窗口（[`DETECT_MAX_SAMPLED_BYTES`] 为总量硬上限）；
//! 2. BOM 优先：UTF-8 BOM（`EF BB BF`）、UTF-16 LE（`FF FE`）、UTF-16 BE（`FE FF`）；
//! 3. 无 BOM：先严格验证 UTF-8，并用“可读性”合理性检查判断是否可信；
//! 4. 仅当 UTF-8 不可信时，才用 GBK/Big5 常用字码表对 GB18030/GBK/Big5 做统计评分；
//! 5. 输出 `encoding + confidence + detector_version`（[`EncodingDetection`]）；
//!    低可信度时 [`EncodingDetection::needs_confirmation`] 为真，等待用户手动覆盖；
//! 6. 覆盖只改变“解释方式”（[`EncodingState::override_encoding`]），**永不写回原文件**：
//!    本模块不含任何写文件路径。
//!
//! # 边界与诚实性规则
//!
//! - 检测只读 [`DETECT_MAX_SAMPLED_BYTES`]（64 KiB）以内的字节，**与文件大小无关**，
//!   不做整本统计；
//! - GBK 与 GB18030 在两字节区间字节完全相同：两字节内容判 `GBK` 并把 `GB18030`
//!   放进 [`EncodingDetection::alternatives`]（两者解码等价，不隐瞒）；
//! - 出现合法 GB18030 **四字节**序列时只有 GB18030 能解释这些字节（`GBK` 结构上不成立），
//!   因此判 `GB18030`；
//! - 指标并列（[`LEGACY_MARGIN_HIGH`] 之内）一律返回低可信度，**不**伪装成高可信度；
//! - 无任何候选成立时回落到 `UTF-8` + 低可信度，并列出得分最高的候选供用户选择；
//! - BOM 与内容矛盾（例如 LE BOM + BE 载荷）时保留 BOM 声明的编码但降为低可信度，
//!   并把互补字节序放进 `alternatives`。

use crate::source::{align_to_utf8_boundaries, SourceError, TextSource};

/// 检测器版本（开发方案 §7.1 的 `encoding_detector_version`；规则变化时必须递增）。
pub const ENCODING_DETECTOR_VERSION: u32 = 1;

/// 头部检测窗口：32 KiB。
pub const DETECT_HEAD_BYTES: usize = 32 * 1024;

/// 中部/尾部采样窗口：16 KiB。
pub const DETECT_SAMPLE_BYTES: usize = 16 * 1024;

/// 单次检测物化字节的硬上限：头部 + 2 个采样（64 KiB），与文件大小无关。
pub const DETECT_MAX_SAMPLED_BYTES: usize = DETECT_HEAD_BYTES + 2 * DETECT_SAMPLE_BYTES;

/// UTF-8 可信度阈值：可疑字符比例（千分比）超过该值即认为 UTF-8 不可信。
pub const UTF8_SUSPICIOUS_MAX_PERMILLE: u32 = 5;

/// UTF-16 合理性阈值（千分比）。
pub const UTF16_SUSPICIOUS_MAX_PERMILLE: u32 = 5;

/// UTF-8 判高可信所需的最小样本字节数。
pub const UTF8_HIGH_MIN_SAMPLED_BYTES: u64 = 256;

/// 样本小于该字节数时 UTF-8 结果一律判低可信度（样本过短）。
pub const UTF8_LOW_MAX_SAMPLED_BYTES: u64 = 64;

/// 传统编码候选结构合法率下限（千分比）：低于该值视为“候选不成立”。
pub const LEGACY_STRUCTURAL_MIN_PERMILLE: u32 = 900;

/// 结构非法字节的允许预算下限（字节）。
///
/// 采样窗口会被 [`crate::TextSource::read_at`] 按 UTF-8 边界裁剪，传统编码在窗口两端
/// 可能出现极少量“落单字节”伪影；该预算只覆盖这类伪影，不掩盖真实非法内容
/// （乱码/二进制内容的非法字节数远超预算）。
pub const LEGACY_INVALID_BUDGET_MIN: u32 = 2;

/// 结构非法字节预算的比例分母：每 `N` 字节允许 1 个非法字节（0.2%）。
pub const LEGACY_INVALID_BUDGET_DIVISOR: u32 = 500;

/// 给定样本字节数下允许的“结构非法字节”预算。
pub const fn structural_invalid_budget(total_bytes: u32) -> u32 {
    let scaled = total_bytes / LEGACY_INVALID_BUDGET_DIVISOR;
    if scaled > LEGACY_INVALID_BUDGET_MIN {
        scaled
    } else {
        LEGACY_INVALID_BUDGET_MIN
    }
}

/// 传统编码候选的常用字命中率下限（千分比）。
///
/// 结构合法但几乎没有任何常用字命中的字节流（例如乱码、二进制或私用区内容）不构成
/// “识别成功”，必须回落为低可信度让用户确认，而不是给出一个看起来正常的结果。
pub const LEGACY_HIT_MIN_PERMILLE: u32 = 50;

/// 判高可信所需的领先优势（千分比分数差）。
pub const LEGACY_MARGIN_HIGH: u32 = 120;

/// 判中可信所需的领先优势。
pub const LEGACY_MARGIN_MEDIUM: u32 = 40;

/// 判高可信所需的最低分数。
pub const LEGACY_SCORE_HIGH_MIN: u32 = 700;

/// 结构合法率在总分中的权重（千分比）。
pub const LEGACY_STRUCTURAL_WEIGHT: u32 = 600;

/// 常用字命中率在总分中的权重（千分比）。
pub const LEGACY_HIT_WEIGHT: u32 = 400;

// @@begin generated tables@@
/// GBK/GB18030 常用字两字节码表（大端 u16，升序；由字频表前 N 个可编码字符推导）。
pub const GBK_COMMON_CODES: [u16; 512] = [
    0xB0A2, 0xB0AE, 0xB0B2, 0xB0CB, 0xB0D1, 0xB0D7, 0xB0D9, 0xB0EB, 0xB0EC, 0xB1A3, 0xB1A8, 0xB1B1,
    0xB1B8, 0xB1BB, 0xB1BE, 0xB1C8, 0xB1D8, 0xB1DF, 0xB1E3, 0xB1E4, 0xB1ED, 0xB1F0, 0xB1F8, 0xB2A1,
    0xB2A2, 0xB2BB, 0xB2BC, 0xB2BD, 0xB2BF, 0xB2C5, 0xB2CE, 0xB2FA, 0xB3A1, 0xB3A3, 0xB3A4, 0xB3AF,
    0xB3B5, 0xB3BC, 0xB3C2, 0xB3C6, 0xB3C7, 0xB3C9, 0xB3CC, 0xB3D4, 0xB3D6, 0xB3F5, 0xB3F6, 0xB3FD,
    0xB4A6, 0xB4AB, 0xB4CB, 0xB4CE, 0xB4D3, 0xB4E6, 0xB4EF, 0xB4F2, 0xB4F3, 0xB4F8, 0xB4FA, 0xB5A5,
    0xB5AB, 0xB5B1, 0xB5B3, 0xB5BC, 0xB5BD, 0xB5C0, 0xB5C2, 0xB5C3, 0xB5C4, 0xB5C8, 0xB5D8, 0xB5DA,
    0xB5DB, 0xB5E3, 0xB5E7, 0xB5F7, 0xB6A8, 0xB6AB, 0xB6AF, 0xB6BC, 0xB6C8, 0xB6CF, 0xB6D3, 0xB6D4,
    0xB6E0, 0xB6F8, 0xB6F9, 0xB6FB, 0xB6FE, 0xB7A2, 0xB7A8, 0xB7B4, 0xB7BD, 0xB7BF, 0xB7C5, 0xB7C7,
    0xB7C9, 0xB7D6, 0xB7E7, 0xB7F2, 0xB7FE, 0xB8AE, 0xB8B4, 0xB8B8, 0xB8C3, 0xB8C4, 0xB8C9, 0xB8D0,
    0xB8DF, 0xB8E6, 0xB8F1, 0xB8F6, 0xB8F7, 0xB8F8, 0xB8F9, 0xB8FC, 0xB9A4, 0xB9A6, 0xB9AB, 0xB9B2,
    0xB9C5, 0xB9CA, 0xB9D8, 0xB9D9, 0xB9DB, 0xB9DC, 0xB9E2, 0xB9E3, 0xB9E6, 0xB9FA, 0xB9FB, 0xB9FD,
    0xBAA3, 0xBAC3, 0xBAC5, 0xBACD, 0xBACE, 0xBACF, 0xBAD3, 0xBADC, 0xBAEC, 0xBAF2, 0xBAF3, 0xBAF5,
    0xBBA8, 0xBBAA, 0xBBAF, 0xBBB0, 0xBBB9, 0xBBC6, 0xBBCA, 0xBBD8, 0xBBE1, 0xBBEE, 0xBBF0, 0xBBF2,
    0xBBF7, 0xBBF9, 0xBBFA, 0xBCAB, 0xBCAF, 0xBCB0, 0xBCB4, 0xBCB6, 0xBCB8, 0xBCBA, 0xBCC3, 0xBCC6,
    0xBCC7, 0xBCD2, 0xBCD3, 0xBCDB, 0xBCE4, 0xBCFB, 0xBCFE, 0xBDA8, 0xBDAB, 0xBDAD, 0xBDBB, 0xBDCC,
    0xBDD0, 0xBDD3, 0xBDDA, 0xBDE1, 0xBDE2, 0xBDE7, 0xBDF0, 0xBDF1, 0xBDF8, 0xBDFC, 0xBEA1, 0xBEA9,
    0xBEAB, 0xBEAD, 0xBEC5, 0xBECD, 0xBED9, 0xBEDD, 0xBEDF, 0xBEF5, 0xBEF6, 0xBEFC, 0xBFAA, 0xBFB4,
    0xBFC6, 0xBFC9, 0xBFCB, 0xBFD5, 0xBFDA, 0xBFEC, 0xBFF6, 0xC0AD, 0xC0B4, 0xC0CF, 0xC0D6, 0xC0E0,
    0xC0EB, 0xC0ED, 0xC0EE, 0xC0EF, 0xC0FA, 0xC0FB, 0xC1A2, 0xC1A6, 0xC1AA, 0xC1AC, 0xC1BD, 0xC1BF,
    0xC1CB, 0xC1D6, 0xC1EC, 0xC1EE, 0xC1F4, 0xC1F7, 0xC1F9, 0xC2B7, 0xC2DB, 0xC2DE, 0xC2ED, 0xC2FA,
    0xC3B4, 0xC3BB, 0xC3BF, 0xC3C0, 0xC3C5, 0xC3C7, 0xC3E6, 0xC3F1, 0xC3F7, 0xC3FB, 0xC3FC, 0xC4B8,
    0xC4BF, 0xC4C7, 0xC4CB, 0xC4CF, 0xC4D1, 0xC4D8, 0xC4DA, 0xC4DC, 0xC4E3, 0xC4EA, 0xC5AE, 0xC6B7,
    0xC6BD, 0xC6DA, 0xC6DF, 0xC6E4, 0xC6F0, 0xC6F7, 0xC6F8, 0xC7A7, 0xC7AE, 0xC7B0, 0xC7BF, 0xC7D0,
    0xC7D2, 0xC7D7, 0xC7E0, 0xC7E1, 0xC7E5, 0xC7E9, 0xC7EB, 0xC7F3, 0xC7F8, 0xC8A1, 0xC8A5, 0xC8A8,
    0xC8AB, 0xC8B4, 0xC8B7, 0xC8BB, 0xC8C3, 0xC8CB, 0xC8CE, 0xC8CF, 0xC8D5, 0xC8DD, 0xC8E7, 0xC8EB,
    0xC8F4, 0xC8FD, 0xC9AB, 0xC9B1, 0xC9BD, 0xC9CC, 0xC9CF, 0xC9D0, 0xC9D9, 0xC9E7, 0xC9E8, 0xC9ED,
    0xC9EE, 0xC9F1, 0xC9F5, 0xC9F9, 0xC9FA, 0xCAA6, 0xCAA7, 0xCAAE, 0xCAAF, 0xCAB1, 0xCAB2, 0xCAB5,
    0xCAB6, 0xCAB7, 0xCAB9, 0xCABC, 0xCABD, 0xCABE, 0xCABF, 0xCAC0, 0xCAC2, 0xCAC7, 0xCAD0, 0xCAD3,
    0xCAD5, 0xCAD6, 0xCAD7, 0xCAD8, 0xCADC, 0xCAE9, 0xCAF5, 0xCAFD, 0xCBAE, 0xCBB5, 0xCBB9, 0xCBBC,
    0xCBBE, 0xCBC0, 0xCBC4, 0xCBC6, 0xCBE3, 0xCBE4, 0xCBE6, 0xCBF9, 0xCBFB, 0xCBFC, 0xCBFD, 0xCCA8,
    0xCCAB, 0xCCD8, 0xCCE1, 0xCCE2, 0xCCE5, 0xCCEC, 0xCCF5, 0xCCFD, 0xCDA8, 0xCDAC, 0xCDB3, 0xCDB7,
    0xCDC5, 0xCDE2, 0xCDEA, 0xCDF2, 0xCDF5, 0xCDF9, 0xCDFB, 0xCEAA, 0xCEB4, 0xCEBB, 0xCEC4, 0xCEC5,
    0xCECA, 0xCED2, 0xCEDE, 0xCEE4, 0xCEE5, 0xCEEF, 0xCEF1, 0xCEF7, 0xCFA2, 0xCFB5, 0xCFC2, 0xCFC8,
    0xCFD6, 0xCFD8, 0xCFE0, 0xCFEB, 0xCFF1, 0xCFF2, 0xCFF3, 0xD0A1, 0xD0A6, 0xD0A9, 0xD0B4, 0xD0C2,
    0xD0C4, 0xD0C5, 0xD0CB, 0xD0CE, 0xD0D0, 0xD0D4, 0xD0EB, 0xD0ED, 0xD1A7, 0xD1D4, 0xD1DB, 0xD1F4,
    0xD1F9, 0xD2AA, 0xD2B2, 0xD2B5, 0xD2BB, 0xD2D1, 0xD2D4, 0xD2D7, 0xD2E0, 0xD2E2, 0xD2E5, 0xD2E9,
    0xD2F2, 0xD2FD, 0xD3A2, 0xD3A6, 0xD3B0, 0xD3C3, 0xD3C9, 0xD3D0, 0xD3D6, 0xD3DA, 0xD3E0, 0xD3EB,
    0xD3EF, 0xD4AA, 0xD4AD, 0xD4B1, 0xD4B6, 0xD4BA, 0xD4BB, 0xD4BC, 0xD4BD, 0xD4C2, 0xD4C6, 0xD4CB,
    0xD4D9, 0xD4DA, 0xD4E7, 0xD4EC, 0xD4F2, 0xD4F5, 0xD4F8, 0xD5B9, 0xD5BD, 0xD5C2, 0xD5C5, 0xD5D5,
    0xD5DF, 0xD5E2, 0xD5E6, 0xD5F9, 0xD5FB, 0xD5FD, 0xD5FE, 0xD6A4, 0xD6A7, 0xD6AA, 0xD6AE, 0xD6B1,
    0xD6B8, 0xD6BB, 0xD6C1, 0xD6C6, 0xD6CA, 0xD6CE, 0xD6D0, 0xD6D6, 0xD6D8, 0xD6DA, 0xD6DC, 0xD6DD,
    0xD6EE, 0xD6F7, 0xD7A1, 0xD7A2, 0xD7AA, 0xD7BC, 0xD7C5, 0xD7CA, 0xD7D3, 0xD7D4, 0xD7D6, 0xD7DA,
    0xD7DC, 0xD7DF, 0xD7E3, 0xD7E9, 0xD7EE, 0xD7F6, 0xD7F7, 0xD7F8,
];

/// Big5 常用字两字节码表（大端 u16，升序；由繁体字频表前 N 个可编码字符推导）。
pub const BIG5_COMMON_CODES: [u16; 512] = [
    0xA440, 0xA446, 0xA447, 0xA448, 0xA44A, 0xA44B, 0xA44F, 0xA451, 0xA453, 0xA454, 0xA455, 0xA457,
    0xA45B, 0xA45D, 0xA466, 0xA468, 0xA46A, 0xA46B, 0xA46C, 0xA470, 0xA473, 0xA475, 0xA476, 0xA477,
    0xA47E, 0xA4A3, 0xA4A4, 0xA4A7, 0xA4AD, 0xA4B0, 0xA4B5, 0xA4B8, 0xA4BA, 0xA4BB, 0xA4BD, 0xA4C0,
    0xA4C6, 0xA4CD, 0xA4CE, 0xA4CF, 0xA4D1, 0xA4D3, 0xA4D6, 0xA4DE, 0xA4DF, 0xA4E2, 0xA4E4, 0xA4E5,
    0xA4E8, 0xA4E9, 0xA4EB, 0xA4F1, 0xA4F4, 0xA4F9, 0xA4FD, 0xA540, 0xA542, 0xA544, 0xA547, 0xA548,
    0xA54C, 0xA54E, 0xA54F, 0xA553, 0xA558, 0xA55B, 0xA55C, 0xA55F, 0xA562, 0xA564, 0xA568, 0xA569,
    0xA573, 0xA574, 0xA575, 0xA578, 0xA57C, 0xA57E, 0xA5A1, 0xA5A2, 0xA5A6, 0xA5AB, 0xA5AD, 0xA5B2,
    0xA5B4, 0xA5BB, 0xA5BC, 0xA5BF, 0xA5C1, 0xA5CD, 0xA5CE, 0xA5D1, 0xA5D5, 0xA5D8, 0xA5DF, 0xA5E6,
    0xA5F3, 0xA5F4, 0xA5F7, 0xA5FA, 0xA5FD, 0xA5FE, 0xA640, 0xA641, 0xA650, 0xA655, 0xA656, 0xA657,
    0xA658, 0xA659, 0xA65D, 0xA65E, 0xA661, 0xA662, 0xA668, 0xA66E, 0xA66F, 0xA670, 0xA672, 0xA677,
    0xA67E, 0xA6A1, 0xA6A8, 0xA6AC, 0xA6B3, 0xA6B8, 0xA6B9, 0xA6BA, 0xA6CB, 0xA6D1, 0xA6D2, 0xA6D3,
    0xA6DB, 0xA6DC, 0xA6E2, 0xA6E6, 0xA6E8, 0xA6EC, 0xA6ED, 0xA6F3, 0xA6FC, 0xA6FD, 0xA740, 0xA741,
    0xA74F, 0xA751, 0xA759, 0xA75F, 0xA761, 0xA769, 0xA7B9, 0xA7C6, 0xA7CC, 0xA7CE, 0xA7D6, 0xA7DA,
    0xA7E2, 0xA7E4, 0xA7EB, 0xA7EF, 0xA7F3, 0xA7F5, 0xA843, 0xA844, 0xA84D, 0xA853, 0xA86B, 0xA873,
    0xA874, 0xA8A3, 0xA8A5, 0xA8AB, 0xA8AD, 0xA8AE, 0xA8BA, 0xA8C3, 0xA8C6, 0xA8C7, 0xA8CF, 0xA8D3,
    0xA8E0, 0xA8E2, 0xA8E4, 0xA8EC, 0xA8EE, 0xA8FA, 0xA8FC, 0xA8FE, 0xA94D, 0xA94F, 0xA952, 0xA95F,
    0xA96C, 0xA970, 0xA977, 0xA9B3, 0xA9C0, 0xA9C7, 0xA9CA, 0xA9CE, 0xA9D2, 0xA9F1, 0xA9F3, 0xA9FA,
    0xAA41, 0xAA42, 0xAA46, 0xAA47, 0xAA4C, 0xAA4F, 0xAA6B, 0xAAA9, 0xAAAB, 0xAAB1, 0xAABA, 0xAABD,
    0xAABE, 0xAAC0, 0xAAC5, 0xAACC, 0xAAD9, 0xAAE1, 0xAAED, 0xAAF1, 0xAAF7, 0xAAF8, 0xAAF9, 0xAAFC,
    0xAB44, 0xAB48, 0xAB4B, 0xAB4F, 0xAB65, 0xAB68, 0xAB6E, 0xAB6F, 0xAB7E, 0xABA2, 0xABAC, 0xABB0,
    0xABC4, 0xABC7, 0xABD7, 0xABD8, 0xABDC, 0xABE1, 0xABE4, 0xABE7, 0xABF9, 0xABFC, 0xAC46, 0xAC47,
    0xAC4F, 0xAC50, 0xAC79, 0xACA1, 0xACB0, 0xACC9, 0xACDB, 0xACDD, 0xACE3, 0xACEC, 0xACF9, 0xACFC,
    0xAD43, 0xAD59, 0xAD5E, 0xAD6E, 0xAD70, 0xAD78, 0xADAB, 0xADB1, 0xADB5, 0xADB7, 0xADB8, 0xADCC,
    0xADD3, 0xADD4, 0xADD7, 0xADEC, 0xADFB, 0xADFE, 0xAE61, 0xAE65, 0xAE74, 0xAE76, 0xAEB3, 0xAEC6,
    0xAEC9, 0xAED1, 0xAED5, 0xAED7, 0xAEE6, 0xAEF0, 0xAEFC, 0xAF53, 0xAF5A, 0xAF64, 0xAF75, 0xAF7D,
    0xAFAB, 0xAFB8, 0xAFBA, 0xAFC5, 0xAFE0, 0xAFEB, 0xB04F, 0xB051, 0xB054, 0xB05F, 0xB067, 0xB0A3,
    0xB0A8, 0xB0AA, 0xB0B5, 0xB0C4, 0xB0C8, 0xB0CA, 0xB0CF, 0xB0D1, 0xB0D5, 0xB0DA, 0xB0DD, 0xB0EA,
    0xB0F2, 0xB14D, 0xB14E, 0xB160, 0xB161, 0xB169, 0xB16A, 0xB16F, 0xB171, 0xB17A, 0xB1A1, 0xB1B5,
    0xB1BC, 0xB1D0, 0xB1E6, 0xB24D, 0xB260, 0xB279, 0xB27A, 0xB27B, 0xB2B4, 0xB2BC, 0xB2C4, 0xB2CE,
    0xB2D5, 0xB342, 0xB351, 0xB35C, 0xB35D, 0xB36F, 0xB371, 0xB373, 0xB374, 0xB379, 0xB3A1, 0xB3A3,
    0xB3AF, 0xB3B9, 0xB3C6, 0xB3CC, 0xB3DF, 0xB3E1, 0xB3E6, 0xB3F5, 0xB3F8, 0xB44E, 0xB458, 0xB4A3,
    0xB4AB, 0xB4BF, 0xB4C1, 0xB54C, 0xB54D, 0xB565, 0xB56F, 0xB57B, 0xB5A5, 0xB5AA, 0xB5B2, 0xB5B9,
    0xB5D8, 0xB5DB, 0xB5F8, 0xB644, 0xB648, 0xB64F, 0xB652, 0xB657, 0xB669, 0xB671, 0xB67D, 0xB6A1,
    0xB6A4, 0xB6AF, 0xB6C0, 0xB6C7, 0xB6DC, 0xB74E, 0xB750, 0xB751, 0xB752, 0xB773, 0xB77C, 0xB77E,
    0xB7ED, 0xB855, 0xB867, 0xB871, 0xB8A3, 0xB8B9, 0xB8CB, 0xB8CC, 0xB8D1, 0xB8D3, 0xB8D5, 0xB8DC,
    0xB8EA, 0xB8F2, 0xB8F4, 0xB8FB, 0xB942, 0xB944, 0xB946, 0xB94C, 0xB971, 0xB9B3, 0xB9CF, 0xB9DA,
    0xB9EA, 0xB9EF, 0xBA71, 0xBA74, 0xBAA1, 0xBAD8, 0xBADE, 0xBAE2, 0xBAEB, 0xBAF4, 0xBB4F, 0xBB50,
    0xBB79, 0xBB7B, 0xBBA1, 0xBBB7, 0xBBDD, 0xBBF1, 0xBBF2, 0xBBF9, 0xBC67, 0xBC76, 0xBCC6, 0xBCCB,
    0xBCD0, 0xBCD6, 0xBD54, 0xBD75, 0xBDD0, 0xBDD2, 0xBDD6, 0xBDD7, 0xBEB9, 0xBEC7, 0xBED4, 0xBEE3,
    0xBEF7, 0xBF57, 0xBFB3, 0xBFDF, 0xBFEC, 0xBFEF, 0xBFF9, 0xBFFA, 0xBFFD, 0xC059, 0xC073, 0xC0B0,
    0xC0B3, 0xC0C9, 0xC160, 0xC16E, 0xC170, 0xC1BF, 0xC1C2, 0xC1C9, 0xC1D9, 0xC1F6, 0xC249, 0xC2E0,
    0xC344, 0xC3D1, 0xC3E4, 0xC3F6, 0xC3F8, 0xC3FE, 0xC440, 0xC4B1, 0xC4B3, 0xC4D2, 0xC576, 0xC577,
    0xC5A5, 0xC5AA, 0xC5DC, 0xC5E7, 0xC5E9, 0xC5FD, 0xC657, 0xC65B,
];
// @@end generated tables@@

/// 支持的编码（七类：UTF-8、UTF-8 BOM、UTF-16LE、UTF-16BE、GB18030、GBK、Big5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextEncoding {
    /// UTF-8（无 BOM）。
    Utf8,
    /// 带 BOM 的 UTF-8。
    Utf8Bom,
    /// UTF-16 小端（`FF FE`）。
    Utf16Le,
    /// UTF-16 大端（`FE FF`）。
    Utf16Be,
    /// GB18030（含四字节序列）。
    Gb18030,
    /// GBK / CP936（两字节区间与 GB18030 相同）。
    Gbk,
    /// Big5（繁体）。
    Big5,
}

impl TextEncoding {
    /// 全部七类编码。
    pub const ALL: [TextEncoding; 7] = [
        TextEncoding::Utf8,
        TextEncoding::Utf8Bom,
        TextEncoding::Utf16Le,
        TextEncoding::Utf16Be,
        TextEncoding::Gb18030,
        TextEncoding::Gbk,
        TextEncoding::Big5,
    ];

    /// 稳定标识（持久化 / UI 文案键；不得作为业务分支依据以外的用途随意变更）。
    pub const fn as_str(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf8Bom => "UTF-8-BOM",
            TextEncoding::Utf16Le => "UTF-16LE",
            TextEncoding::Utf16Be => "UTF-16BE",
            TextEncoding::Gb18030 => "GB18030",
            TextEncoding::Gbk => "GBK",
            TextEncoding::Big5 => "Big5",
        }
    }

    /// BOM 长度（无 BOM 为 0）。
    pub const fn bom_len(self) -> usize {
        match self {
            TextEncoding::Utf8Bom => 3,
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => 2,
            _ => 0,
        }
    }

    /// 是否带 BOM。
    pub const fn has_bom(self) -> bool {
        self.bom_len() > 0
    }

    /// UTF-8 系（UTF-8 / UTF-8 BOM）。
    pub const fn is_utf8_family(self) -> bool {
        matches!(self, TextEncoding::Utf8 | TextEncoding::Utf8Bom)
    }

    /// UTF-16 系。
    pub const fn is_utf16_family(self) -> bool {
        matches!(self, TextEncoding::Utf16Le | TextEncoding::Utf16Be)
    }

    /// 传统中文编码（GB18030 / GBK / Big5）。
    pub const fn is_legacy_cjk(self) -> bool {
        matches!(
            self,
            TextEncoding::Gb18030 | TextEncoding::Gbk | TextEncoding::Big5
        )
    }

    /// 评分并列时的确定性优先序（数值小者优先；只影响并列，不影响分数）。
    pub const fn preference_rank(self) -> u32 {
        match self {
            TextEncoding::Utf8 => 0,
            TextEncoding::Utf8Bom => 1,
            TextEncoding::Utf16Le => 2,
            TextEncoding::Utf16Be => 3,
            // GBK 是 GB18030 的真子集：两字节内容的解码完全等价，并列时优先窄的那个。
            TextEncoding::Gbk => 4,
            TextEncoding::Gb18030 => 5,
            TextEncoding::Big5 => 6,
        }
    }

    /// 解码等价族（GBK ⊂ GB18030 视为同一族；比较领先优势时按族比较）。
    pub const fn family(self) -> EncodingFamily {
        match self {
            TextEncoding::Utf8 | TextEncoding::Utf8Bom => EncodingFamily::Utf8,
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => EncodingFamily::Utf16,
            TextEncoding::Gb18030 | TextEncoding::Gbk => EncodingFamily::Gb,
            TextEncoding::Big5 => EncodingFamily::Big5,
        }
    }
}

impl core::fmt::Display for TextEncoding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 解码等价族。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncodingFamily {
    /// UTF-8 / UTF-8 BOM。
    Utf8,
    /// UTF-16 LE / BE。
    Utf16,
    /// GB18030 / GBK。
    Gb,
    /// Big5。
    Big5,
}

/// 检测可信度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfidenceLevel {
    /// 低可信度：必须让用户确认或手动指定编码。
    Low,
    /// 中可信度：结果可用，但用户可随时覆盖。
    Medium,
    /// 高可信度：BOM 明确，或 UTF-8 严格合法且内容合理，或传统编码评分明显领先。
    High,
}

impl ConfidenceLevel {
    /// 稳定标识（`low` / `medium` / `high`，与夹具 manifest 一致）。
    pub const fn as_str(self) -> &'static str {
        match self {
            ConfidenceLevel::Low => "low",
            ConfidenceLevel::Medium => "medium",
            ConfidenceLevel::High => "high",
        }
    }
}

/// 单个候选编码的评分明细（诊断与 UI 展示用；不得当作业务分支依据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodingCandidate {
    /// 候选编码。
    pub encoding: TextEncoding,
    /// 0..=1000 的综合评分。
    pub score: u32,
    /// 结构合法字节比例（千分比）。
    pub structural_ok_permille: u32,
    /// 常用字命中率（千分比；按多字节序列计）。
    pub hit_permille: u32,
    /// 多字节序列数（含 GB18030 四字节序列）。
    pub sequences: u32,
    /// 命中常用字码表的序列数。
    pub table_hits: u32,
    /// GB18030 四字节序列数（其余候选恒为 0）。
    pub four_byte_sequences: u32,
    /// 结构非法的字节数（窗口边界伪影已排除）。
    pub invalid_bytes: u32,
    /// 参与评分的字节总数。
    pub total_bytes: u32,
}

impl EncodingCandidate {
    /// 结构上是否成立（非法字节不超过窗口边界伪影预算）。
    pub const fn is_structurally_valid(&self) -> bool {
        self.invalid_bytes <= structural_invalid_budget(self.total_bytes)
    }

    /// 是否达到“候选成立”下限。
    pub const fn is_plausible(&self) -> bool {
        self.structural_ok_permille >= LEGACY_STRUCTURAL_MIN_PERMILLE
    }
}

/// 编码识别结果（开发方案 §7.1 的 `encoding + confidence + encoding_detector_version`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingDetection {
    /// 识别出的编码。
    pub encoding: TextEncoding,
    /// 可信度。
    pub confidence: ConfidenceLevel,
    /// 综合评分（0..=1000）。
    pub score: u32,
    /// 检测器版本。
    pub detector_version: u32,
    /// 命中 BOM 的字节数（0 表示无 BOM）。
    pub bom_len: usize,
    /// 本次检测实际读取（物化）的字节数。
    pub sampled_bytes: u64,
    /// 本次检测实际读取的窗口数。
    pub windows_read: u32,
    /// 数据源总字节数（用于证明“未整本扫描”）。
    pub source_bytes: u64,
    /// UTF-8 路径是否通过（严格合法且内容合理）。
    pub utf8_trustworthy: bool,
    /// 传统编码候选评分（UTF-16 路径下为空；其余路径仅作诊断/覆盖建议）。
    pub legacy_candidates: Vec<EncodingCandidate>,
    /// 解码等价或评分接近的其它候选，供 UI 直接呈现给用户选择。
    pub alternatives: Vec<TextEncoding>,
}

impl EncodingDetection {
    /// 是否需要提示用户确认/选择编码（低可信度且有内容可判）。
    ///
    /// 空文件不提示：无字节可评分，且任何编码对空内容的解码结果相同。
    pub fn needs_confirmation(&self) -> bool {
        self.confidence == ConfidenceLevel::Low && self.sampled_bytes > 0
    }

    /// 是否由 BOM 声明编码。
    pub fn is_bom_declared(&self) -> bool {
        self.bom_len > 0
    }

    /// 本次检测是否覆盖了整个数据源（数据源整体落在头部窗口内）。
    ///
    /// 窗口边界允许 ≤4 字节的对齐裁剪，因此不能用 `sampled_bytes == source_bytes` 判定；
    /// 这里表达的是“采样计划覆盖了全部字节”。
    pub fn sampled_whole_source(&self) -> bool {
        self.source_bytes > 0 && self.source_bytes <= DETECT_HEAD_BYTES as u64
    }

    /// 取某个候选的评分明细。
    pub fn candidate(&self, encoding: TextEncoding) -> Option<&EncodingCandidate> {
        self.legacy_candidates
            .iter()
            .find(|candidate| candidate.encoding == encoding)
    }
}

/// 比较两个编码是否解出同一串内容（V1 范围内只声明字节级等价族）。
pub fn decodes_equivalently(a: TextEncoding, b: TextEncoding) -> bool {
    a == b || (a.family() == b.family() && a.family() == EncodingFamily::Gb)
}

/// 索引失效报告（覆盖编码后内容/章节/分页索引都必须重建）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexInvalidation {
    /// 内容索引（字符/位置）是否失效。
    pub content_index_invalidated: bool,
    /// 章节索引是否失效。
    pub chapter_index_invalidated: bool,
    /// 分页索引是否失效。
    pub pagination_invalidated: bool,
    /// 变更前的 `content_index_version`。
    pub previous_content_index_version: u64,
    /// 变更后的 `content_index_version`。
    pub content_index_version: u64,
}

impl IndexInvalidation {
    /// 未发生失效（版本不变）。
    pub const fn none(version: u64) -> Self {
        Self {
            content_index_invalidated: false,
            chapter_index_invalidated: false,
            pagination_invalidated: false,
            previous_content_index_version: version,
            content_index_version: version,
        }
    }

    /// 是否发生了任何失效。
    pub const fn invalidated(&self) -> bool {
        self.content_index_invalidated
            || self.chapter_index_invalidated
            || self.pagination_invalidated
    }
}

/// 编码状态：识别结果 + 用户覆盖 + `content_index_version`（开发方案 §7.1 编码记录）。
///
/// 覆盖语义：只改变“如何解释原始字节”，因此**不修改原文件**，但必须让内容、章节和
/// 分页索引全部失效（[`IndexInvalidation`]），并把 `content_index_version` 递增。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingState {
    detection: EncodingDetection,
    user_override: Option<TextEncoding>,
    content_index_version: u64,
}

impl EncodingState {
    /// 初始 `content_index_version`（首次导入/首次索引）。
    pub const INITIAL_CONTENT_INDEX_VERSION: u64 = 1;

    /// 由识别结果建立编码状态。
    pub fn new(detection: EncodingDetection) -> Self {
        Self {
            detection,
            user_override: None,
            content_index_version: Self::INITIAL_CONTENT_INDEX_VERSION,
        }
    }

    /// 识别结果。
    pub fn detection(&self) -> &EncodingDetection {
        &self.detection
    }

    /// 检测器版本（持久化字段）。
    pub fn detector_version(&self) -> u32 {
        self.detection.detector_version
    }

    /// 内容索引版本。
    pub fn content_index_version(&self) -> u64 {
        self.content_index_version
    }

    /// 用户指定的编码（未覆盖时为 `None`）。
    pub fn user_override(&self) -> Option<TextEncoding> {
        self.user_override
    }

    /// 实际生效的编码（用户覆盖优先）。
    pub fn effective_encoding(&self) -> TextEncoding {
        self.user_override.unwrap_or(self.detection.encoding)
    }

    /// 实际生效的可信度（用户指定后视为高可信度：由用户负责）。
    pub fn effective_confidence(&self) -> ConfidenceLevel {
        if self.user_override.is_some() {
            ConfidenceLevel::High
        } else {
            self.detection.confidence
        }
    }

    /// 是否仍需用户确认（覆盖后不再需要）。
    pub fn needs_confirmation(&self) -> bool {
        self.user_override.is_none() && self.detection.needs_confirmation()
    }

    /// 用户手动覆盖编码；返回索引失效报告。
    ///
    /// - 覆盖成与当前生效编码等价（[`decodes_equivalently`]）时视为无变更：版本不变、不失效；
    /// - 否则内容/章节/分页索引全部失效，`content_index_version` 递增；
    /// - 原文件字节不会被触碰（本模块不写文件）。
    pub fn override_encoding(&mut self, encoding: TextEncoding) -> IndexInvalidation {
        if decodes_equivalently(self.effective_encoding(), encoding) {
            return IndexInvalidation::none(self.content_index_version);
        }
        self.user_override = Some(encoding);
        self.invalidate()
    }

    /// 清除用户覆盖，回到识别结果；返回索引失效报告（本来就没有覆盖时返回 [`None`]）。
    pub fn clear_override(&mut self) -> Option<IndexInvalidation> {
        self.user_override.take()?;
        Some(self.invalidate())
    }

    fn invalidate(&mut self) -> IndexInvalidation {
        let previous = self.content_index_version;
        self.content_index_version = previous.saturating_add(1);
        IndexInvalidation {
            content_index_invalidated: true,
            chapter_index_invalidated: true,
            pagination_invalidated: true,
            previous_content_index_version: previous,
            content_index_version: self.content_index_version,
        }
    }
}

/// 对数据源做有限窗口编码检测（头部 + 中部/尾部采样，永不整本扫描）。
pub fn detect_encoding(source: &dyn TextSource) -> Result<EncodingDetection, SourceError> {
    let len = source.len();
    if len == 0 {
        return Ok(detect_encoding_from_samples(0, &[]));
    }
    let mut windows: Vec<Vec<u8>> = Vec::new();
    let head_len = DETECT_HEAD_BYTES.min(len as usize);
    windows.push(source.read_at(0, head_len)?.data);
    if len > DETECT_HEAD_BYTES as u64 {
        // 传统编码都是 2/4 字节码元：采样偏移取偶数，避免人为制造奇偶错位。
        let mid = (len / 2) & !1u64;
        let tail = len.saturating_sub(DETECT_SAMPLE_BYTES as u64) & !1u64;
        if mid >= DETECT_HEAD_BYTES as u64 {
            windows.push(source.read_at(mid, DETECT_SAMPLE_BYTES)?.data);
        }
        if tail > mid + DETECT_SAMPLE_BYTES as u64 {
            windows.push(source.read_at(tail, DETECT_SAMPLE_BYTES)?.data);
        }
    }
    let samples: Vec<&[u8]> = windows.iter().map(|window| window.as_slice()).collect();
    Ok(detect_encoding_from_samples(len, &samples))
}

/// 纯函数版检测：给定数据源总字节数与采样窗口，返回识别结果（便于单测与复现）。
///
/// `samples` 是**互不重叠**的采样窗口（通常为头部 + 中部 + 尾部）；本函数只读取这些
/// 字节，不做任何 I/O。
pub fn detect_encoding_from_samples(source_bytes: u64, samples: &[&[u8]]) -> EncodingDetection {
    let sampled_bytes: u64 = samples.iter().map(|sample| sample.len() as u64).sum();
    let windows_read = samples.len() as u32;
    if sampled_bytes == 0 || samples.is_empty() {
        return EncodingDetection {
            encoding: TextEncoding::Utf8,
            confidence: ConfidenceLevel::Low,
            score: 0,
            detector_version: ENCODING_DETECTOR_VERSION,
            bom_len: 0,
            sampled_bytes: 0,
            windows_read: 0,
            source_bytes,
            utf8_trustworthy: true,
            legacy_candidates: Vec::new(),
            alternatives: Vec::new(),
        };
    }

    let bom = detect_bom(samples[0]);
    let payload: Vec<&[u8]> = match bom {
        Some((_, bom_len)) => {
            let mut stripped: Vec<&[u8]> = samples.to_vec();
            stripped[0] = &stripped[0][bom_len..];
            stripped
        }
        None => samples.to_vec(),
    };

    if let Some((encoding, bom_len)) = bom {
        return bom_detection(
            encoding,
            bom_len,
            source_bytes,
            sampled_bytes,
            windows_read,
            samples,
            &payload,
        );
    }

    let legacy_candidates = score_legacy_candidates(samples);
    let utf8 = walk_utf8(samples);
    let suspicious_permille = ratio_permille(utf8.suspicious, utf8.chars);
    let utf8_trustworthy = utf8.valid && suspicious_permille <= UTF8_SUSPICIOUS_MAX_PERMILLE;

    if utf8_trustworthy {
        let confidence = if suspicious_permille == 0 && sampled_bytes >= UTF8_HIGH_MIN_SAMPLED_BYTES
        {
            ConfidenceLevel::High
        } else if sampled_bytes < UTF8_LOW_MAX_SAMPLED_BYTES {
            ConfidenceLevel::Low
        } else {
            ConfidenceLevel::Medium
        };
        let mut alternatives: Vec<TextEncoding> = Vec::new();
        if confidence != ConfidenceLevel::High {
            // 记录“有解释力”的传统编码候选，供用户覆盖时参考。
            for candidate in &legacy_candidates {
                if candidate.is_plausible() {
                    alternatives.push(candidate.encoding);
                }
            }
        }
        return EncodingDetection {
            encoding: TextEncoding::Utf8,
            confidence,
            score: 1000u32.saturating_sub(suspicious_permille),
            detector_version: ENCODING_DETECTOR_VERSION,
            bom_len: 0,
            sampled_bytes,
            windows_read,
            source_bytes,
            utf8_trustworthy: true,
            legacy_candidates,
            alternatives,
        };
    }

    let decision = decide_legacy(&legacy_candidates);
    EncodingDetection {
        encoding: decision.encoding,
        confidence: decision.confidence,
        score: decision.score,
        detector_version: ENCODING_DETECTOR_VERSION,
        bom_len: 0,
        sampled_bytes,
        windows_read,
        source_bytes,
        utf8_trustworthy: false,
        legacy_candidates,
        alternatives: decision.alternatives,
    }
}

struct LegacyDecision {
    encoding: TextEncoding,
    confidence: ConfidenceLevel,
    score: u32,
    alternatives: Vec<TextEncoding>,
}

/// 传统编码评分决策（UTF-8 不可信时）。
fn decide_legacy(candidates: &[EncodingCandidate]) -> LegacyDecision {
    let mut ranked: Vec<&EncodingCandidate> = candidates.iter().collect();
    ranked.sort_by(|a, b| {
        b.score.cmp(&a.score).then(
            a.encoding
                .preference_rank()
                .cmp(&b.encoding.preference_rank()),
        )
    });
    let Some(best) = ranked.first().copied() else {
        return LegacyDecision {
            encoding: TextEncoding::Utf8,
            confidence: ConfidenceLevel::Low,
            score: 0,
            alternatives: Vec::new(),
        };
    };

    let alternatives_from =
        |best: &EncodingCandidate, predicate: &dyn Fn(&EncodingCandidate) -> bool| {
            let mut list: Vec<TextEncoding> = ranked
                .iter()
                .filter(|candidate| candidate.encoding != best.encoding)
                .filter(|candidate| predicate(candidate))
                .map(|candidate| candidate.encoding)
                .collect();
            list.sort_by_key(|encoding| encoding.preference_rank());
            list
        };

    // 1) 没有任何候选成立（结构不合法，或结构合法却几乎没有常用字命中）：
    //    回落到 UTF-8 低可信度，并把得分最高的候选交给用户选择。
    if !best.is_plausible() || best.hit_permille < LEGACY_HIT_MIN_PERMILLE {
        let alternatives = alternatives_from(best, &|_| true);
        return LegacyDecision {
            encoding: TextEncoding::Utf8,
            confidence: ConfidenceLevel::Low,
            score: best.score,
            alternatives,
        };
    }

    // 2) GB18030 四字节独占规则：只有 GB18030 能解释这些字节，GBK 结构上不成立。
    let gb18030 = candidates
        .iter()
        .find(|c| c.encoding == TextEncoding::Gb18030);
    let gbk = candidates.iter().find(|c| c.encoding == TextEncoding::Gbk);
    if let (Some(gb18030), Some(gbk)) = (gb18030, gbk) {
        let four_byte_dominates = gb18030.four_byte_sequences >= 1
            && gb18030.structural_ok_permille > gbk.structural_ok_permille
            && gb18030.is_plausible()
            && gb18030.hit_permille >= LEGACY_HIT_MIN_PERMILLE;
        if four_byte_dominates {
            let confidence =
                if gb18030.is_structurally_valid() && gb18030.score >= LEGACY_SCORE_HIGH_MIN {
                    ConfidenceLevel::High
                } else {
                    ConfidenceLevel::Medium
                };
            return LegacyDecision {
                encoding: TextEncoding::Gb18030,
                confidence,
                score: gb18030.score,
                alternatives: Vec::new(),
            };
        }
    }

    // 3) 常规评分：领先优势按“解码等价族”比较（GBK 与 GB18030 并列不算分歧）。
    let margin = margin_over_other_family(best, candidates);
    let confidence = if best.is_structurally_valid()
        && margin >= LEGACY_MARGIN_HIGH
        && best.score >= LEGACY_SCORE_HIGH_MIN
    {
        ConfidenceLevel::High
    } else if best.is_plausible() && margin >= LEGACY_MARGIN_MEDIUM {
        ConfidenceLevel::Medium
    } else {
        ConfidenceLevel::Low
    };
    let mut alternatives = alternatives_from(best, &|candidate| {
        candidate.is_structurally_valid()
            && best.score.saturating_sub(candidate.score) <= LEGACY_MARGIN_HIGH
    });
    if best.encoding == TextEncoding::Gbk && !alternatives.contains(&TextEncoding::Gb18030) {
        // 两字节区间内 GBK 与 GB18030 字节完全相同：总是把等价候选告知用户。
        alternatives.push(TextEncoding::Gb18030);
        alternatives.sort_by_key(|encoding| encoding.preference_rank());
    }
    LegacyDecision {
        encoding: best.encoding,
        confidence,
        score: best.score,
        alternatives,
    }
}

/// BOM 声明的编码分支。
#[allow(clippy::too_many_arguments)]
fn bom_detection(
    encoding: TextEncoding,
    bom_len: usize,
    source_bytes: u64,
    sampled_bytes: u64,
    windows_read: u32,
    raw_samples: &[&[u8]],
    payload: &[&[u8]],
) -> EncodingDetection {
    let base = EncodingDetection {
        encoding,
        confidence: ConfidenceLevel::High,
        score: 1000,
        detector_version: ENCODING_DETECTOR_VERSION,
        bom_len,
        sampled_bytes,
        windows_read,
        source_bytes,
        utf8_trustworthy: false,
        legacy_candidates: Vec::new(),
        alternatives: Vec::new(),
    };

    match encoding {
        TextEncoding::Utf8Bom => {
            let utf8 = walk_utf8(payload);
            let suspicious_permille = ratio_permille(utf8.suspicious, utf8.chars);
            let trustworthy = utf8.valid && suspicious_permille <= UTF8_SUSPICIOUS_MAX_PERMILLE;
            if trustworthy {
                EncodingDetection {
                    utf8_trustworthy: true,
                    score: 1000u32.saturating_sub(suspicious_permille),
                    ..base
                }
            } else {
                // BOM 与内容矛盾：保留 BOM 声明但降为低可信度，并给出有解释力的传统候选。
                let legacy_candidates = score_legacy_candidates(payload);
                let mut alternatives: Vec<TextEncoding> = legacy_candidates
                    .iter()
                    .filter(|candidate| candidate.is_plausible())
                    .map(|candidate| candidate.encoding)
                    .collect();
                alternatives.sort_by_key(|candidate| candidate.preference_rank());
                EncodingDetection {
                    confidence: ConfidenceLevel::Low,
                    score: 0,
                    legacy_candidates,
                    alternatives,
                    ..base
                }
            }
        }
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let little_endian = encoding == TextEncoding::Utf16Le;
            let utf16 = walk_utf16(payload, little_endian);
            let suspicious_permille = ratio_permille(utf16.suspicious, utf16.chars);
            let trustworthy =
                utf16.chars > 0 && suspicious_permille <= UTF16_SUSPICIOUS_MAX_PERMILLE;
            if trustworthy {
                EncodingDetection {
                    score: 1000u32.saturating_sub(suspicious_permille),
                    ..base
                }
            } else {
                let swapped = if little_endian {
                    TextEncoding::Utf16Be
                } else {
                    TextEncoding::Utf16Le
                };
                EncodingDetection {
                    confidence: ConfidenceLevel::Low,
                    score: 0,
                    alternatives: vec![swapped],
                    ..base
                }
            }
        }
        // 其余编码不会由 BOM 分支产生；保留原始样本用于诊断。
        _ => {
            let legacy_candidates = score_legacy_candidates(raw_samples);
            EncodingDetection {
                confidence: ConfidenceLevel::Low,
                score: 0,
                legacy_candidates,
                ..base
            }
        }
    }
}

/// 识别 BOM（UTF-8 / UTF-16LE / UTF-16BE）。
pub fn detect_bom(sample: &[u8]) -> Option<(TextEncoding, usize)> {
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Some((TextEncoding::Utf8Bom, 3));
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        // 注：UTF-32LE（FF FE 00 00）不在 V1 范围内，此处按 UTF-16LE 处理。
        return Some((TextEncoding::Utf16Le, 2));
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return Some((TextEncoding::Utf16Be, 2));
    }
    None
}

/// 对 GB18030 / GBK / Big5 三个候选做统计评分（顺序固定）。
pub fn score_legacy_candidates(samples: &[&[u8]]) -> Vec<EncodingCandidate> {
    vec![
        score_gb_candidate(TextEncoding::Gb18030, samples),
        score_gb_candidate(TextEncoding::Gbk, samples),
        score_big5_candidate(samples),
    ]
}

const fn is_gbk_lead(byte: u8) -> bool {
    byte >= 0x81 && byte <= 0xFE
}

const fn is_gbk_trail(byte: u8) -> bool {
    byte >= 0x40 && byte <= 0xFE && byte != 0x7F
}

const fn is_big5_lead(byte: u8) -> bool {
    byte >= 0xA1 && byte <= 0xF9
}

const fn is_big5_trail(byte: u8) -> bool {
    (byte >= 0x40 && byte <= 0x7E) || (byte >= 0xA1 && byte <= 0xFE)
}

const fn is_ascii_digit(byte: u8) -> bool {
    byte >= 0x30 && byte <= 0x39
}

/// 传统编码走查的原始计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LegacyTally {
    total: u32,
    valid: u32,
    sequences: u32,
    hits: u32,
    four_byte: u32,
    invalid: u32,
}

/// GBK/GB18030 结构走查 + 常用字命中统计。
///
/// 窗口边界容错：
/// - 样本尾部被截断的序列按“边界伪影”处理（不计非法、不计序列），避免采样切分污染评分；
/// - 采样窗口经 [`crate::TextSource::read_at`] 的 UTF-8 边界修正后可能丢掉首个字节
///   （传统编码的首字节常落在 `0x80..=0xBF`，会被误判为续接字节），因此对“起始相位 0/1”
///   各评一次并取较优 —— 对所有候选同等对待，只补偿采样伪影，不偏袒任何编码。
fn score_gb_candidate(encoding: TextEncoding, samples: &[&[u8]]) -> EncodingCandidate {
    let primary = tally_gb(encoding, samples, false);
    let shifted = if samples
        .iter()
        .any(|sample| sample.first().is_some_and(|byte| *byte >= 0x80))
    {
        tally_gb(encoding, samples, true)
    } else {
        primary
    };
    let candidate = finalize_candidate(encoding, primary);
    let alternative = finalize_candidate(encoding, shifted);
    if alternative.score > candidate.score {
        alternative
    } else {
        candidate
    }
}

fn tally_gb(encoding: TextEncoding, samples: &[&[u8]], shift_phase: bool) -> LegacyTally {
    let allow_four_byte = encoding == TextEncoding::Gb18030;
    let mut tally = LegacyTally::default();

    for sample in samples {
        let bytes = *sample;
        tally.total = tally.total.saturating_add(bytes.len() as u32);
        let mut index = if shift_phase && bytes.first().is_some_and(|byte| *byte >= 0x80) {
            1
        } else {
            0
        };
        while index < bytes.len() {
            let lead = bytes[index];
            if lead < 0x80 {
                tally.valid += 1;
                index += 1;
                continue;
            }
            if !is_gbk_lead(lead) {
                tally.invalid += 1;
                index += 1;
                continue;
            }
            if index + 1 >= bytes.len() {
                // 窗口尾部截断：边界伪影，不奖不罚。
                break;
            }
            let trail = bytes[index + 1];
            if allow_four_byte && is_ascii_digit(trail) {
                if index + 3 >= bytes.len() {
                    break;
                }
                if is_gbk_lead(bytes[index + 2]) && is_ascii_digit(bytes[index + 3]) {
                    tally.four_byte += 1;
                    tally.sequences += 1;
                    tally.valid += 4;
                    index += 4;
                    continue;
                }
                tally.invalid += 1;
                index += 1;
                continue;
            }
            if !is_gbk_trail(trail) {
                tally.invalid += 1;
                index += 1;
                continue;
            }
            let code = ((lead as u16) << 8) | trail as u16;
            tally.sequences += 1;
            tally.valid += 2;
            if GBK_COMMON_CODES.binary_search(&code).is_ok() {
                tally.hits += 1;
            }
            index += 2;
        }
    }
    tally
}

/// Big5 结构走查 + 常用字命中统计（相位补偿规则同 [`tally_gb`]）。
fn score_big5_candidate(samples: &[&[u8]]) -> EncodingCandidate {
    let primary = tally_big5(samples, false);
    let shifted = if samples
        .iter()
        .any(|sample| sample.first().is_some_and(|byte| *byte >= 0x80))
    {
        tally_big5(samples, true)
    } else {
        primary
    };
    let candidate = finalize_candidate(TextEncoding::Big5, primary);
    let alternative = finalize_candidate(TextEncoding::Big5, shifted);
    if alternative.score > candidate.score {
        alternative
    } else {
        candidate
    }
}

fn tally_big5(samples: &[&[u8]], shift_phase: bool) -> LegacyTally {
    let mut tally = LegacyTally::default();

    for sample in samples {
        let bytes = *sample;
        tally.total = tally.total.saturating_add(bytes.len() as u32);
        let mut index = if shift_phase && bytes.first().is_some_and(|byte| *byte >= 0x80) {
            1
        } else {
            0
        };
        while index < bytes.len() {
            let lead = bytes[index];
            if lead < 0x80 {
                tally.valid += 1;
                index += 1;
                continue;
            }
            if !is_big5_lead(lead) {
                tally.invalid += 1;
                index += 1;
                continue;
            }
            if index + 1 >= bytes.len() {
                break;
            }
            let trail = bytes[index + 1];
            if !is_big5_trail(trail) {
                tally.invalid += 1;
                index += 1;
                continue;
            }
            let code = ((lead as u16) << 8) | trail as u16;
            tally.sequences += 1;
            tally.valid += 2;
            if BIG5_COMMON_CODES.binary_search(&code).is_ok() {
                tally.hits += 1;
            }
            index += 2;
        }
    }
    tally
}

fn finalize_candidate(encoding: TextEncoding, tally: LegacyTally) -> EncodingCandidate {
    let total_bytes = tally.total;
    let structural_ok_permille = tally
        .valid
        .saturating_mul(1000)
        .checked_div(total_bytes)
        .map_or(0, |value| value.min(1000));
    let hit_permille = tally
        .hits
        .saturating_mul(1000)
        .checked_div(tally.sequences)
        .map_or(0, |value| value.min(1000));
    let score = (structural_ok_permille.saturating_mul(LEGACY_STRUCTURAL_WEIGHT)
        + hit_permille.saturating_mul(LEGACY_HIT_WEIGHT))
        / 1000;
    EncodingCandidate {
        encoding,
        score,
        structural_ok_permille,
        hit_permille,
        sequences: tally.sequences,
        table_hits: tally.hits,
        four_byte_sequences: tally.four_byte,
        invalid_bytes: tally.invalid,
        total_bytes,
    }
}

/// 领先优势：与“不同解码等价族”中最高分的差值（无其它族时按满分计）。
fn margin_over_other_family(best: &EncodingCandidate, candidates: &[EncodingCandidate]) -> u32 {
    let family = best.encoding.family();
    candidates
        .iter()
        .filter(|candidate| candidate.encoding.family() != family)
        .map(|candidate| candidate.score)
        .max()
        .map_or(1000, |other| best.score.saturating_sub(other))
}

fn ratio_permille(part: u32, whole: u32) -> u32 {
    part.saturating_mul(1000)
        .checked_div(whole)
        .map_or(0, |value| value.min(1000))
}

/// UTF-8 走查结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Utf8Walk {
    valid: bool,
    chars: u32,
    suspicious: u32,
}

/// 样本两端对齐到合法 UTF-8 字符边界（只裁剪窗口边界伪影）。
fn trim_utf8_sample(sample: &[u8]) -> &[u8] {
    let adjustment = align_to_utf8_boundaries(sample);
    let keep = adjustment.keep_range(sample.len());
    &sample[keep]
}

/// 严格 UTF-8 验证 + 可读性统计（不做任何容错替换）。
fn walk_utf8(samples: &[&[u8]]) -> Utf8Walk {
    let mut walk = Utf8Walk {
        valid: true,
        chars: 0,
        suspicious: 0,
    };
    for sample in samples {
        let bytes = trim_utf8_sample(sample);
        let mut index = 0usize;
        while index < bytes.len() {
            match core::str::from_utf8(&bytes[index..]) {
                Ok(text) => {
                    count_chars(text, &mut walk);
                    index = bytes.len();
                }
                Err(error) => {
                    let good = error.valid_up_to();
                    let prefix = core::str::from_utf8(&bytes[index..index + good])
                        .expect("valid_up_to 前缀必然是合法 UTF-8");
                    count_chars(prefix, &mut walk);
                    if error.error_len().is_none() {
                        // 尾部不完整字符：窗口边界伪影，不判非法。
                        index = bytes.len();
                    } else {
                        walk.valid = false;
                        return walk;
                    }
                }
            }
        }
    }
    walk
}

fn count_chars(text: &str, walk: &mut Utf8Walk) {
    for character in text.chars() {
        walk.chars += 1;
        if !char_is_text_plausible(character) {
            walk.suspicious += 1;
        }
    }
}

/// UTF-16 合理性走查（BOM 之后的载荷；代理对合成，未配对代理记可疑）。
fn walk_utf16(samples: &[&[u8]], little_endian: bool) -> Utf8Walk {
    let mut walk = Utf8Walk {
        valid: true,
        chars: 0,
        suspicious: 0,
    };
    let mut pending_high = false;
    for sample in samples {
        let usable = sample.len() - (sample.len() % 2);
        let mut index = 0usize;
        while index + 1 < usable {
            let pair = [sample[index], sample[index + 1]];
            let unit = if little_endian {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            };
            index += 2;
            if (0xD800..0xDC00).contains(&unit) {
                if pending_high {
                    walk.chars += 1;
                    walk.suspicious += 1;
                }
                pending_high = true;
                continue;
            }
            if (0xDC00..0xE000).contains(&unit) {
                walk.chars += 1;
                if !pending_high {
                    walk.suspicious += 1;
                }
                pending_high = false;
                continue;
            }
            if pending_high {
                walk.chars += 1;
                walk.suspicious += 1;
                pending_high = false;
            }
            walk.chars += 1;
            match char::from_u32(unit as u32) {
                Some(character) if char_is_text_plausible(character) => {}
                _ => walk.suspicious += 1,
            }
        }
    }
    if pending_high {
        walk.chars += 1;
        walk.suspicious += 1;
    }
    walk
}

/// 字符是否“像正文”（用于判 UTF-8/UTF-16 结果是否可信）。
///
/// 允许：制表/换行、ASCII 可打印、拉丁补充、常用标点与符号、CJK 标点/表意文字/兼容形式、
/// 假名、谚文、全角半角形式等。**不**允许：C0/C1 控制符、私用区、非字符、
/// Tifinagh 等罕用文字区（这些通常是“字节序/编码判错”造成的乱码）。
pub fn char_is_text_plausible(character: char) -> bool {
    let code = character as u32;
    matches!(character, '\n' | '\r' | '\t')
        || (0x20..=0x7E).contains(&code)
        || (0xA0..=0x24F).contains(&code)
        || (0x2000..=0x2BFF).contains(&code)
        || (0x2E80..=0x2EFF).contains(&code)
        || (0x2F00..=0x2FDF).contains(&code)
        || (0x3000..=0x30FF).contains(&code)
        || (0x3100..=0x312F).contains(&code)
        || (0x3200..=0x33FF).contains(&code)
        || (0x3400..=0x4DBF).contains(&code)
        || (0x4E00..=0x9FFF).contains(&code)
        || (0xA000..=0xA4CF).contains(&code)
        || (0xAC00..=0xD7AF).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
        || (0xFE10..=0xFE4F).contains(&code)
        || (0xFF00..=0xFFEF).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    /// 把文本编码为 UTF-16 字节（测试辅助）。
    fn utf16_bytes(text: &str, little_endian: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for unit in text.encode_utf16() {
            if little_endian {
                out.extend_from_slice(&unit.to_le_bytes());
            } else {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
        out
    }

    /// 用码表前 `count` 个码构造一段纯 GBK 字节流（每两个字节一个常用字）。
    fn gbk_code_block(count: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(count * 2);
        for code in &GBK_COMMON_CODES[..count] {
            bytes.push((code >> 8) as u8);
            bytes.push((code & 0xFF) as u8);
        }
        bytes
    }

    #[test]
    fn encoding_detector_constants_fit_the_frozen_window_contract() {
        // 依据 results/gates/gate-0-baseline.md §2.3（默认 64 KiB / 硬上限 256 KiB）。
        let head = DETECT_HEAD_BYTES;
        let sample = DETECT_SAMPLE_BYTES;
        let total = DETECT_MAX_SAMPLED_BYTES;
        let default_window = crate::DEFAULT_WINDOW_BYTES;
        let max_window = crate::MAX_WINDOW_BYTES;
        assert_eq!(head, 32 * 1024);
        assert_eq!(sample, 16 * 1024);
        assert_eq!(total, head + 2 * sample);
        assert!(head <= default_window, "头部窗口不得超过默认单次窗口");
        assert!(sample <= max_window, "采样窗口不得超过读取硬上限");
        assert!(
            total < max_window,
            "检测总量（{total}）必须小于单次读取硬上限（{max_window}）"
        );
        assert_eq!(
            sample, DEFAULT_SAMPLE_FOR_FIXTURES,
            "夹具 manifest 使用的采样值"
        );
    }

    /// 夹具 manifest 中声明的采样窗口大小（跨语言对照常量）。
    const DEFAULT_SAMPLE_FOR_FIXTURES: usize = 16 * 1024;

    #[test]
    fn encoding_seven_encodings_have_stable_ids_and_bom_lengths() {
        let all = TextEncoding::ALL;
        assert_eq!(all.len(), 7, "V1 支持七类编码");
        let ids: Vec<&str> = all.iter().map(|encoding| encoding.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "UTF-8",
                "UTF-8-BOM",
                "UTF-16LE",
                "UTF-16BE",
                "GB18030",
                "GBK",
                "Big5"
            ]
        );
        let mut ranks: Vec<u32> = all
            .iter()
            .map(|encoding| encoding.preference_rank())
            .collect();
        ranks.sort_unstable();
        ranks.dedup();
        assert_eq!(
            ranks.len(),
            all.len(),
            "并列优先序必须两两不同（保证确定性）"
        );
        assert_eq!(TextEncoding::Utf8Bom.bom_len(), 3);
        assert_eq!(TextEncoding::Utf16Le.bom_len(), 2);
        assert_eq!(TextEncoding::Utf16Be.bom_len(), 2);
        assert_eq!(TextEncoding::Utf8.bom_len(), 0);
        assert_eq!(TextEncoding::Gbk.bom_len(), 0);
        assert!(TextEncoding::Utf8Bom.has_bom());
        assert!(TextEncoding::Utf8.is_utf8_family());
        assert!(TextEncoding::Utf16Be.is_utf16_family());
        assert!(TextEncoding::Big5.is_legacy_cjk());
        assert!(!TextEncoding::Utf8.is_legacy_cjk());
        assert_eq!(TextEncoding::Gbk.to_string(), "GBK");
        assert_eq!(TextEncoding::Gb18030.family(), TextEncoding::Gbk.family());
        assert_eq!(
            TextEncoding::Utf16Le.family(),
            TextEncoding::Utf16Be.family(),
            "UTF-16 两字节序同族（但不等价，见等价测试）"
        );
    }

    #[test]
    fn encoding_bom_detection_covers_utf8_and_utf16() {
        assert_eq!(
            detect_bom(&[0xEF, 0xBB, 0xBF, b'a']),
            Some((TextEncoding::Utf8Bom, 3))
        );
        assert_eq!(
            detect_bom(&[0xFF, 0xFE, 0x61, 0x00]),
            Some((TextEncoding::Utf16Le, 2))
        );
        assert_eq!(
            detect_bom(&[0xFE, 0xFF, 0x00, 0x61]),
            Some((TextEncoding::Utf16Be, 2))
        );
        assert_eq!(detect_bom(b"hello world"), None);
        assert_eq!(detect_bom(&[]), None);
        assert_eq!(detect_bom(&[0xEF, 0xBB]), None, "不完整 BOM 不得判定");

        let mut sample = vec![0xEF, 0xBB, 0xBF];
        sample.extend_from_slice("第一章 雨夜孤灯".as_bytes());
        let detection = detect_encoding_from_samples(sample.len() as u64, &[&sample]);
        assert_eq!(detection.encoding, TextEncoding::Utf8Bom);
        assert_eq!(detection.confidence, ConfidenceLevel::High);
        assert_eq!(detection.bom_len, 3);
        assert!(detection.is_bom_declared());
        assert!(detection.utf8_trustworthy);
        assert!(!detection.needs_confirmation());
        assert_eq!(detection.detector_version, ENCODING_DETECTOR_VERSION);
        assert_eq!(detection.sampled_bytes, sample.len() as u64);
    }

    #[test]
    fn encoding_utf16_bom_mismatch_is_low_confidence_with_alternative() {
        let text = "第一章 夜雨孤灯，少年走进那间窄小的书铺。";
        let little_endian = utf16_bytes(text, true);
        let big_endian = utf16_bytes(text, false);
        assert_eq!(little_endian.len(), big_endian.len());

        let mut good = vec![0xFF, 0xFE];
        good.extend_from_slice(&little_endian);
        let detection = detect_encoding_from_samples(good.len() as u64, &[&good]);
        assert_eq!(detection.encoding, TextEncoding::Utf16Le);
        assert_eq!(detection.confidence, ConfidenceLevel::High);
        assert!(!detection.needs_confirmation());

        // LE BOM + BE 载荷：BOM 与内容矛盾 → 保留 BOM 声明但降为低可信度并给出替代。
        let mut bad = vec![0xFF, 0xFE];
        bad.extend_from_slice(&big_endian);
        let detection = detect_encoding_from_samples(bad.len() as u64, &[&bad]);
        assert_eq!(detection.encoding, TextEncoding::Utf16Le);
        assert_eq!(detection.confidence, ConfidenceLevel::Low);
        assert!(detection.needs_confirmation());
        assert_eq!(detection.alternatives, vec![TextEncoding::Utf16Be]);
        assert!(
            detection.legacy_candidates.is_empty(),
            "UTF-16 路径不产出传统编码候选"
        );
    }

    #[test]
    fn encoding_short_ascii_sample_is_low_confidence() {
        let sample = b"Chapter 1\nHello.";
        let detection = detect_encoding_from_samples(sample.len() as u64, &[sample]);
        assert_eq!(detection.encoding, TextEncoding::Utf8);
        assert_eq!(
            detection.confidence,
            ConfidenceLevel::Low,
            "样本过短不得判高可信"
        );
        assert!(detection.utf8_trustworthy);
        assert!(detection.needs_confirmation());
    }

    #[test]
    fn encoding_private_use_utf8_is_untrustworthy_and_falls_back() {
        // 合法的 UTF-8 私用区内容：结构合法但没有任何常用字命中 → 必须低可信度。
        let text: String = (0..64u32)
            .map(|offset| char::from_u32(0xE000 + offset).expect("私用区码点"))
            .collect();
        let bytes = text.as_bytes();
        let detection = detect_encoding_from_samples(bytes.len() as u64, &[bytes]);
        assert!(!detection.utf8_trustworthy, "私用区内容不得视为可信 UTF-8");
        assert_eq!(
            detection.encoding,
            TextEncoding::Utf8,
            "无成立候选时回落 UTF-8"
        );
        assert_eq!(detection.confidence, ConfidenceLevel::Low);
        assert!(detection.needs_confirmation());
        for candidate in &detection.legacy_candidates {
            assert!(
                candidate.hit_permille < LEGACY_HIT_MIN_PERMILLE,
                "{} 的巧合命中率（{}‰）必须低于成立下限，否则不得回落",
                candidate.encoding,
                candidate.hit_permille
            );
        }
    }

    #[test]
    fn encoding_phase_compensation_recovers_sample_lost_leading_byte() {
        let block = gbk_code_block(48);
        let full = score_legacy_candidates(&[&block]);
        let gbk = full
            .iter()
            .find(|candidate| candidate.encoding == TextEncoding::Gbk)
            .expect("GBK 候选");
        assert_eq!(gbk.hit_permille, 1000, "码表码构成的字节流必须 100% 命中");
        assert_eq!(gbk.invalid_bytes, 0);

        // 模拟 TextSource::read_at 的 UTF-8 首部对齐：丢掉首字节后对齐被破坏。
        let shifted = &block[1..];
        let candidates = score_legacy_candidates(&[shifted]);
        let gbk = candidates
            .iter()
            .find(|candidate| candidate.encoding == TextEncoding::Gbk)
            .expect("GBK 候选");
        assert_eq!(
            gbk.invalid_bytes, 0,
            "相位补偿必须把落单首字节当边界伪影而不是非法字节"
        );
        assert_eq!(
            gbk.hit_permille, 1000,
            "相位补偿必须恢复真实对齐（实测 {}）",
            gbk.hit_permille
        );
        assert!(
            gbk.score >= 950,
            "补偿后评分应接近满分（实测 {}）",
            gbk.score
        );
    }

    #[test]
    fn encoding_plausibility_classifier_rejects_garbage_ranges() {
        for plausible in [
            '中', 'a', '\n', '\t', '，', '！', '한', 'あ', '—', '…', '（',
        ] {
            assert!(
                char_is_text_plausible(plausible),
                "{plausible:?} 应视为正文可读字符"
            );
        }
        for implausible in [
            '\u{0}',
            '\u{7}',
            '\u{E000}',
            '\u{2D4E}',
            '\u{FFFD}',
            '\u{10FFFF}',
        ] {
            assert!(
                !char_is_text_plausible(implausible),
                "{implausible:?} 应视为乱码/不可读字符"
            );
        }
    }

    #[test]
    fn encoding_empty_and_single_byte_samples_are_handled() {
        let empty = detect_encoding_from_samples(0, &[]);
        assert_eq!(empty.encoding, TextEncoding::Utf8);
        assert_eq!(empty.confidence, ConfidenceLevel::Low);
        assert_eq!(empty.sampled_bytes, 0);
        assert_eq!(empty.windows_read, 0);
        assert!(!empty.needs_confirmation(), "空文件没有内容可确认");
        assert!(empty.legacy_candidates.is_empty());
        assert!(
            !empty.sampled_whole_source(),
            "0 字节数据源不算“覆盖整个数据源”"
        );

        let one = *b"a";
        let samples: [&[u8]; 1] = [&one];
        let single = detect_encoding_from_samples(1, &samples);
        assert_eq!(single.encoding, TextEncoding::Utf8);
        assert_eq!(single.confidence, ConfidenceLevel::Low);
        assert!(single.needs_confirmation());
        assert_eq!(single.windows_read, 1);
    }

    #[test]
    fn encoding_override_and_equivalence_helpers_agree() {
        assert!(decodes_equivalently(
            TextEncoding::Gbk,
            TextEncoding::Gb18030
        ));
        assert!(decodes_equivalently(
            TextEncoding::Gb18030,
            TextEncoding::Gbk
        ));
        assert!(decodes_equivalently(TextEncoding::Utf8, TextEncoding::Utf8));
        assert!(!decodes_equivalently(
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be
        ));
        assert!(!decodes_equivalently(TextEncoding::Gbk, TextEncoding::Big5));
        assert!(!decodes_equivalently(
            TextEncoding::Utf8,
            TextEncoding::Utf8Bom
        ));

        let unchanged = IndexInvalidation::none(7);
        assert!(!unchanged.invalidated());
        assert_eq!(unchanged.previous_content_index_version, 7);
        assert_eq!(unchanged.content_index_version, 7);

        let block = gbk_code_block(64);
        let detection = detect_encoding_from_samples(block.len() as u64, &[&block]);
        assert_eq!(detection.encoding, TextEncoding::Gbk);
        let mut state = EncodingState::new(detection);
        assert_eq!(
            state.content_index_version(),
            EncodingState::INITIAL_CONTENT_INDEX_VERSION
        );
        assert_eq!(state.detector_version(), ENCODING_DETECTOR_VERSION);
        assert!(state.user_override().is_none());
        assert_eq!(state.effective_confidence(), ConfidenceLevel::High);
        let invalidation = state.override_encoding(TextEncoding::Big5);
        assert!(invalidation.invalidated());
        assert_eq!(
            state.content_index_version(),
            EncodingState::INITIAL_CONTENT_INDEX_VERSION + 1
        );
        assert_eq!(state.effective_encoding(), TextEncoding::Big5);
        assert_eq!(state.effective_confidence(), ConfidenceLevel::High);
    }

    #[test]
    fn encoding_detects_rc2_utf8_fixtures_without_new_files() {
        // 与 RC2 已交付夹具的交叉检查：既有 UTF-8 夹具必须被识别为 UTF-8 系。
        let plain = test_support::fixture_source("utf8-mixed-lines.txt");
        let detection = detect_encoding(&plain).expect("夹具检测");
        assert_eq!(detection.encoding, TextEncoding::Utf8);
        assert_eq!(detection.confidence, ConfidenceLevel::High);
        assert!(detection.utf8_trustworthy);

        let bom = test_support::fixture_source("utf8-bom.txt");
        let detection = detect_encoding(&bom).expect("夹具检测");
        assert_eq!(detection.encoding, TextEncoding::Utf8Bom);
        assert_eq!(detection.bom_len, 3);
        assert_eq!(detection.confidence, ConfidenceLevel::High);

        let empty = test_support::fixture_source("empty.txt");
        let detection = detect_encoding(&empty).expect("空夹具检测");
        assert_eq!(detection.sampled_bytes, 0);
        assert_eq!(detection.confidence, ConfidenceLevel::Low);
        assert!(!detection.needs_confirmation());
    }
}

/// 按编码把一段字节（通常是一行）解码为文本；无法构成合法文本时返回 `None`。
///
/// 用于章节扫描：**必须先按文件实际编码解码**再匹配章节规则。
/// 真机缺陷背景：扫描曾对每行做 `std::str::from_utf8`，GBK 行必然失败 ⇒ 整行被丢弃 ⇒
/// 目录只剩一个虚拟章节「全文」。
///
/// UTF-8 走零拷贝借用；其他编码用 `encoding_rs` 解码（GB18030 覆盖 GBK 的码位空间）。
pub fn decode_line<'a>(bytes: &'a [u8], encoding: TextEncoding) -> Option<std::borrow::Cow<'a, str>> {
    if bytes.is_empty() {
        return Some(std::borrow::Cow::Borrowed(""));
    }
    if matches!(encoding, TextEncoding::Utf8 | TextEncoding::Utf8Bom) {
        return std::str::from_utf8(bytes).ok().map(std::borrow::Cow::Borrowed);
    }
    let codec = match encoding {
        TextEncoding::Utf16Le => encoding_rs::UTF_16LE,
        TextEncoding::Utf16Be => encoding_rs::UTF_16BE,
        TextEncoding::Gb18030 | TextEncoding::Gbk => encoding_rs::GB18030,
        TextEncoding::Big5 => encoding_rs::BIG5,
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => encoding_rs::UTF_8,
    };
    let (decoded, _encoding_used, _had_errors) = codec.decode(bytes);
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}
