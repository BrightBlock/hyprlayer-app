//! Per-harness capability layer.
//!
//! Claude, codex, and opencode express "what may this stage's agent do" with
//! different primitives: Claude takes per-command `allowed`/`disallowed` tool
//! lists, codex takes a sandbox + approval [`CodexPosture`], and opencode drives
//! its own permission model over HTTP. This module is the single place that maps
//! a `(harness, stage)` pair onto the primitive its driver understands, so the
//! launch path stays a thin consumer instead of re-deriving policy per arm.
//!
//! See ADR-0007 for the divergence surface and the guardrails codex cannot
//! reproduce (per-command deny, the Ship publishing agent).

use crate::agent::Harness;
use crate::claude::PermissionMode;
use crate::codex::CodexPosture;
use crate::pipeline::{agent_for, policy_for, SessionAgent, Stage};

/// A stage's guardrails, lowered onto the primitive the chosen harness enforces.
pub enum LaunchPolicy {
    Claude {
        permission_mode: PermissionMode,
        allowed_tools: Vec<String>,
        disallowed_tools: Vec<String>,
        agent: Option<SessionAgent>,
    },
    Codex {
        permission_mode: PermissionMode,
        posture: CodexPosture,
    },
    Opencode {
        permission_mode: PermissionMode,
    },
}

/// Resolve the launch policy for a stage on a given harness. `requested` is the
/// user's explicit permission-mode override (from the UI toggle); when absent the
/// stage's own default applies.
pub fn launch_policy(
    harness: Harness,
    stage: Stage,
    requested: Option<PermissionMode>,
) -> LaunchPolicy {
    let base = policy_for(stage);
    let permission_mode = requested.unwrap_or(base.permission_mode);
    match harness {
        Harness::Claude => LaunchPolicy::Claude {
            permission_mode,
            allowed_tools: base.allowed_tools,
            disallowed_tools: base.disallowed_tools,
            agent: agent_for(stage),
        },
        Harness::Codex => LaunchPolicy::Codex {
            permission_mode,
            posture: codex_posture(stage, permission_mode),
        },
        Harness::Opencode => LaunchPolicy::Opencode { permission_mode },
    }
}

