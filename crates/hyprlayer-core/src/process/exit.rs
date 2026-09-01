use hyprlayer_models::session::ExitOutcome;

const EXTERNALLY_TERMINATED: i32 = 0x4001_0004;

#[cfg(unix)]
fn is_stop_signal(signal: i32) -> bool {
    matches!(
        signal,
        libc::SIGHUP | libc::SIGINT | libc::SIGTERM | libc::SIGKILL
    )
}

#[cfg(not(unix))]
fn is_stop_signal(_signal: i32) -> bool {
    false
}

fn is_stop_exit_code(code: i32) -> bool {
    matches!(code, 129 | 130 | 137 | 143)
}

pub fn outcome_from_unclean_exit(code: Option<i32>, signal: Option<i32>) -> ExitOutcome {
    if let Some(signal) = signal {
        return if is_stop_signal(signal) {
            ExitOutcome::Killed
        } else {
            ExitOutcome::Crashed
        };
    }
    match code {
        Some(EXTERNALLY_TERMINATED) => ExitOutcome::Killed,
        Some(code) if is_stop_exit_code(code) => ExitOutcome::Killed,
        _ => ExitOutcome::Crashed,
    }
}

pub fn outcome_from_status(status: &std::process::ExitStatus) -> ExitOutcome {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(status);
    #[cfg(not(unix))]
    let signal = None;
    outcome_from_unclean_exit(status.code(), signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_external_termination_is_a_kill_not_a_crash() {
        assert_eq!(
            outcome_from_unclean_exit(Some(1_073_807_364), None),
            ExitOutcome::Killed
        );
        assert_eq!(
            outcome_from_unclean_exit(Some(1), None),
            ExitOutcome::Crashed
        );
        assert_eq!(outcome_from_unclean_exit(None, None), ExitOutcome::Crashed);
    }

    #[test]
    fn unix_stop_signals_are_kills_but_faults_are_crashes() {
        for signal in [1, 2, 15, 9] {
            assert_eq!(
                outcome_from_unclean_exit(None, Some(signal)),
                ExitOutcome::Killed,
                "signal {signal}"
            );
        }
        for signal in [6, 11] {
            assert_eq!(
                outcome_from_unclean_exit(None, Some(signal)),
                ExitOutcome::Crashed,
                "signal {signal}"
            );
        }
    }

    #[test]
    fn stop_exit_codes_are_kills_not_crashes() {
        for code in [129, 130, 137, 143] {
            assert_eq!(
                outcome_from_unclean_exit(Some(code), None),
                ExitOutcome::Killed,
                "exit {code}"
            );
        }
        for code in [1, 2, 127, 128] {
            assert_eq!(
                outcome_from_unclean_exit(Some(code), None),
                ExitOutcome::Crashed,
                "exit {code}"
            );
        }
    }
}
