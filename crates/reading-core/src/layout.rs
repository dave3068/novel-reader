//! `LayoutSignature`：排版签名的**冻结字段集**与确定性缓存键。
//!
//! 依据 `results/gates/gate-0-baseline.md` §2.5（布局签名字段冻结清单）与
//! `reading-core/spec.md` CR4（「用包含视口、字体、字号、行距、段距、缩进、边距及算法版本的
//! 布局签名管理分页缓存」）。
//!
//! # 字段（冻结 13 项）
//!
//! 视口宽 / 视口高 / 方向 / 字体族 / 字重 / 字号 / 行距 / 段距 / 首行缩进 /
//! 水平边距 / 垂直边距 / 排版算法版本 / 平台字体指纹（[`LAYOUT_SIGNATURE_FIELDS`]）。
//!
//! # 为什么全部用整数毫单位
//!
//! 浮点在跨平台（x86_64 / arm64）与跨语言（Rust / C / Kotlin）之间没有逐位一致的保证，
//! 而布局签名是**缓存键**：`20.0` 与 `19.999999999999996` 必须被判定为同一个签名，
//! 否则每次窗口映射都会误判「设置变化」并清空全部分页缓存。因此所有尺寸都用
//! 千分之一单位的 `u32`（`20 px` → `20000`），并由平台适配层在测量前完成取整。
//!
//! # 与 UI 的边界
//!
//! 本模块**不做任何文本测量或字形光栅**：签名只描述「用哪套排版参数测量的」，
//! 真实测量由 desktop（Slint）与 android（Compose）各自的适配层完成，
//! 测量出的 `end_byte` 再回到 `reading-index::boundary` 校验（卡片 RC5 禁止事项）。

use reading_source::hash_bytes;

use crate::error::{ErrorCode, NrError, NrResult};

/// 排版算法版本（冻结起始值 `1`）。
///
/// 算法版本属于布局签名：一旦排版语义变化，该版本必须递增，
/// 从而让全部旧分页边界自动失效（`docs/novel-reader-v1-development-plan.md` §部署与升级）。
pub const LAYOUT_ALGORITHM_VERSION: u32 = 1;

/// 布局签名的冻结字段数量（不含 [`LayoutKey`] 的 `content_index_version`）。
pub const LAYOUT_SIGNATURE_FIELDS: usize = 13;

/// 布局签名编码前缀（版本化；不兼容变更必须换前缀）。
pub const LAYOUT_SIGNATURE_ENCODING_PREFIX: &str = "nrlay1";

/// 默认字重（常规体，CSS 口径 400）。
pub const DEFAULT_FONT_WEIGHT: u16 = 400;

/// 排版方向（冻结 2 项：横向 / 纵向）。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextDirection {
    /// 横向（默认）。
    Horizontal = 0,
    /// 纵向。
    Vertical = 1,
}

impl TextDirection {
    /// 稳定数值（落库/日志用）。
    pub const fn id(self) -> i32 {
        self as i32
    }

    /// 稳定短名。
    pub const fn as_str(self) -> &'static str {
        match self {
            TextDirection::Horizontal => "Horizontal",
            TextDirection::Vertical => "Vertical",
        }
    }

    /// 由稳定数值还原。
    pub const fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(TextDirection::Horizontal),
            1 => Some(TextDirection::Vertical),
            _ => None,
        }
    }
}

/// 布局签名：一组**决定分页结果**的排版参数。
///
/// 相等即代表「同一套排版参数」，可复用同一份分页边界；任一字段变化都会改变
/// [`LayoutSignature::layout_id`] 并触发旧边界失效（AC4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutSignature {
    /// 正文区域宽度（像素，窗口映射后的实际可用宽度）。
    pub viewport_width: u32,
    /// 正文区域高度（像素）。
    pub viewport_height: u32,
    /// 排版方向。
    pub direction: TextDirection,
    /// 字体族（平台字体族名，如 `Noto Sans CJK SC`）。
    pub font_family: String,
    /// 字重。
    pub font_weight: u16,
    /// 字号（千分之一像素）。
    pub font_size_milli: u32,
    /// 行距（千分之一像素）。
    pub line_spacing_milli: u32,
    /// 段距（千分之一像素）。
    pub paragraph_spacing_milli: u32,
    /// 首行缩进（千分之一像素）。
    pub first_line_indent_milli: u32,
    /// 水平边距（千分之一像素，左右各一份）。
    pub margin_horizontal_milli: u32,
    /// 垂直边距（千分之一像素，上下各一份）。
    pub margin_vertical_milli: u32,
    /// 排版算法版本。
    pub algorithm_version: u32,
    /// 平台字体指纹（字体度量指纹；由平台适配层提供，`0` 表示尚未取得）。
    pub platform_font_fingerprint: u64,
}

impl Default for LayoutSignature {
    fn default() -> Self {
        Self {
            viewport_width: 1080,
            viewport_height: 1920,
            direction: TextDirection::Horizontal,
            font_family: "Noto Sans CJK SC".to_string(),
            font_weight: DEFAULT_FONT_WEIGHT,
            font_size_milli: 20_000,
            line_spacing_milli: 8_000,
            paragraph_spacing_milli: 12_000,
            first_line_indent_milli: 40_000,
            margin_horizontal_milli: 24_000,
            margin_vertical_milli: 24_000,
            algorithm_version: LAYOUT_ALGORITHM_VERSION,
            platform_font_fingerprint: 0,
        }
    }
}

