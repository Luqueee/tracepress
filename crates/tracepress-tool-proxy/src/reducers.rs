#[path = "reducers/cargo_diagnostics.rs"]
mod cargo_diagnostics;
#[path = "reducers/cargo_test.rs"]
mod cargo_test;
#[path = "reducers/git_status.rs"]
mod git_status;
#[path = "reducers/ripgrep.rs"]
mod ripgrep;

const fn estimated_tokens(bytes: u64) -> u64 {
    bytes.saturating_add(3) / 4
}

pub use cargo_diagnostics::{
    cargo_check_v1_active, cargo_check_v1_shadow, cargo_check_v2_active, cargo_check_v2_shadow,
    cargo_clippy_v1_shadow,
};
pub use cargo_test::{cargo_test_v1_active, cargo_test_v1_shadow};
pub use git_status::git_status_v1_shadow;
pub use ripgrep::{rg_v1_active, rg_v1_shadow};
