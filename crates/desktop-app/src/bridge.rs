//! 后台结果切回 Slint UI 线程的机制。
//!
//! 依据 `desktop/design.md` §页面状态 与 `design.md` §4（每个异步请求带 request id、
//! 每次排版带 layout id、过期结果必须丢弃）：
//!
//! - 后台线程只做「读数据」这类阻塞工作，产出 [`BackgroundResult`]；
//! - 结果由 [`UiBridge::deliver`] 交给 [`UiExecutor`]，生产环境是
//!   `slint::invoke_from_event_loop`（公开 API，不使用私有 Slint 文本 API），
//!   测试环境是 [`DirectExecutor`]；
//! - 状态变更与过期判定统一在 UI 线程的 [`UiBridge::handle_result`] 里完成，
//!   因此 Slint 组件不需要任何锁，也不会被后台线程直接触碰。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::app_state::{AppState, ApplyOutcome, BackgroundResult, ShelfRequest, UiSnapshot};

/// 「回到 UI 线程」的执行器抽象。
pub trait UiExecutor: Send + Sync + 'static {
    /// 在 UI 线程上执行任务。
    fn execute(&self, task: Box<dyn FnOnce() + Send>);
}

/// 生产执行器：`slint::invoke_from_event_loop`（Slint 公开 API）。
#[derive(Debug, Default)]
pub struct SlintEventLoopExecutor;

impl UiExecutor for SlintEventLoopExecutor {
    fn execute(&self, task: Box<dyn FnOnce() + Send>) {
        // 事件循环尚未启动/已退出时返回 Err：此处只丢弃（进程通常已在退出路径），
        // 不使用 `unwrap` 以免在退出竞态里 panic。
        let _ = slint::invoke_from_event_loop(task);
    }
}

/// 测试/同线程执行器：立刻在当前线程执行（用于确定性地验证状态机）。
#[derive(Debug, Default)]
pub struct DirectExecutor;

impl UiExecutor for DirectExecutor {
    fn execute(&self, task: Box<dyn FnOnce() + Send>) {
        task();
    }
}

/// 后台结果投递桥。
pub struct UiBridge {
    state: Arc<Mutex<AppState>>,
    executor: Arc<dyn UiExecutor>,
    on_snapshot: Arc<dyn Fn(UiSnapshot) + Send + Sync>,
    delivered: AtomicU64,
    projected: AtomicU64,
}

impl UiBridge {
    /// 构造桥：`state` 是唯一状态源，`on_snapshot` 在 UI 线程上把投影写入 Slint。
    pub fn new(
        state: Arc<Mutex<AppState>>,
        executor: Arc<dyn UiExecutor>,
        on_snapshot: Arc<dyn Fn(UiSnapshot) + Send + Sync>,
    ) -> Self {
        Self {
            state,
            executor,
            on_snapshot,
            delivered: AtomicU64::new(0),
            projected: AtomicU64::new(0),
        }
    }

    /// 后台线程入口：把结果交回 UI 线程。
    pub fn deliver(&self, result: BackgroundResult) {
        let state = Arc::clone(&self.state);
        let on_snapshot = Arc::clone(&self.on_snapshot);
        self.delivered.fetch_add(1, Ordering::Relaxed);
        self.executor.execute(Box::new(move || {
            let snapshot = {
                let mut guard = match state.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if guard.apply_result(result) == ApplyOutcome::Applied {
                    Some(guard.project())
                } else {
                    // 过期结果：不刷新页面（旧结果不得覆盖当前页面）。
                    None
                }
            };
            if let Some(snapshot) = snapshot {
                on_snapshot(snapshot);
            }
        }));
    }

    /// 立即把当前状态投影一次（UI 线程用；启动与本地交互后调用）。
    pub fn project_now(&self) {
        let snapshot = {
            let guard = match self.state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.project()
        };
        self.projected.fetch_add(1, Ordering::Relaxed);
        (self.on_snapshot)(snapshot);
    }

    /// 已投递 / 已投影计数（自检与测试用）。
    pub fn counters(&self) -> (u64, u64) {
        (
            self.delivered.load(Ordering::Relaxed),
            self.projected.load(Ordering::Relaxed),
        )
    }
}

/// 后台任务执行函数：由编排层注入真实实现（DT1 是读书架，DT2 起是导入/索引）。
pub type JobRunner = Box<dyn Fn(ShelfRequest) -> BackgroundResult + Send + 'static>;

