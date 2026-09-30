//! C 侧 ABI 消费者测试（RC7 跨语言契约的真实闸门）。
//!
//! 本测试用系统 C 编译器编译 `crates/reading-ffi/tests/abi_smoke.c`（C11 + `-Werror`），
//! 链接真实的 `libreading_ffi.so` 并执行它。这样「头文件在 C 里能编过」与
//! 「冻结入口在 C 里能跑通」就不再只是人工步骤，而是 `cargo test -p reading-ffi --release`
//! 的一部分。
//!
//! 前置条件与降级口径（**不得静默通过**）：
//!   - `cargo test` 只构建 rlib 测试产物，**不会**产出 cdylib；因此本测试要求先跑一次
//!     `cargo build -p reading-ffi --release`。
//!   - 缺少 `libreading_ffi.so`、缺少 `cc`、或源文件不存在时，打印显式 `SKIP` 原因并返回，
//!     绝不把「没跑」算成「通过」。
//!   - 一旦真的编译/执行，任何非 0 退出码都会让本测试失败，并附带完整的 C 侧输出。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 测试二进制所在目录的上一级，即 cargo 的 profile 目录（`target/<profile>`）。
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe 可用");
    exe.parent() // deps/
        .and_then(Path::parent) // target/<profile>
        .expect("测试二进制位于 target/<profile>/deps/ 下")
        .to_path_buf()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/reading-ffi 的上级两级是仓库根")
        .to_path_buf()
}

fn command_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// 若前置条件不足，打印显式 SKIP 并返回 `None`。
fn prerequisites() -> Option<(PathBuf, PathBuf)> {
    let profile = profile_dir();
    let shared_lib = profile.join("libreading_ffi.so");
    if !shared_lib.exists() {
        println!(
            "SKIP C ABI 消费者测试：找不到 {}（请先执行 cargo build -p reading-ffi --release）",
            shared_lib.display()
        );
        return None;
    }
    let source = repo_root().join("crates/reading-ffi/tests/abi_smoke.c");
    if !source.exists() {
        println!("SKIP C ABI 消费者测试：找不到源码 {}", source.display());
        return None;
    }
    if !command_available("cc") {
        println!("SKIP C ABI 消费者测试：系统缺少 C 编译器 `cc`");
        return None;
    }
    Some((profile, source))
}

#[test]
fn c_consumer_of_the_frozen_header_links_and_runs_against_the_real_library() {
    let Some((profile, source)) = prerequisites() else {
        return;
    };
    let repo = repo_root();
    let include = repo.join("include");
    assert!(
        include.join("novel_reader.h").exists(),
        "头文件必须存在于 {}",
        include.join("novel_reader.h").display()
    );

    let out_dir = std::env::temp_dir().join(format!("nr-abi-c-consumer-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).expect("创建临时目录");
    let binary = out_dir.join("nr_abi_smoke");

    // ---- 编译：-std=c11 -Wall -Wextra -Werror 让头文件的任何告警都变成失败 ----
    let compile = Command::new("cc")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-I")
        .arg(&include)
        .arg(&source)
        .arg("-L")
        .arg(&profile)
        .arg("-lreading_ffi")
        .arg("-lpthread")
        .arg("-ldl")
        .arg("-lm")
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("调用 cc");
    let stderr = String::from_utf8_lossy(&compile.stderr);
    assert!(
        compile.status.success(),
        "C 头文件必须能在 -std=c11 -Wall -Wextra -Werror 下编译：\n{stderr}"
    );
    assert!(binary.exists(), "编译成功必须产出可执行文件");

    // ---- 运行：.so 与测试二进制同目录，用 LD_LIBRARY_PATH 指向 profile 目录 ----
    let run = Command::new(&binary)
        .env("LD_LIBRARY_PATH", &profile)
        .output()
        .expect("执行 C 冒烟程序");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let run_stderr = String::from_utf8_lossy(&run.stderr);
    println!("--- C 侧输出开始 ---\n{stdout}--- C 侧输出结束 ---");
    assert!(
        run.status.success(),
        "C 侧断言必须全部通过（退出码 {:?}）：\n{stdout}\n{run_stderr}",
        run.status.code()
    );
    // 防空检查：必须真的打印出手工断言的 PASS 汇总行，否则可能是空跑。
    let passed = stdout.matches("PASS").count();
    assert!(
        passed >= 20 && stdout.contains("PASS（0 项失败）"),
        "C 侧必须报告足量 PASS 且零失败（实际 PASS 行 {passed}）"
    );

    let _ = std::fs::remove_dir_all(&out_dir);
}