impl LayoutSignature {
    /// 签名自检：视口、字号必须为正，字体族必须非空，算法版本必须已冻结。
    ///
    /// 非法签名必须是**错误**而不是「一个特殊的合法签名」：否则错误的排版参数会静默写入
    /// 分页缓存，并在窗口尺寸为 0 时产出无穷多空页面。
    pub fn validate(&self) -> NrResult<()> {
        if self.viewport_width == 0 || self.viewport_height == 0 {
            return Err(invalid(format!(
                "视口尺寸必须为正: {}x{}",
                self.viewport_width, self.viewport_height
            )));
        }
        if self.font_size_milli == 0 {
            return Err(invalid("字号必须为正".to_string()));
        }
        if self.font_family.trim().is_empty() {
            return Err(invalid("字体族不得为空".to_string()));
        }
        if self.algorithm_version == 0 {
            return Err(invalid("排版算法版本不得为 0".to_string()));
        }
        Ok(())
    }

    /// 确定性的稳定文本形式（缓存键与日志用）。
    ///
    /// 字段顺序即 [`LAYOUT_SIGNATURE_FIELDS`] 的冻结顺序；字体族按十六进制编码，
    /// 以免分隔符出现在字体名里造成歧义。
    pub fn encode(&self) -> String {
        format!(
            "{prefix}:w={w}:h={h}:dir={dir}:family={family}:weight={weight}:size={size}:\
             line={line}:para={para}:indent={indent}:margin_h={mh}:margin_v={mv}:algo={algo}:\
             fontfp={fontfp}",
            prefix = LAYOUT_SIGNATURE_ENCODING_PREFIX,
            w = self.viewport_width,
            h = self.viewport_height,
            dir = self.direction.as_str(),
            family = hex_encode(self.font_family.as_bytes()),
            weight = self.font_weight,
            size = self.font_size_milli,
            line = self.line_spacing_milli,
            para = self.paragraph_spacing_milli,
            indent = self.first_line_indent_milli,
            mh = self.margin_horizontal_milli,
            mv = self.margin_vertical_milli,
            algo = self.algorithm_version,
            fontfp = self.platform_font_fingerprint,
        )
    }

    /// 签名的确定性哈希（FNV-1a 64，与文件指纹同一算法）。
    pub fn layout_id(&self) -> u64 {
        hash_bytes(self.encode().as_bytes())
    }

    /// 平台字体指纹的确定性派生值。
    ///
    /// 仅用于平台尚未提供真实字体度量指纹时的**一致退化**：同一字体族/字重/字号必须得到
    /// 同一数值，否则同一设备的相邻两次测量会被判为不同签名。平台一旦能提供真实度量指纹，
    /// 应直接写入 [`LayoutSignature::platform_font_fingerprint`]。
    pub fn font_fingerprint(font_family: &str, font_weight: u16, font_size_milli: u32) -> u64 {
        let mut seed = Vec::with_capacity(font_family.len() + 8);
        seed.extend_from_slice(font_family.as_bytes());
        seed.extend_from_slice(&font_weight.to_le_bytes());
        seed.extend_from_slice(&font_size_milli.to_le_bytes());
        hash_bytes(&seed)
    }
}

/// 分页缓存键：`layout_signature + content_index_version`（gate-0 §2.5 冻结规则）。
///
/// 章节索引重建（编码切换、文件指纹变化）会改变 `content_index_version`，
/// 从而让旧缓存键整体失效，而**不影响**书籍、进度与书签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutKey {
    /// 排版签名。
    pub signature: LayoutSignature,
    /// 内容索引版本（`reading_index::CHAPTER_INDEX_VERSION` 或上层索引版本）。
    pub content_index_version: u32,
}

impl LayoutKey {
    /// 组合缓存键。
    pub fn new(signature: LayoutSignature, content_index_version: u32) -> Self {
        Self {
            signature,
            content_index_version,
        }
    }

    /// 缓存键的确定性哈希（= 签名哈希 + 内容索引版本，二者任一变化都会改变该值）。
    pub fn signature_hash(&self) -> u64 {
        let mut seed = Vec::with_capacity(24);
        seed.extend_from_slice(&self.signature.layout_id().to_le_bytes());
        seed.extend_from_slice(&self.content_index_version.to_le_bytes());
        hash_bytes(&seed)
    }
}

