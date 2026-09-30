//! 头文件 —— Rust 对照测试：把"头文件与 Rust 导出符号逐项一致"变成机器校验。
//!
//! 本测试直接解析 `include/novel_reader.h`（而不是解析一份手抄清单），逐项比对：
//!
//! 1. 函数：18 项声明必须与 [`reading_ffi::abi::ABI_FUNCTIONS`] 的返回类型 + 参数类型逐项一致
//!    （参数名被忽略，类型必须完全一致），且声明顺序与冻结入口清单一致；
//! 2. 结构体：`ABI_STRUCTS` 登记的每个结构体，其字段名、C 类型、出现顺序必须与头文件一致；
//! 3. 宏：错误码、编码、数据源种类、提交阶段、页边界状态与拒绝原因、窗口上限等
//!    必须与 Rust 常量同数值；
//! 4. 三处 ABI 版本（头文件宏 / Rust 常量 / `nr_abi_version`）必须一致。
//!
//! 任何一侧改动而另一侧未同步，本测试立即失败——这正是 reading-core NFR-C4 的要求。

use std::collections::BTreeMap;
use std::path::PathBuf;

use reading_ffi::abi::{self, ABI_FUNCTIONS};
use reading_ffi::structs::ABI_STRUCTS;

fn header_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("include")
        .join("novel_reader.h")
}

fn header_text() -> String {
    let path = header_path();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取头文件 {} 失败: {error}", path.display()))
}

/// 去掉全部空白：头文件与 Rust 清单用同一套"规范化后比较"的规则。
fn normalize(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 把一条声明拆成 `(C 类型, 名字)`；名字可能为空（末 token 只有 `*`，或清单只给类型）。
fn split_declaration(declaration: &str) -> (String, String) {
    let decl = declaration.trim().trim_end_matches(';').trim();
    if decl.contains("(*") {
        // 函数指针声明：`int32_t (*read_window)(void *, uint64_t, ...)`
        let normalized = normalize(decl);
        let start = normalized.find("(*").expect("函数指针声明");
        let close = normalized[start + 2..]
            .find(')')
            .map(|offset| start + 2 + offset)
            .expect("函数指针声明必须有闭合括号");
        let name = normalized[start + 2..close].to_string();
        let ty = format!("{}(*){}", &normalized[..start], &normalized[close + 1..]);
        return (ty, name);
    }
    let last_start = decl
        .rfind(|c: char| c.is_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0);
    let last = &decl[last_start..];
    let stars = last.len() - last.trim_start_matches('*').len();
    let name = last.trim_start_matches('*').to_string();
    if name.is_empty() {
        // 末 token 只有 `*`：名字为空，整行就是类型。
        return (normalize(decl), String::new());
    }
    let head = decl[..last_start].trim_end();
    (normalize(&format!("{head} {}", "*".repeat(stars))), name)
}

/// 规范化一条函数声明为 `返回类型(参数类型,...)`。
///
/// `strip_param_names = true` 用于头文件声明（参数名被去掉）；
/// `false` 用于 Rust 清单（清单里本来就只写类型）。
fn normalize_signature(declaration: &str, strip_param_names: bool) -> String {
    let decl = declaration.trim().trim_end_matches(';').trim();
    let Some(open) = decl.find('(') else {
        return normalize(decl);
    };
    let Some(close) = decl.rfind(')') else {
        return normalize(decl);
    };
    let head = decl[..open].trim();
    let return_type = head.split_whitespace().next().unwrap_or(head).to_string();
    let params = decl[open + 1..close].trim();
    if params.is_empty() {
        return format!("{return_type}()");
    }
    let mut types: Vec<String> = Vec::new();
    for param in params.split(',') {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }
        if param == "void" {
            types.push("void".to_string());
            continue;
        }
        if strip_param_names {
            let (ty, _name) = split_declaration(param);
            types.push(ty);
        } else {
            types.push(normalize(param));
        }
    }
    format!("{return_type}({})", types.join(","))
}

/// 解析头文件中的结构体：结构体名 → 有序 `(C 类型, 字段名)`。
fn parse_structs(text: &str) -> BTreeMap<String, Vec<(String, String)>> {
    let mut structs: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("typedef struct ") {
            let name = rest
                .split_whitespace()
                .next()
                .expect("结构体名")
                .to_string();
            current = Some(name.clone());
            structs.entry(name).or_default();
            continue;
        }
        let Some(name) = current.clone() else {
            continue;
        };
        if trimmed.starts_with('}') {
            let alias = trimmed
                .trim_start_matches('}')
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
            assert_eq!(
                alias, name,
                "typedef 别名必须与结构体名一致: {alias} != {name}"
            );
            current = None;
            continue;
        }
        // 行尾可能跟块注释（`uint32_t reserved0;  /* 必须为 0 */`）：先剥掉注释再解析。
        let code = trimmed.split("/*").next().unwrap_or("").trim();
        if code.is_empty() || !code.ends_with(';') {
            continue;
        }
        let (ty, field) = split_declaration(code);
        structs
            .get_mut(&name)
            .expect("结构体已登记")
            .push((ty, field));
    }
    structs
}

/// 解析头文件中的函数声明：函数名 → 规范化签名（已去掉参数名）。
fn parse_functions(text: &str) -> BTreeMap<String, String> {
    let mut functions = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.ends_with(';')
            || trimmed.starts_with('#')
            || !(trimmed.starts_with("int32_t ") || trimmed.starts_with("uint32_t "))
        {
            continue;
        }
        let Some(open) = trimmed.find('(') else {
            continue;
        };
        let name = trimmed[..open]
            .split_whitespace()
            .last()
            .expect("函数名")
            .to_string();
        if !name.starts_with("nr_") {
            continue;
        }
        functions.insert(name, normalize_signature(trimmed, true));
    }
    functions
}

