pub mod activity;
pub mod paths;
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
    AgentRun, ContextUsage, HookInput, Origin, ProcessRef, RateLimits, RateWindow, SessionState,
    Snapshot, Status, Transition, WindowRef, aggregate,
};
