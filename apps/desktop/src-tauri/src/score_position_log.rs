//! Per-session score-position breadcrumbs (#354).
//!
//! Five VA runs reported "no visual cursor" while the only backend
//! breadcrumb — a process-wide `std::sync::Once` "first score-position
//! emitted" — could not fire twice by construction, so every log read as
//! "one emission then silence" whether or not emissions continued. The
//! Rust worker loop is proven by `audio_pipeline`'s emission-contract
//! tests, and the webview path is proven by the Playwright cursor spec in
//! Chromium AND WebKit; what's left is the real app's IPC boundary, which
//! only a tester's log can observe. This module makes that log decisive:
//! per-session state, measure-change lines, and a heartbeat that proves
//! emissions CONTINUE — plus explicit suppression/resume lines when the
//! exploration gate rests the cursor.

/// Emit a heartbeat crumb every this-many position emissions (~10 s at
/// the 10 Hz emit cadence): frequent enough that a tester's minute-long
/// session shows several, rare enough to never flood the log file.
const HEARTBEAT_EVERY: u64 = 100;

/// Longest webview-supplied breadcrumb we will put in the log, in bytes.
/// The webview is inside the trust boundary, but a runaway string (a
/// stringified exception, say) must not bloat the unrotated log file.
const FRONTEND_BREADCRUMB_MAX_BYTES: usize = 300;

/// One log-worthy observation about the position-emission stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionBreadcrumb {
    /// The session's first position left the backend.
    First { measure: usize },
    /// The reported measure changed (the cursor should visibly move).
    MeasureChanged {
        from: usize,
        to: usize,
        emitted: u64,
    },
    /// Emissions are still flowing — the "not one-then-silence" proof.
    Heartbeat { emitted: u64, measure: usize },
    /// The exploration gate started swallowing positions.
    SuppressionStarted { emitted: u64 },
    /// Emissions resumed after a suppressed stretch.
    Resumed { measure: usize },
}

/// Per-session emission log state. Owned by the session's position-emit
/// closure — a fresh session gets a fresh log, so "first emitted" means
/// first FOR THIS SESSION, unlike the process-wide `Once` it replaces.
#[derive(Debug, Default)]
pub struct ScorePositionLog {
    emitted: u64,
    last_measure: Option<usize>,
    suppressed: bool,
}

impl ScorePositionLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one emission; returns every crumb this emission is worth
    /// (possibly none — steady-state emissions inside a measure are
    /// silent between heartbeats).
    pub fn emitted(&mut self, measure: usize) -> Vec<PositionBreadcrumb> {
        let mut crumbs = Vec::new();
        if self.suppressed {
            self.suppressed = false;
            crumbs.push(PositionBreadcrumb::Resumed { measure });
        }
        if self.emitted == 0 {
            crumbs.push(PositionBreadcrumb::First { measure });
        } else if let Some(last) = self.last_measure {
            if last != measure {
                crumbs.push(PositionBreadcrumb::MeasureChanged {
                    from: last,
                    to: measure,
                    emitted: self.emitted,
                });
            }
        }
        self.emitted += 1;
        self.last_measure = Some(measure);
        if self.emitted.is_multiple_of(HEARTBEAT_EVERY) {
            crumbs.push(PositionBreadcrumb::Heartbeat {
                emitted: self.emitted,
                measure,
            });
        }
        crumbs
    }

    /// Record a position swallowed by the exploration gate. Returns a
    /// crumb only at the START of a suppressed stretch — one line per
    /// exploration, not one per swallowed tick.
    pub fn swallowed(&mut self) -> Option<PositionBreadcrumb> {
        if self.suppressed {
            return None;
        }
        self.suppressed = true;
        Some(PositionBreadcrumb::SuppressionStarted {
            emitted: self.emitted,
        })
    }
}