/// 生成非法排版参数的统一错误。
fn invalid(message: String) -> NrError {
    NrError::new(ErrorCode::InvalidArgument, message)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0F), 16).unwrap_or('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_signature_covers_frozen_field_set() {
        let signature = LayoutSignature::default();
        assert_eq!(signature.algorithm_version, LAYOUT_ALGORITHM_VERSION);
        let field_count: usize = LAYOUT_SIGNATURE_FIELDS;
        assert_eq!(field_count, 13);
        // 冻结字段必须全部参与编码（逐项改名/改值时编码都会变）。
        let encoded = signature.encode();
        for expected in [
            "nrlay1",
            "w=1080",
            "h=1920",
            "dir=Horizontal",
            "family=",
            "weight=400",
            "size=20000",
            "line=8000",
            "para=12000",
            "indent=40000",
            "margin_h=24000",
            "margin_v=24000",
            "algo=1",
            "fontfp=0",
        ] {
            assert!(encoded.contains(expected), "编码缺少 {expected}: {encoded}");
        }
        assert!(signature.validate().is_ok());
        assert_eq!(TextDirection::from_id(0), Some(TextDirection::Horizontal));
        assert_eq!(TextDirection::from_id(1), Some(TextDirection::Vertical));
        assert_eq!(TextDirection::Vertical.id(), 1);
        assert_eq!(TextDirection::from_id(2), None);
    }

    #[test]
    fn layout_signature_change_changes_hash_and_default_is_stable() {
        let base = LayoutSignature::default();
        let again = LayoutSignature::default();
        assert_eq!(base.layout_id(), again.layout_id(), "同参数必须同哈希");

        let mut metrics = [
            LayoutSignature {
                viewport_width: 1081,
                ..base.clone()
            },
            LayoutSignature {
                viewport_height: 1921,
                ..base.clone()
            },
            LayoutSignature {
                direction: TextDirection::Vertical,
                ..base.clone()
            },
            LayoutSignature {
                font_family: "Serif".to_string(),
                ..base.clone()
            },
            LayoutSignature {
                font_weight: 700,
                ..base.clone()
            },
            LayoutSignature {
                font_size_milli: 24_000,
                ..base.clone()
            },
            LayoutSignature {
                line_spacing_milli: 9_000,
                ..base.clone()
            },
            LayoutSignature {
                paragraph_spacing_milli: 13_000,
                ..base.clone()
            },
            LayoutSignature {
                first_line_indent_milli: 0,
                ..base.clone()
            },
            LayoutSignature {
                margin_horizontal_milli: 25_000,
                ..base.clone()
            },
            LayoutSignature {
                margin_vertical_milli: 25_000,
                ..base.clone()
            },
            LayoutSignature {
                algorithm_version: LAYOUT_ALGORITHM_VERSION + 1,
                ..base.clone()
            },
            LayoutSignature {
                platform_font_fingerprint: 42,
                ..base.clone()
            },
        ];
        for candidate in &mut metrics {
            assert_ne!(
                candidate.layout_id(),
                base.layout_id(),
                "任一冻结字段变化都必须改变签名哈希: {candidate:?}"
            );
        }
    }

    #[test]
    fn layout_key_mixes_signature_and_content_index_version() {
        let signature = LayoutSignature::default();
        let key = LayoutKey::new(signature.clone(), 1);
        let same = LayoutKey::new(signature.clone(), 1);
        let other_version = LayoutKey::new(signature.clone(), 2);
        let other_signature = LayoutKey::new(
            LayoutSignature {
                font_size_milli: 30_000,
                ..signature
            },
            1,
        );
        assert_eq!(key.signature_hash(), same.signature_hash());
        assert_ne!(key.signature_hash(), other_version.signature_hash());
        assert_ne!(key.signature_hash(), other_signature.signature_hash());
    }

    #[test]
    fn layout_signature_rejects_invalid_viewport_and_font() {
        let zero_width = LayoutSignature {
            viewport_width: 0,
            ..LayoutSignature::default()
        };
        assert_eq!(
            zero_width.validate().expect_err("零宽视口必须被拒").code(),
            ErrorCode::InvalidArgument
        );
        let zero_height = LayoutSignature {
            viewport_height: 0,
            ..LayoutSignature::default()
        };
        assert!(zero_height.validate().is_err());
        let zero_font = LayoutSignature {
            font_size_milli: 0,
            ..LayoutSignature::default()
        };
        assert!(zero_font.validate().is_err());
        let blank_family = LayoutSignature {
            font_family: "   ".to_string(),
            ..LayoutSignature::default()
        };
        assert!(blank_family.validate().is_err());
        let zero_algo = LayoutSignature {
            algorithm_version: 0,
            ..LayoutSignature::default()
        };
        assert!(zero_algo.validate().is_err());
    }

    #[test]
    fn layout_font_fingerprint_is_deterministic_and_sensitive() {
        let base = LayoutSignature::font_fingerprint("Noto Sans CJK SC", 400, 20_000);
        assert_eq!(
            base,
            LayoutSignature::font_fingerprint("Noto Sans CJK SC", 400, 20_000)
        );
        assert_ne!(
            base,
            LayoutSignature::font_fingerprint("Serif", 400, 20_000)
        );
        assert_ne!(
            base,
            LayoutSignature::font_fingerprint("Noto Sans CJK SC", 700, 20_000)
        );
        assert_ne!(
            base,
            LayoutSignature::font_fingerprint("Noto Sans CJK SC", 400, 20_001)
        );
        assert_ne!(base, 0, "派生指纹不得为 0（0 保留给「尚未取得」）");
    }
}
