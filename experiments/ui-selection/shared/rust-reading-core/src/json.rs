//! 极简 JSON 编码器（零依赖）。
//!
//! 跨语言边界（C ABI/JNI）只传 UTF-8 字节串，不传语言对象，因此批次数据
//! 统一编码为该模块产出的 JSON 文本。

/// 转义 JSON 字符串字面量内容。
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// 转义并加引号。
pub fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    out.push_str(&escape(s));
    out.push('"');
    out
}

/// 浮点数字面量（保留 4 位小数，避免 NaN/Inf 进入 JSON）。
pub fn number(v: f32) -> String {
    if v.is_finite() {
        format!("{:.4}", v)
    } else {
        "0.0000".to_string()
    }
}