/// Sanitize a webview-supplied breadcrumb for the log: control characters
/// (including newlines — one breadcrumb is one log line) become spaces,
/// and the result is capped at a UTF-8 boundary.
pub fn clip_frontend_breadcrumb(message: &str) -> String {
    let cleaned: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if cleaned.len() <= FRONTEND_BREADCRUMB_MAX_BYTES {
        return cleaned;
    }
    let mut end = FRONTEND_BREADCRUMB_MAX_BYTES;
    while !cleaned.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &cleaned[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The #354 log gap this module exists to close: a session that keeps
    /// emitting must SAY SO — first line, then heartbeats — so "one line
    /// then silence" can only ever mean emissions actually stopped.
    #[test]
    fn continuing_emissions_produce_first_then_heartbeats() {
        let mut log = ScorePositionLog::new();
        let mut crumbs = Vec::new();
        for _ in 0..250 {
            crumbs.extend(log.emitted(1));
        }
        assert_eq!(crumbs[0], PositionBreadcrumb::First { measure: 1 });
        let heartbeats: Vec<_> = crumbs
            .iter()
            .filter(|c| matches!(c, PositionBreadcrumb::Heartbeat { .. }))
            .collect();
        assert_eq!(
            heartbeats,
            vec![
                &PositionBreadcrumb::Heartbeat {
                    emitted: 100,
                    measure: 1
                },
                &PositionBreadcrumb::Heartbeat {
                    emitted: 200,
                    measure: 1
                },
            ],
            "250 steady emissions log exactly two heartbeats"
        );
        assert_eq!(crumbs.len(), 3, "steady state is otherwise silent");
    }

    /// A measure change is always a line — that's the cursor visibly
    /// moving, the observation the VA is asked to make.
    #[test]
    fn measure_changes_are_logged_with_direction_and_count() {
        let mut log = ScorePositionLog::new();
        log.emitted(1);
        log.emitted(1);
        let crumbs = log.emitted(2);
        assert_eq!(
            crumbs,
            vec![PositionBreadcrumb::MeasureChanged {
                from: 1,
                to: 2,
                emitted: 2
            }]
        );
        // Same measure again: silent.
        assert!(log.emitted(2).is_empty());
    }

    /// Suppression logs ONCE per stretch, and resumption names the
    /// measure the cursor comes back on — the #341 explore gate becomes
    /// visible in the log instead of masquerading as a dead stream.
    #[test]
    fn suppression_logs_once_and_resume_names_the_measure() {
        let mut log = ScorePositionLog::new();
        log.emitted(3);
        assert_eq!(
            log.swallowed(),
            Some(PositionBreadcrumb::SuppressionStarted { emitted: 1 })
        );
        assert_eq!(
            log.swallowed(),
            None,
            "second swallow in a stretch is silent"
        );
        let crumbs = log.emitted(3);
        assert_eq!(crumbs, vec![PositionBreadcrumb::Resumed { measure: 3 }]);
    }

    /// A fresh session starts a fresh log — the process-wide-Once bug
    /// shape (first line can never appear twice) must be impossible.
    #[test]
    fn each_session_gets_its_own_first_line() {
        let mut a = ScorePositionLog::new();
        assert_eq!(a.emitted(1)[0], PositionBreadcrumb::First { measure: 1 });
        let mut b = ScorePositionLog::new();
        assert_eq!(
            b.emitted(5)[0],
            PositionBreadcrumb::First { measure: 5 },
            "a new session's log must announce its own first emission"
        );
    }

    /// Webview breadcrumbs: newlines can't forge extra log lines, and a
    /// runaway string can't bloat the unrotated log file.
    #[test]
    fn frontend_breadcrumbs_are_clipped_and_single_line() {
        assert_eq!(
            clip_frontend_breadcrumb("cursor shown\nimg=(1,2)"),
            "cursor shown img=(1,2)"
        );
        let long = "x".repeat(1000);
        let clipped = clip_frontend_breadcrumb(&long);
        assert!(clipped.len() <= FRONTEND_BREADCRUMB_MAX_BYTES + '…'.len_utf8());
        assert!(clipped.ends_with('…'));
        // Cap lands on a char boundary even mid-multibyte — and the
        // multibyte input is genuinely CLIPPED, not passed through (a
        // skip-clipping-for-multibyte mutant must die here).
        let multi = "é".repeat(400); // 800 bytes
        let clipped_multi = clip_frontend_breadcrumb(&multi);
        assert!(clipped_multi.ends_with('…'));
        assert!(clipped_multi.len() <= FRONTEND_BREADCRUMB_MAX_BYTES + '…'.len_utf8());
    }
}
