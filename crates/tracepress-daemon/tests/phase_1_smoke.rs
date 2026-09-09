//! Ensures the workspace Phase 1 smoke target builds the real daemon binary.

#[test]
fn daemon_binary_is_available_to_workspace_smoke() {
    assert!(!env!("CARGO_BIN_EXE_tracepressd").is_empty());
}