/// 启动后台工作线程：只负责阻塞读取，结果一律经 `bridge` 回到 UI 线程。
pub fn spawn_worker(
    requests: Receiver<ShelfRequest>,
    bridge: Arc<UiBridge>,
    runner: JobRunner,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(request) = requests.recv() {
            let result = runner(request);
            bridge.deliver(result);
        }
    })
}

/// 后台请求通道（生产者留在 UI 线程）。
pub struct JobChannel {
    tx: Sender<ShelfRequest>,
}

impl JobChannel {
    /// 创建通道并启动工作线程。
    pub fn start(bridge: Arc<UiBridge>, runner: JobRunner) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<ShelfRequest>();
        let handle = spawn_worker(rx, bridge, runner);
        (Self { tx }, handle)
    }

    /// 提交一个后台请求；通道关闭时返回 `false`。
    pub fn submit(&self, request: ShelfRequest) -> bool {
        self.tx.send(request).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::{BackgroundPayload, Screen, SettingsView, ShelfRow};
    use std::path::PathBuf;

    /// 测试夹具三元组：桥接器、共享应用状态、快照接收端。
    type BridgeFixture = (
        Arc<UiBridge>,
        Arc<Mutex<AppState>>,
        Arc<Mutex<Vec<UiSnapshot>>>,
    );

    fn bridge_with_state() -> BridgeFixture {
        let state = Arc::new(Mutex::new(AppState::new("test")));
        let seen: Arc<Mutex<Vec<UiSnapshot>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let bridge = Arc::new(UiBridge::new(
            Arc::clone(&state),
            Arc::new(DirectExecutor),
            Arc::new(move |snapshot: UiSnapshot| {
                sink.lock().unwrap().push(snapshot);
            }),
        ));
        (bridge, state, seen)
    }

    #[test]
    fn background_result_reaches_ui_thread_and_projects() {
        let (bridge, _, seen) = bridge_with_state();
        let (channel, worker) = JobChannel::start(
            Arc::clone(&bridge),
            Box::new(|request: ShelfRequest| BackgroundResult {
                request_id: request.request_id,
                layout_id: request.layout_id,
                payload: BackgroundPayload::Shelf {
                    rows: vec![ShelfRow {
                        id: "42".to_string(),
                        title: "真实书".to_string(),
                        initial: "真".to_string(),
                        meta: "/tmp/42.txt · 1.0 KB".to_string(),
                        status: "ok".to_string(),
                        status_tone: 2,
                        progress: "第 1 章".to_string(),
                        percent: 0,
                        recent: String::new(),
                        needs_relink: false,
                        can_open: true,
                        recent_epoch: 0,
                        created_epoch: 0,
                    }],
                    total_count: 1,
                    removed_count: 0,
                    settings: Box::new(SettingsView::default()),
                    status_text: "书架已加载".to_string(),
                    categories: Vec::new(),
                    filtered_count: 1,
                    scope_label: String::new(),
                },
            }),
        );

        let request = {
            let mut state = bridge.state.lock().unwrap();
            state.next_shelf_request(PathBuf::from("/tmp/library.db"))
        };
        assert!(channel.submit(request));
        drop(channel);
        worker.join().expect("后台线程应正常结束");

        let snapshots = seen.lock().unwrap();
        assert_eq!(
            snapshots.len(),
            1,
            "后台结果必须经执行器回到 UI 线程并投影一次"
        );
        assert_eq!(snapshots[0].shelf.rows.len(), 1);
        assert_eq!(snapshots[0].shelf.rows[0].id, "42");
        assert_eq!(snapshots[0].status_text, "书架已加载");
        assert_eq!(bridge.counters().0, 1);
    }

    #[test]
    fn stale_background_result_is_not_projected() {
        let (bridge, state, seen) = bridge_with_state();
        let request = {
            let mut guard = state.lock().unwrap();
            let request = guard.next_shelf_request(PathBuf::from("/tmp/library.db"));
            guard.invalidate_layout();
            request
        };
        bridge.deliver(BackgroundResult {
            request_id: request.request_id,
            layout_id: request.layout_id,
            payload: BackgroundPayload::Failure {
                message: "旧布局结果".to_string(),
            },
        });
        assert!(seen.lock().unwrap().is_empty(), "过期结果不得刷新页面");
        assert_eq!(state.lock().unwrap().screen(), Screen::Shelf);
        assert_eq!(bridge.counters(), (1, 0));
    }
}
