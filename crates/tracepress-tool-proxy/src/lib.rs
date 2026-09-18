//! Conservative source-side command admission, execution, and output reduction.
#![allow(
    missing_docs,
    reason = "The internal proxy API remains deliberately narrow while its runtime contract settles"
)]

mod admission;
mod codex_hook;
mod execution;
mod model;
mod reducers;

pub use admission::decide;
pub use codex_hook::{codex_pre_tool_use_identity_rewrite, codex_pre_tool_use_rewrite};
pub use execution::execute_passthrough;
pub use model::{
    CargoOutputCandidate, CargoTestShadowCandidate, CommandFamily, FailOpenReason, OutputContract,
    RewriteDecision, SourceExecutionMetadata, SourceOutputCandidate,
};
pub use reducers::{
    cargo_check_v1_active, cargo_check_v1_shadow, cargo_check_v2_active, cargo_check_v2_shadow,
    cargo_clippy_v1_shadow, cargo_test_v1_active, cargo_test_v1_shadow, git_status_v1_active,
    git_status_v1_shadow, rg_v1_active, rg_v1_shadow,
};
