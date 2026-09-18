use crate::model::{CommandFamily, FailOpenReason, OutputContract, RewriteDecision};

/// Allowlist parser: shell composition is never split or reinterpreted.
#[must_use]
pub fn decide(command: &str) -> RewriteDecision {
    if command.contains([
        '|', '>', '<', ';', '&', '$', '`', '(', ')', '\n', '\r', '\'', '"', '\\', '*', '?', '[',
        ']', '{', '}', '~', '!',
    ]) {
        return RewriteDecision::FailOpen(FailOpenReason::ShellSyntax);
    }
    let words: Vec<_> = command.split_ascii_whitespace().collect();
    match words.as_slice() {
        ["cargo", "test", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::CargoTest,
            contract: OutputContract::AgentReadable,
        },
        ["cargo", "check", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::CargoCheck,
            contract: OutputContract::AgentReadable,
        },
        ["cargo", "clippy", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::CargoClippy,
            contract: OutputContract::AgentReadable,
        },
        ["rg", ..] => RewriteDecision::Passthrough {
            family: CommandFamily::Ripgrep,
            contract: OutputContract::AgentReadable,
        },
        ["git", "status"] => RewriteDecision::Passthrough {
            family: CommandFamily::GitStatus,
            contract: OutputContract::AgentReadable,
        },
        _ => RewriteDecision::FailOpen(FailOpenReason::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_only_supported_standalone_commands() {
        for (command, family) in [
            ("cargo test -q", CommandFamily::CargoTest),
            ("cargo check --workspace", CommandFamily::CargoCheck),
            ("cargo clippy --workspace", CommandFamily::CargoClippy),
            ("rg -n fn crates", CommandFamily::Ripgrep),
            ("git status", CommandFamily::GitStatus),
        ] {
            assert!(matches!(
                decide(command),
                RewriteDecision::Passthrough {
                    family: actual,
                    contract: OutputContract::AgentReadable,
                } if actual == family
            ));
        }
        for command in ["git status --short", "cargo build"] {
            assert_eq!(
                decide(command),
                RewriteDecision::FailOpen(FailOpenReason::Unsupported)
            );
        }
    }

    #[test]
    fn shell_composition_fails_open() {
        for value in [
            "cargo test | tail",
            "cargo test > log",
            "cargo test && git status",
            "$(cargo test)",
            "cargo test 'quoted test'",
            "cargo test \\\n+              --all",
        ] {
            assert_eq!(
                decide(value),
                RewriteDecision::FailOpen(FailOpenReason::ShellSyntax)
            );
        }
    }
}
