use std::ops::Sub;
use std::time::Duration;

use hyprlayer_models::goal::{Goal, GoalCheckOutcome, GoalEndReason, GoalPauseReason};

use crate::agent::Harness;

const CHECK_ERROR_LIMIT: u32 = 3;
const STALL_LIMIT: u32 = 3;
const TURN_FAILURE_LIMIT: u32 = 2;
const NEEDS_HUMAN_PREFIX: &str = "NEEDS-HUMAN:";

/// What the runner knows at a boundary; the counters already include the iteration just closed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoundaryInput<'a> {
    pub outcome: GoalCheckOutcome,
    pub iteration: u32,
    pub max_iterations: u32,
    pub harness: Harness,
    pub spend_reached: bool,
    /// Claude's `--max-budget-usd` refused the turn; the cap is sticky, so no later turn would work.
    pub budget_tripped: bool,
    pub check_errors: u32,
    pub stall_count: u32,
    pub turn_failures: u32,
    pub last_text: Option<&'a str>,
    pub pause_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Boundary {
    Met,
    End(GoalEndReason),
    Pause(GoalPauseReason, Option<String>),
    Next,
}

/// First match wins: met, then limits, then failures, then pauses.
pub(crate) fn decide_boundary(input: &BoundaryInput<'_>) -> Boundary {
    if input.outcome == GoalCheckOutcome::Pass {
        return Boundary::Met;
    }
    if input.iteration >= input.max_iterations {
        return Boundary::End(GoalEndReason::IterationLimit);
    }
    if input.spend_reached || input.budget_tripped {
        return Boundary::End(spend_limit(input.harness));
    }
    if input.check_errors >= CHECK_ERROR_LIMIT {
        return Boundary::End(GoalEndReason::ChecksBroken);
    }
    if input.stall_count >= STALL_LIMIT {
        return Boundary::End(GoalEndReason::Stalled);
    }
    if input.turn_failures >= TURN_FAILURE_LIMIT {
        return Boundary::Pause(GoalPauseReason::TurnFailures, None);
    }
    if let Some(question) = input.last_text.and_then(needs_human) {
        return Boundary::Pause(GoalPauseReason::NeedsHuman, Some(question));
    }
    if input.pause_requested {
        return Boundary::Pause(GoalPauseReason::User, None);
    }
    Boundary::Next
}

/// A reading below the baseline is a new process or thread and counts whole; no reading changes nothing.
pub(crate) fn usage_delta<T>(baseline: T, value: Option<T>) -> (T, T)
where
    T: Copy + Default + PartialOrd + Sub<Output = T>,
{
    match value {
        None => (T::default(), baseline),
        Some(value) if value < baseline => (value, value),
        Some(value) => (value - baseline, value),
    }
}

/// Spend has reached its cap, in the unit the harness reports: dollars, or Codex's uncached input plus output tokens.
pub(crate) fn spend_reached(goal: &Goal, harness: Harness) -> bool {
    let limits = &goal.limits;
    match harness {
        Harness::Codex => limits
            .max_tokens
            .is_some_and(|max| goal.spent_tokens >= max),
        Harness::Claude | Harness::Opencode => limits
            .max_cost_usd
            .is_some_and(|max| goal.spent_cost_usd >= max),
    }
}

/// The end reason for spend at its cap, in the unit the harness reports.
pub(crate) fn spend_limit(harness: Harness) -> GoalEndReason {
    match harness {
        Harness::Codex => GoalEndReason::TokenLimit,
        Harness::Claude | Harness::Opencode => GoalEndReason::BudgetLimit,
    }
}

/// Codex's mid-turn cap: the goal's spend before this iteration plus the thread's growth since its send.
pub(crate) fn token_cap_reached(spent: u32, baseline: u32, thread_tokens: u32, max: u32) -> bool {
    let (delta, _) = usage_delta(baseline, Some(thread_tokens));
    spent.saturating_add(delta) >= max
}

/// What two iterations must share for the second to count as no progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StallKey<'a> {
    pub progress_hash: Option<&'a str>,
    pub outcome: GoalCheckOutcome,
    pub reason: Option<&'a str>,
}

/// Counts the current iteration; an unreadable progress hash never repeats, so it cannot stall a goal.
pub(crate) fn stall_count_after(
    prev: Option<&StallKey<'_>>,
    key: &StallKey<'_>,
    prev_count: u32,
) -> u32 {
    match prev {
        Some(prev) if key.progress_hash.is_some() && prev == key => prev_count.saturating_add(1),
        _ => 1,
    }
}

/// The question after a `NEEDS-HUMAN:` prefix on any line of the agent's last text.
pub(crate) fn needs_human(last_text: &str) -> Option<String> {
    last_text
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(NEEDS_HUMAN_PREFIX))
        .map(|question| question.trim().to_string())
}

