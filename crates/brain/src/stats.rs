//! Practice statistics calculations for the history UI.
//!
//! Aggregates session data to compute totals, averages, weekly trends,
//! and other dashboard metrics needed by the practice history feature.

use chrono::{DateTime, Datelike, Duration, Utc, Weekday};
use serde::{Deserialize, Serialize};

use crate::store::SessionSummary;

/// Trend direction comparing this week to last week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trend {
    /// More sessions or more time than last week.
    #[serde(rename = "up")]
    Up,
    /// Fewer sessions or less time than last week.
    #[serde(rename = "down")]
    Down,
    /// Same or similar activity compared to last week.
    #[serde(rename = "stable")]
    Stable,
}

/// Dashboard statistics shown at the top of the history page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PracticeStats {
    /// Total number of sessions ever recorded.
    pub total_sessions: usize,
    /// Total practice time in seconds across all sessions.
    pub total_time_secs: f64,
    /// Number of sessions in the current week (Mon-Sun).
    pub sessions_this_week: usize,
    /// Average session duration in seconds.
    pub avg_session_length_secs: f64,
    /// Practice activity trend: this week vs. last week.
    pub trend: Trend,
}

impl PracticeStats {
    /// Calculate statistics from a list of sessions.
    ///
    /// `now` is used to define "this week" (Mon-Sun in UTC) and
    /// compute the trend against the previous week.
    pub fn calculate(sessions: &[SessionSummary], now: DateTime<Utc>) -> Self {
        let total_sessions = sessions.len();
        let total_time_secs: f64 = sessions.iter().map(|s| s.duration_secs).sum();
        let avg_session_length_secs = if total_sessions == 0 {
            0.0
        } else {
            total_time_secs / total_sessions as f64
        };

        let week_boundaries = Self::week_boundaries(now);
        let last_week_boundaries = (
            week_boundaries.0 - Duration::days(7),
            week_boundaries.1 - Duration::days(7),
        );

        let sessions_this_week = sessions
            .iter()
            .filter(|s| s.started_at >= week_boundaries.0 && s.started_at < week_boundaries.1)
            .count();

        let last_week_sessions = sessions
            .iter()
            .filter(|s| {
                s.started_at >= last_week_boundaries.0 && s.started_at < last_week_boundaries.1
            })
            .count();

        let trend = match (sessions_this_week, last_week_sessions) {
            (0, 0) => Trend::Stable,
            (curr, prev) if curr > prev => Trend::Up,
            (curr, prev) if curr < prev => Trend::Down,
            _ => Trend::Stable,
        };

        Self {
            total_sessions,
            total_time_secs,
            sessions_this_week,
            avg_session_length_secs,
            trend,
        }
    }