/// Lower a stage's intent onto codex's sandbox + approval posture.
///
/// The base posture comes from the permission mode, but some stages carry an
/// intent the mode alone cannot express — most sharply Validate, a read-only
/// audit that must never mutate the tree regardless of the mode the session runs
/// under. Codex has no per-command deny, so this is the only lever available.
pub fn codex_posture(stage: Stage, mode: PermissionMode) -> CodexPosture {
    let mut posture = CodexPosture::from_permission_mode(mode);
    match stage {
        // A read-only audit: never let the model mutate the tree, whatever mode
        // the session runs under.
        Stage::Validate => {
            posture.sandbox = "read-only";
            posture.approval_policy = "on-request";
            posture.network_access = false;
        }
        // Commit's contract is "commit, but never push without a yes." Claude
        // enforces it with a mode-independent deny of `git push`. Codex has no
        // per-command deny, so the closest equivalent is to require approval
        // before *any* command runs — a push included — even under AcceptEdits or
        // Bypass, which would otherwise auto-run it. The push still goes through
        // once the user approves; it just cannot happen silently.
        Stage::Commit => {
            posture.approval_policy = "untrusted";
            posture.sandbox = "workspace-write";
            posture.network_access = true;
        }
        // Research explores: on codex its collaboration turn runs a long tail of
        // read-only commands, and `untrusted` raises an approval card for every
        // one of them. Soften the approval only in the mode that would have
        // produced `untrusted`, so a mode the user chose deliberately is left
        // alone. The sandbox stays writable — the second turn writes the
        // document — and research legitimately fetches.
        Stage::Research if mode == PermissionMode::Default => {
            posture.approval_policy = "on-request";
        }
        _ => {}
    }
    posture
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_policy_matches_the_stage_tool_lists() {
        let base = policy_for(Stage::Commit);
        match launch_policy(Harness::Claude, Stage::Commit, None) {
            LaunchPolicy::Claude {
                permission_mode,
                allowed_tools,
                disallowed_tools,
                agent,
            } => {
                assert_eq!(permission_mode, base.permission_mode);
                assert_eq!(allowed_tools, base.allowed_tools);
                assert_eq!(disallowed_tools, base.disallowed_tools);
                assert_eq!(agent, agent_for(Stage::Commit));
            }
            _ => panic!("claude harness must yield a Claude policy"),
        }
    }

    #[test]
    fn ship_carries_the_publishing_agent_on_claude_only() {
        match launch_policy(Harness::Claude, Stage::Ship, None) {
            LaunchPolicy::Claude { agent, .. } => assert!(agent.is_some()),
            _ => panic!("expected Claude policy"),
        }
        match launch_policy(Harness::Codex, Stage::Ship, None) {
            LaunchPolicy::Codex { .. } => {}
            _ => panic!("expected Codex policy"),
        }
    }

    #[test]
    fn requested_permission_mode_overrides_the_stage_default() {
        let base = policy_for(Stage::Commit).permission_mode;
        assert_ne!(base, PermissionMode::BypassPermissions);
        match launch_policy(
            Harness::Claude,
            Stage::Commit,
            Some(PermissionMode::BypassPermissions),
        ) {
            LaunchPolicy::Claude {
                permission_mode, ..
            } => assert_eq!(permission_mode, PermissionMode::BypassPermissions),
            _ => panic!("expected Claude policy"),
        }
    }

    #[test]
    fn codex_validate_is_read_only_regardless_of_mode() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
        ] {
            let posture = codex_posture(Stage::Validate, mode);
            assert_eq!(posture.sandbox, "read-only");
            assert!(!posture.network_access);
        }
    }

    #[test]
    fn codex_unconstrained_stages_follow_the_permission_mode() {
        assert_eq!(
            codex_posture(Stage::Implement, PermissionMode::Default),
            CodexPosture::from_permission_mode(PermissionMode::Default)
        );
        assert_eq!(
            codex_posture(Stage::Implement, PermissionMode::AcceptEdits),
            CodexPosture::from_permission_mode(PermissionMode::AcceptEdits)
        );
    }

    #[test]
    fn codex_commit_always_requires_approval_before_a_push() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
        ] {
            let posture = codex_posture(Stage::Commit, mode);
            assert_eq!(
                posture.approval_policy, "untrusted",
                "commit must prompt before any command on {mode:?}"
            );
            assert_eq!(posture.sandbox, "workspace-write");
            assert!(posture.network_access, "an approved push needs the network");
        }
        assert_ne!(
            CodexPosture::from_permission_mode(PermissionMode::AcceptEdits).approval_policy,
            "untrusted",
            "the guarantee must be the stage's doing, not the mode's",
        );
    }

    #[test]
    fn codex_research_softens_approval_only_where_it_was_untrusted() {
        let posture = codex_posture(Stage::Research, PermissionMode::Default);
        assert_eq!(
            posture.approval_policy, "on-request",
            "an exploration turn cannot raise a card per read-only command"
        );
        assert_eq!(
            posture.sandbox, "workspace-write",
            "the second turn still has to write the research document"
        );
        assert!(posture.network_access, "research legitimately fetches");
        assert_eq!(
            CodexPosture::from_permission_mode(PermissionMode::Default).approval_policy,
            "untrusted",
            "the softening exists because Default is the untrusted mode",
        );
        for mode in [
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
        ] {
            assert_eq!(
                codex_posture(Stage::Research, mode),
                CodexPosture::from_permission_mode(mode),
                "a deliberately chosen {mode:?} is left alone"
            );
        }
    }

    #[test]
    fn codex_workspace_write_keeps_network_for_push_stages() {
        let posture = codex_posture(Stage::Ship, PermissionMode::Default);
        assert_eq!(posture.sandbox, "workspace-write");
        assert!(posture.network_access);
    }
}