/// The active-time timer, re-armed from the recorded `active_ms` at every (re)start.
pub(crate) fn remaining_active(max_active_secs: u32, active_ms: u32) -> Duration {
    let cap_ms = u64::from(max_active_secs) * 1000;
    Duration::from_millis(cap_ms.saturating_sub(u64::from(active_ms)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> BoundaryInput<'static> {
        BoundaryInput {
            outcome: GoalCheckOutcome::Fail,
            iteration: 1,
            max_iterations: 10,
            harness: Harness::Claude,
            spend_reached: false,
            budget_tripped: false,
            check_errors: 0,
            stall_count: 0,
            turn_failures: 0,
            last_text: None,
            pause_requested: false,
        }
    }

    #[test]
    fn a_limit_outranks_a_failure_and_a_failure_outranks_a_pause() {
        let everything = BoundaryInput {
            iteration: 10,
            spend_reached: true,
            budget_tripped: true,
            check_errors: 3,
            stall_count: 3,
            turn_failures: 2,
            last_text: Some("working on it\nNEEDS-HUMAN: which database?\n"),
            pause_requested: true,
            ..quiet()
        };
        let passing = BoundaryInput {
            outcome: GoalCheckOutcome::Pass,
            ..everything
        };
        assert_eq!(
            decide_boundary(&passing),
            Boundary::Met,
            "met is matched first"
        );
        assert_eq!(
            decide_boundary(&everything),
            Boundary::End(GoalEndReason::IterationLimit)
        );
        // A cap lowered below the current iteration still ends the goal.
        let lowered = BoundaryInput {
            max_iterations: 4,
            iteration: 7,
            ..everything
        };
        assert_eq!(
            decide_boundary(&lowered),
            Boundary::End(GoalEndReason::IterationLimit)
        );

        let under_cap = BoundaryInput {
            iteration: 5,
            ..everything
        };
        assert_eq!(
            decide_boundary(&under_cap),
            Boundary::End(GoalEndReason::BudgetLimit)
        );
        let on_codex = BoundaryInput {
            harness: Harness::Codex,
            budget_tripped: false,
            ..under_cap
        };
        assert_eq!(
            decide_boundary(&on_codex),
            Boundary::End(GoalEndReason::TokenLimit)
        );
        // The backstop ends the goal even when the counted spend is under the cap.
        let tripped = BoundaryInput {
            spend_reached: false,
            ..under_cap
        };
        assert_eq!(
            decide_boundary(&tripped),
            Boundary::End(GoalEndReason::BudgetLimit)
        );
        let within_budget = BoundaryInput {
            budget_tripped: false,
            ..tripped
        };
        assert_eq!(
            decide_boundary(&within_budget),
            Boundary::End(GoalEndReason::ChecksBroken)
        );
        let checks_fine = BoundaryInput {
            check_errors: 0,
            ..within_budget
        };
        assert_eq!(
            decide_boundary(&checks_fine),
            Boundary::End(GoalEndReason::Stalled)
        );
        let moving = BoundaryInput {
            stall_count: 0,
            ..checks_fine
        };
        assert_eq!(
            decide_boundary(&moving),
            Boundary::Pause(GoalPauseReason::TurnFailures, None)
        );
        let turns_fine = BoundaryInput {
            turn_failures: 0,
            ..moving
        };
        assert_eq!(
            decide_boundary(&turns_fine),
            Boundary::Pause(GoalPauseReason::NeedsHuman, Some("which database?".into()))
        );
        let no_question = BoundaryInput {
            last_text: Some("all done here"),
            ..turns_fine
        };
        assert_eq!(
            decide_boundary(&no_question),
            Boundary::Pause(GoalPauseReason::User, None)
        );
        let no_pause = BoundaryInput {
            pause_requested: false,
            ..no_question
        };
        assert_eq!(decide_boundary(&no_pause), Boundary::Next);
    }

    #[test]
    fn cost_delta_resets_on_a_new_process() {
        let (first, baseline) = usage_delta(0.0, Some(0.4));
        let (second, baseline) = usage_delta(baseline, Some(0.8));
        // A new process restarts its cumulative cost, so its first reading is below the baseline.
        let (third, baseline) = usage_delta(baseline, Some(0.3));
        assert_eq!([first, second, third], [0.4, 0.4, 0.3]);
        assert_eq!(baseline, 0.3);
    }

    #[test]
    fn token_totals_continue_across_a_resume() {
        // Probe R: `thread/resume` reports the thread's total again, and the baseline is kept.
        let (first, baseline) = usage_delta(0, Some(17_812));
        let (resumed, baseline) = usage_delta(baseline, Some(17_812));
        let (next, _) = usage_delta(baseline, Some(39_653));
        assert_eq!([first, resumed, next], [17_812, 0, 21_841]);
    }

    #[test]
    fn the_token_cap_trips_on_the_usage_that_crosses_it() {
        let (spent, baseline, max) = (1_980_000, 17_812, 2_000_000);
        assert!(!token_cap_reached(spent, baseline, 37_811, max));
        assert!(token_cap_reached(spent, baseline, 37_812, max));
    }

    #[test]
    fn the_deadline_resumes_from_recorded_active_time() {
        assert_eq!(remaining_active(60, 0), Duration::from_secs(60));
        assert_eq!(remaining_active(60, 15_000), Duration::from_secs(45));
        assert_eq!(remaining_active(60, 59_999), Duration::from_millis(1));
        assert_eq!(remaining_active(60, 60_000), Duration::ZERO);
        assert_eq!(remaining_active(60, 90_000), Duration::ZERO);
    }
}