    /// Return `(week_start, week_end)` for the week containing `dt`.
    /// Week is Mon 00:00 UTC through Sun 23:59:59 UTC.
    fn week_boundaries(dt: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
        let weekday = dt.weekday();
        let days_since_monday = match weekday {
            Weekday::Mon => 0,
            Weekday::Tue => 1,
            Weekday::Wed => 2,
            Weekday::Thu => 3,
            Weekday::Fri => 4,
            Weekday::Sat => 5,
            Weekday::Sun => 6,
        };
        let week_start = (dt - Duration::days(days_since_monday))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("0:0:0 is always a valid time of day")
            .and_utc();
        let week_end = week_start + Duration::days(7);
        (week_start, week_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(started_at: DateTime<Utc>, duration_secs: f64) -> SessionSummary {
        use crate::session::SessionId;
        SessionSummary {
            id: SessionId::new(),
            instrument: "trumpet".to_owned(),
            started_at,
            duration_secs,
            phrase_count: 1,
        }
    }

    #[test]
    fn empty_sessions_yields_zero_stats() {
        let stats = PracticeStats::calculate(&[], Utc::now());
        assert_eq!(stats.total_sessions, 0);
        assert_eq!(stats.total_time_secs, 0.0);
        assert_eq!(stats.sessions_this_week, 0);
        assert_eq!(stats.avg_session_length_secs, 0.0);
        assert_eq!(stats.trend, Trend::Stable);
    }

    #[test]
    fn single_session_stats() {
        let now = Utc::now();
        let sessions = vec![make_session(now, 1200.0)];
        let stats = PracticeStats::calculate(&sessions, now);

        assert_eq!(stats.total_sessions, 1);
        assert_eq!(stats.total_time_secs, 1200.0);
        assert_eq!(stats.sessions_this_week, 1);
        assert_eq!(stats.avg_session_length_secs, 1200.0);
    }

    #[test]
    fn multiple_sessions_aggregation() {
        let now = Utc::now();
        let sessions = vec![
            make_session(now, 600.0),
            make_session(now - Duration::days(1), 720.0),
            make_session(now - Duration::days(2), 480.0),
        ];
        let stats = PracticeStats::calculate(&sessions, now);

        assert_eq!(stats.total_sessions, 3);
        assert_eq!(stats.total_time_secs, 1800.0);
        assert!((stats.avg_session_length_secs - 600.0).abs() < 0.1);
    }

    #[test]
    fn trend_up_when_more_sessions_this_week() {
        // Use a specific date: 2026-04-21 is a Tuesday
        let wednesday = DateTime::parse_from_rfc3339("2026-04-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // This week: 3 sessions
        let sessions = vec![
            make_session(wednesday, 600.0),
            make_session(wednesday - Duration::days(1), 600.0),
            make_session(wednesday - Duration::days(2), 600.0),
            // Last week: 1 session
            make_session(wednesday - Duration::days(8), 600.0),
        ];
        let stats = PracticeStats::calculate(&sessions, wednesday);

        assert_eq!(stats.trend, Trend::Up);
    }

    #[test]
    fn trend_down_when_fewer_sessions_this_week() {
        let wednesday = DateTime::parse_from_rfc3339("2026-04-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // This week: 1 session
        let sessions = vec![
            make_session(wednesday, 600.0),
            // Last week: 3 sessions
            make_session(wednesday - Duration::days(8), 600.0),
            make_session(wednesday - Duration::days(9), 600.0),
            make_session(wednesday - Duration::days(10), 600.0),
        ];
        let stats = PracticeStats::calculate(&sessions, wednesday);

        assert_eq!(stats.trend, Trend::Down);
    }

    #[test]
    fn trend_stable_when_equal_activity() {
        let wednesday = DateTime::parse_from_rfc3339("2026-04-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // This week: 2 sessions
        let sessions = vec![
            make_session(wednesday, 600.0),
            make_session(wednesday - Duration::days(1), 600.0),
            // Last week: 2 sessions
            make_session(wednesday - Duration::days(8), 600.0),
            make_session(wednesday - Duration::days(9), 600.0),
        ];
        let stats = PracticeStats::calculate(&sessions, wednesday);

        assert_eq!(stats.trend, Trend::Stable);
    }

    #[test]
    fn sessions_this_week_excludes_earlier() {
        let wednesday = DateTime::parse_from_rfc3339("2026-04-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let sessions = vec![
            make_session(wednesday, 600.0),
            make_session(wednesday - Duration::days(1), 600.0),
            make_session(wednesday - Duration::days(2), 600.0), // Monday this week
            make_session(wednesday - Duration::days(8), 600.0), // Sunday last week
            make_session(wednesday - Duration::days(10), 600.0),
        ];
        let stats = PracticeStats::calculate(&sessions, wednesday);

        assert_eq!(stats.sessions_this_week, 3);
    }

    #[test]
    fn avg_session_length_is_mean() {
        let now = Utc::now();
        let sessions = vec![
            make_session(now, 300.0),
            make_session(now - Duration::days(1), 600.0),
            make_session(now - Duration::days(2), 900.0),
        ];
        let stats = PracticeStats::calculate(&sessions, now);

        assert!((stats.avg_session_length_secs - 600.0).abs() < 0.1);
    }
}
