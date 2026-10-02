pub mod activity;
pub mod appicon;
pub mod cloud;
pub mod codex;
pub mod hermes;
pub mod paths;
pub mod process;
pub mod repo;
pub mod state;
pub mod store;
pub mod time;
pub mod transcript;
// 祖先のたどり方は OS に依らない関数にしてあり、macOS でも試験できるようにする。
#[cfg(any(windows, test))]
pub mod winfocus;

pub use activity::{Activity, ActivityKind};
pub use paths::MarimoHome;
pub use state::{
    AgentRun, CodexRateLimits, CodexRateWindow, ContextUsage, HookInput, HostProcess, Origin,
    ProcessRef, Provider, RateLimits, RateWindow, SessionState, Snapshot, Status, Transition,
    WindowRef, aggregate,
};
