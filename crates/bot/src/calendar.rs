use chrono::{DateTime, Datelike, NaiveDate, Utc};

/// Describes the calendar phase of a given date.
#[derive(Debug, Clone)]
pub struct CalendarPhase {
    /// True if within `params.month_start_days` calendar days from month start
    pub is_month_start: bool,
    /// True if within `params.month_end_days` calendar days from month end
    pub is_month_end: bool,
    /// True if within last 5 calendar days of a quarter end month (Mar/Jun/Sep/Dec)
    pub is_quarter_end: bool,
    /// True if in January and day <= 5 (January effect)
    pub is_year_start: bool,
    /// True if in December and day >= 26
    pub is_year_end: bool,
    pub day_of_month: u32,
    pub month: u32,
    pub quarter: u32,
}

#[derive(Debug, Clone)]
pub struct CalendarParams {
    /// Boost to apply during month start (additive, e.g. +0.3)
    pub month_start_boost: f64,
    /// Number of days from month start to consider "month start" [1..5]
    pub month_start_days: usize,
    /// Caution penalty during month end (additive, e.g. -0.3)
    pub month_end_caution: f64,
    /// Number of days from month end to consider "month end" [1..5]
    pub month_end_days: usize,
    /// Extra caution penalty during quarter end (additive, e.g. -0.2)
    pub quarter_end_caution: f64,
    /// Boost during year start / January effect (additive, e.g. +0.2)
    pub year_end_boost: f64,
}

impl Default for CalendarParams {
    fn default() -> Self {
        Self {
            month_start_boost: 0.3,
            month_start_days: 3,
            month_end_caution: -0.2,
            month_end_days: 3,
            quarter_end_caution: -0.15,
            year_end_boost: 0.2,
        }
    }
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    let next_month_first = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    next_month_first
        .expect("valid date")
        .pred_opt()
        .expect("valid predecessor")
        .day()
}

/// Detect the calendar phase for a given UTC timestamp.
/// `start_days`: how many days into month counts as "start"
/// `end_days`: how many days before end of month counts as "end"
pub fn detect_phase(dt: &DateTime<Utc>, start_days: usize, end_days: usize) -> CalendarPhase {
    let day = dt.day();
    let month = dt.month();
    let year = dt.year();

    let last_day = last_day_of_month(year, month);
    let quarter = (month - 1) / 3 + 1;

    let is_month_start = day <= start_days as u32;
    let is_month_end = day >= last_day.saturating_sub(end_days as u32);
    let is_quarter_end = matches!(month, 3 | 6 | 9 | 12) && day >= last_day.saturating_sub(4);
    let is_year_start = month == 1 && day <= 5;
    let is_year_end = month == 12 && day >= 26;

    CalendarPhase {
        is_month_start,
        is_month_end,
        is_quarter_end,
        is_year_start,
        is_year_end,
        day_of_month: day,
        month,
        quarter,
    }
}

/// Compute a composite calendar modifier to add to a trading score.
/// Positive = bullish boost, negative = caution/penalty.
pub fn compute_modifier(phase: &CalendarPhase, params: &CalendarParams) -> f64 {
    let mut modifier = 0.0;

    if phase.is_month_start {
        modifier += params.month_start_boost;
    }
    if phase.is_month_end {
        modifier += params.month_end_caution;
    }
    if phase.is_quarter_end {
        modifier += params.quarter_end_caution;
    }
    if phase.is_year_start || phase.is_year_end {
        modifier += params.year_end_boost;
    }

    modifier
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_month_start() {
        let dt = Utc.with_ymd_and_hms(2024, 3, 1, 12, 0, 0).unwrap();
        let phase = detect_phase(&dt, 3, 3);
        assert!(phase.is_month_start);
        assert!(!phase.is_month_end);
        assert_eq!(phase.day_of_month, 1);
        assert_eq!(phase.month, 3);
        assert_eq!(phase.quarter, 1);
    }

    #[test]
    fn test_month_end() {
        let dt = Utc.with_ymd_and_hms(2024, 3, 31, 12, 0, 0).unwrap();
        let phase = detect_phase(&dt, 3, 3);
        assert!(phase.is_month_end);
        assert!(!phase.is_month_start);
    }

    #[test]
    fn test_quarter_end_march() {
        let dt = Utc.with_ymd_and_hms(2024, 3, 31, 12, 0, 0).unwrap();
        let phase = detect_phase(&dt, 3, 3);
        assert!(phase.is_quarter_end);
        assert!(phase.is_month_end);
        assert_eq!(phase.quarter, 1);
    }

    #[test]
    fn test_year_start() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 3, 12, 0, 0).unwrap();
        let phase = detect_phase(&dt, 3, 3);
        assert!(phase.is_year_start);
        assert!(!phase.is_year_end);
    }

    #[test]
    fn test_modifier_accumulates() {
        let dt = Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap();
        let params = CalendarParams::default();
        let phase = detect_phase(&dt, params.month_start_days, params.month_end_days);
        let modifier = compute_modifier(&phase, &params);
        assert!(modifier > 0.0);
        assert!((modifier - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_quarter_end_stacks_with_month_end() {
        let dt = Utc.with_ymd_and_hms(2024, 12, 31, 12, 0, 0).unwrap();
        let params = CalendarParams::default();
        let phase = detect_phase(&dt, params.month_start_days, params.month_end_days);
        let modifier = compute_modifier(&phase, &params);
        let expected = params.month_end_caution + params.quarter_end_caution + params.year_end_boost;
        assert!((modifier - expected).abs() < 1e-10);
    }
}
