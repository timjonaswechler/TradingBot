//! Session-gate detector. Emits `SessionOpen` / `SessionClose` at the first bar
//! inside / first bar outside a configured daily window.
//!
//! Session window is given as `(start_minute_utc, end_minute_utc)` in `[0, 1440)`.
//! If `start < end`, the window is same-day; otherwise it wraps midnight
//! (e.g. Asia 22:00-06:00 → `start=1320, end=360`).

use domain::Candle;

use super::super::{AnchorEvent, RollingDetector, SessionId};

#[derive(Debug)]
pub struct SessionDetector {
    start_min: u32,
    end_min: u32,
    session: SessionId,
    was_inside: bool,
}

impl SessionDetector {
    /// `start_min` / `end_min` are minutes-of-day in UTC, range `0..1440`.
    pub fn new(start_min: u32, end_min: u32, session: SessionId) -> Self {
        assert!(start_min < 1440 && end_min < 1440);
        Self {
            start_min,
            end_min,
            session,
            was_inside: false,
        }
    }

    /// Parse `"HHMM-HHMM"` in the user's local timezone, given hour-offset.
    /// Returns `(start_utc_min, end_utc_min)`.
    pub fn parse(spec: &str, tz_hours: i32) -> Option<(u32, u32)> {
        let (s, e) = spec.split_once('-')?;
        let parse_hm = |s: &str| -> Option<u32> {
            if s.len() != 4 {
                return None;
            }
            let h: u32 = s[0..2].parse().ok()?;
            let m: u32 = s[2..4].parse().ok()?;
            if h >= 24 || m >= 60 {
                return None;
            }
            Some(h * 60 + m)
        };
        let s_local = parse_hm(s)? as i32;
        let e_local = parse_hm(e)? as i32;
        let to_utc = |local: i32| -> u32 { ((local - tz_hours * 60).rem_euclid(1440)) as u32 };
        Some((to_utc(s_local), to_utc(e_local)))
    }

    fn contains(&self, minute_of_day: u32) -> bool {
        if self.start_min < self.end_min {
            minute_of_day >= self.start_min && minute_of_day < self.end_min
        } else {
            // Wraps midnight.
            minute_of_day >= self.start_min || minute_of_day < self.end_min
        }
    }
}

fn minute_of_day_utc(ts_ms: i64) -> u32 {
    let secs = ts_ms.div_euclid(1000);
    let day_secs = secs.rem_euclid(86_400);
    ((day_secs / 60) as u32).min(1439)
}

impl RollingDetector for SessionDetector {
    fn on_candle(&mut self, c: &Candle, bar: u64) -> Option<AnchorEvent> {
        let inside = self.contains(minute_of_day_utc(c.timestamp));
        let ev = match (self.was_inside, inside) {
            (false, true) => Some(AnchorEvent::SessionOpen {
                bar,
                session: self.session,
            }),
            (true, false) => Some(AnchorEvent::SessionClose {
                bar,
                session: self.session,
            }),
            _ => None,
        };
        self.was_inside = inside;
        ev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle_at(ts_ms: i64) -> Candle {
        Candle {
            timestamp: ts_ms,
            symbol: "T".into(),
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 0.0,
            timeframe: "1m".parse().unwrap(),
        }
    }

    #[test]
    fn parse_same_day_no_offset() {
        assert_eq!(SessionDetector::parse("0900-1700", 0), Some((540, 1020)));
    }

    #[test]
    fn parse_with_positive_tz() {
        // UTC+2 local 09:00 → UTC 07:00 = 420.
        assert_eq!(SessionDetector::parse("0900-1700", 2), Some((420, 900)));
    }

    #[test]
    fn parse_wrapping_session() {
        // Asia 00:00-08:00 local at UTC+2 → UTC 22:00-06:00 = (1320, 360)
        assert_eq!(SessionDetector::parse("0000-0800", 2), Some((1320, 360)));
    }

    #[test]
    fn fires_open_and_close_once() {
        // Window 09:00-10:00 UTC. Bars at 08:00, 09:30, 10:30.
        let mut d = SessionDetector::new(540, 600, SessionId::London);
        let t = |h: i64, m: i64| h * 3600_000 + m * 60_000;
        let e1 = d.on_candle(&candle_at(t(8, 0)), 0);
        let e2 = d.on_candle(&candle_at(t(9, 30)), 1);
        let e3 = d.on_candle(&candle_at(t(10, 30)), 2);
        assert!(matches!(e1, None));
        assert!(matches!(e2, Some(AnchorEvent::SessionOpen { bar: 1, .. })));
        assert!(matches!(e3, Some(AnchorEvent::SessionClose { bar: 2, .. })));
    }

    #[test]
    fn wrapping_window_contains_midnight() {
        // 22:00-06:00 UTC
        let d = SessionDetector::new(1320, 360, SessionId::Asia);
        assert!(d.contains(1400)); // 23:20
        assert!(d.contains(60)); // 01:00
        assert!(!d.contains(720)); // 12:00
    }
}
