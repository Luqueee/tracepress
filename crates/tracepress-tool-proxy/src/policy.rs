use crate::model::CommandFamily;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourcePolicyMode {
    Shadow,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourcePolicyDecision {
    ShadowOnly,
    AcceptedOptIn,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SourceReducerPolicy {
    pub reducer_id: &'static str,
    pub command_family: CommandFamily,
    pub mode: SourcePolicyMode,
    pub decision: SourcePolicyDecision,
}

const SOURCE_REDUCER_POLICIES: [SourceReducerPolicy; 11] = [
    SourceReducerPolicy {
        reducer_id: "cargo_test_v1_shadow",
        command_family: CommandFamily::CargoTest,
        mode: SourcePolicyMode::Shadow,
        decision: SourcePolicyDecision::ShadowOnly,
    },
    SourceReducerPolicy {
        reducer_id: "cargo_test_v1_active",
        command_family: CommandFamily::CargoTest,
        mode: SourcePolicyMode::Active,
        decision: SourcePolicyDecision::AcceptedOptIn,
    },
    SourceReducerPolicy {
        reducer_id: "cargo_check_v1_shadow",
        command_family: CommandFamily::CargoCheck,
        mode: SourcePolicyMode::Shadow,
        decision: SourcePolicyDecision::ShadowOnly,
    },
    SourceReducerPolicy {
        reducer_id: "cargo_check_v1_active",
        command_family: CommandFamily::CargoCheck,
        mode: SourcePolicyMode::Active,
        decision: SourcePolicyDecision::Rejected,
    },
    SourceReducerPolicy {
        reducer_id: "cargo_check_v2_shadow",
        command_family: CommandFamily::CargoCheck,
        mode: SourcePolicyMode::Shadow,
        decision: SourcePolicyDecision::ShadowOnly,
    },
    SourceReducerPolicy {
        reducer_id: "cargo_check_v2_active",
        command_family: CommandFamily::CargoCheck,
        mode: SourcePolicyMode::Active,
        decision: SourcePolicyDecision::AcceptedOptIn,
    },
    SourceReducerPolicy {
        reducer_id: "cargo_clippy_v1_shadow",
        command_family: CommandFamily::CargoClippy,
        mode: SourcePolicyMode::Shadow,
        decision: SourcePolicyDecision::Rejected,
    },
    SourceReducerPolicy {
        reducer_id: "rg_v1_shadow",
        command_family: CommandFamily::Ripgrep,
        mode: SourcePolicyMode::Shadow,
        decision: SourcePolicyDecision::ShadowOnly,
    },
    SourceReducerPolicy {
        reducer_id: "rg_v1_active",
        command_family: CommandFamily::Ripgrep,
        mode: SourcePolicyMode::Active,
        decision: SourcePolicyDecision::AcceptedOptIn,
    },
    SourceReducerPolicy {
        reducer_id: "git_status_v1_shadow",
        command_family: CommandFamily::GitStatus,
        mode: SourcePolicyMode::Shadow,
        decision: SourcePolicyDecision::ShadowOnly,
    },
    SourceReducerPolicy {
        reducer_id: "git_status_v1_active",
        command_family: CommandFamily::GitStatus,
        mode: SourcePolicyMode::Active,
        decision: SourcePolicyDecision::Rejected,
    },
];

#[must_use]
pub const fn source_reducer_policies() -> &'static [SourceReducerPolicy] {
    &SOURCE_REDUCER_POLICIES
}

#[must_use]
pub fn source_reducer_policy(reducer_id: &str) -> Option<SourceReducerPolicy> {
    SOURCE_REDUCER_POLICIES
        .iter()
        .copied()
        .find(|policy| policy.reducer_id == reducer_id)
}

#[must_use]
pub fn active_source_policy_allowed(reducer_id: &str, family: CommandFamily) -> bool {
    source_reducer_policy(reducer_id).is_some_and(|policy| {
        policy.command_family == family
            && policy.mode == SourcePolicyMode::Active
            && policy.decision == SourcePolicyDecision::AcceptedOptIn
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_measured_positive_active_policies_are_authorized() {
        for (reducer, family) in [
            ("cargo_test_v1_active", CommandFamily::CargoTest),
            ("cargo_check_v2_active", CommandFamily::CargoCheck),
            ("rg_v1_active", CommandFamily::Ripgrep),
        ] {
            assert!(active_source_policy_allowed(reducer, family), "{reducer}");
        }

        for (reducer, family) in [
            ("cargo_check_v1_active", CommandFamily::CargoCheck),
            ("git_status_v1_active", CommandFamily::GitStatus),
            ("cargo_clippy_v1_shadow", CommandFamily::CargoClippy),
        ] {
            assert!(!active_source_policy_allowed(reducer, family), "{reducer}");
        }

        let accepted: std::collections::BTreeSet<_> = source_reducer_policies()
            .iter()
            .filter(|policy| policy.decision == SourcePolicyDecision::AcceptedOptIn)
            .map(|policy| policy.reducer_id)
            .collect();
        assert_eq!(
            accepted,
            std::collections::BTreeSet::from([
                "cargo_check_v2_active",
                "cargo_test_v1_active",
                "rg_v1_active",
            ])
        );
    }

    #[test]
    fn registry_preserves_rejected_and_shadow_evidence() {
        assert_eq!(
            source_reducer_policy("git_status_v1_active").map(|policy| policy.decision),
            Some(SourcePolicyDecision::Rejected)
        );
        assert_eq!(
            source_reducer_policy("cargo_clippy_v1_shadow").map(|policy| policy.decision),
            Some(SourcePolicyDecision::Rejected)
        );
        assert_eq!(
            source_reducer_policy("git_status_v1_shadow").map(|policy| policy.decision),
            Some(SourcePolicyDecision::ShadowOnly)
        );
        assert_eq!(source_reducer_policy("unknown"), None);
    }
}
