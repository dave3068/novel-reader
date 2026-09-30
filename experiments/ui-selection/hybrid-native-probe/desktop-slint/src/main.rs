//! 桌面端探针入口。
//!
//! 用法：
//! ```text
//! source /opt/dev-tools/slint-libs/slint-env.sh
//! cargo run --release -- --variant femtovg      # 交互式窗口
//! cargo run --release -- --pagination-self-check --variant femtovg
//! ```

fn main() {
    std::process::exit(desktop_slint_probe::run());
}