/// 解析 `#define NR_X <value>`（只处理十进制字面量宏）。
fn parse_macros(text: &str) -> BTreeMap<String, u64> {
    let mut macros = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("#define ") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        if name == "NR_ABI_VERSION" || name.starts_with("NR_") {
            let cleaned = value.trim_end_matches(['u', 'U']);
            if let Ok(number) = cleaned.parse::<u64>() {
                macros.insert(name.to_string(), number);
            }
        }
    }
    macros
}

/// 头文件中函数声明的出现顺序。
fn declared_entry_order(text: &str) -> Vec<String> {
    let mut declared: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("int32_t nr_") || trimmed.starts_with("uint32_t nr_") {
            let name = trimmed
                .split('(')
                .next()
                .expect("函数名")
                .split_whitespace()
                .last()
                .expect("函数名")
                .to_string();
            declared.push(name);
        }
    }
    declared
}

#[test]
fn header_declares_exactly_the_frozen_eighteen_entries_with_matching_signatures() {
    let text = header_text();
    let functions = parse_functions(&text);
    assert_eq!(
        functions.len(),
        ABI_FUNCTIONS.len(),
        "头文件函数数量必须恰为 18，实际 {}: {:?}",
        functions.len(),
        functions.keys().collect::<Vec<_>>()
    );
    for (name, signature) in ABI_FUNCTIONS {
        let found = functions
            .get(name)
            .unwrap_or_else(|| panic!("头文件缺少入口声明: {name}"));
        assert_eq!(
            found,
            &normalize_signature(signature, false),
            "入口 {name} 的签名与 Rust 清单不一致"
        );
    }
    for name in functions.keys() {
        assert!(
            ABI_FUNCTIONS.iter().any(|(declared, _)| declared == name),
            "头文件声明了未冻结的入口: {name}"
        );
    }
}

#[test]
fn header_function_order_matches_frozen_entry_list() {
    let text = header_text();
    assert_eq!(
        declared_entry_order(&text),
        abi::ABI_ENTRY_NAMES.to_vec(),
        "头文件声明顺序必须与冻结入口清单一致"
    );
}

#[test]
fn header_structs_match_the_field_manifest_exactly() {
    let text = header_text();
    let parsed = parse_structs(&text);
    assert_eq!(
        parsed.len(),
        ABI_STRUCTS.len(),
        "头文件结构体数量必须与清单一致；头文件 {} 个，清单 {} 个",
        parsed.len(),
        ABI_STRUCTS.len()
    );
    for (name, fields) in ABI_STRUCTS {
        let found = parsed
            .get(*name)
            .unwrap_or_else(|| panic!("头文件缺少结构体: {name}"));
        assert_eq!(
            found.len(),
            fields.len(),
            "结构体 {name} 字段数量不一致：头文件 {} 个，清单 {} 个",
            found.len(),
            fields.len()
        );
        for (index, (expected_field, expected_type)) in fields.iter().enumerate() {
            let (actual_type, actual_field) = &found[index];
            assert_eq!(
                actual_field,
                expected_field,
                "结构体 {name} 第 {} 个字段名不一致（顺序也必须一致）",
                index + 1
            );
            assert_eq!(
                actual_type,
                &normalize(expected_type),
                "结构体 {name} 的字段 {expected_field} 类型不一致"
            );
        }
    }
}

#[test]
fn header_macros_match_rust_constants() {
    let text = header_text();
    let macros = parse_macros(&text);
    let mut checked = 0usize;
    for (name, expected) in abi::macro_manifest() {
        let found = macros
            .get(name)
            .unwrap_or_else(|| panic!("头文件缺少宏: {name}"));
        assert_eq!(*found, expected, "宏 {name} 与 Rust 常量不一致");
        checked += 1;
    }
    // 18 项状态/常量宏 + 12 项错误码 + 7 项编码 + 9 项拒绝原因 = 46 项。
    assert_eq!(checked, 46, "宏清单必须覆盖全部冻结常量");
    assert_eq!(
        macros.get("NR_ABI_VERSION").copied(),
        Some(u64::from(reading_ffi::ABI_VERSION)),
        "NR_ABI_VERSION 必须等于 Rust ABI_VERSION"
    );
    assert_eq!(
        macros.get("NR_ABI_VERSION").copied(),
        Some(u64::from(reading_ffi::exports::nr_abi_version())),
        "NR_ABI_VERSION 必须等于 nr_abi_version() 的返回值"
    );
}

#[test]
fn header_documents_ownership_and_platform_protocol() {
    // 所有权契约与平台窗口协议必须写在头文件里（冻结要求），而不是只存在于实现注释中。
    let text = header_text();
    for required in [
        "nr_release_buffer",
        "NR_ERR_ENGINE_DESTROYED",
        "NR_SOURCE_KIND_PLATFORM",
        "out_len",
        "NR_MAX_WINDOW_BYTES",
        "NR_ERR_CANCELLED",
        "NR_ERR_CHAPTER_OUT_OF_RANGE",
    ] {
        assert!(
            text.contains(required),
            "头文件必须声明/说明 `{required}`（所有权、生命周期、平台协议与取消契约）"
        );
    }
    let comment_text: String = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('*') || trimmed.starts_with("/*")
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        comment_text.contains("禁止") && comment_text.contains("回调"),
        "头文件必须写明平台回调的禁止项（不得重入 nr_* 入口）"
    );
    assert!(
        comment_text.contains("必须调用一次 nr_release_buffer"),
        "头文件必须写明缓冲区的归还义务"
    );
    assert_eq!(
        abi::ABI_VERSION_MACRO_TEXT,
        "NR_ABI_VERSION",
        "头文件宏名是冻结契约的一部分"
    );
}
