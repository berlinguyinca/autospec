pub mod contract;
pub mod review_dispatch;

pub use contract::{render_handoff_prompt, AgentResult, AgentTask, SafeModePolicy};
pub use review_dispatch::{build_review_argv, HarnessKind, ReviewDispatchOutcome};
