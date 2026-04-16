use chrono::{Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// A parsed cron expression.
///
/// Format: `minute hour day_of_month month day_of_week`
///
/// Each field supports:
/// - `*` — any value
/// - `5` — exact value
/// - `1,3,5` — list
/// - `1-5` — range
/// - `*/15` — step (every 15)
/// - `1-30/5` — range with step
///
/// Examples:
/// - `0 8 * * *`     — Every day at 8:00 AM
/// - `*/30 * * * *`  — Every 30 minutes
/// - `0 9-17 * * 1-5` — Every hour 9-5 on weekdays
/// - `0 0 1 * *`     — First day of every month at midnight
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronExpr {
    raw: String,
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days_of_month: Vec<u32>,
    months: Vec<u32>,
    days_of_week: Vec<u32>,
}

impl CronExpr {
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let parts: Vec<&str> = expr.trim().split_whitespace().collect();
        if parts.len() != 5 {
            return Err(CronError::InvalidFormat(format!(
                "expected 5 fields (min hour dom month dow), got {}", parts.len()
            )));
        }

        Ok(Self {
            raw: expr.to_string(),
            minutes: parse_field(parts[0], 0, 59, "minute")?,
            hours: parse_field(parts[1], 0, 23, "hour")?,
            days_of_month: parse_field(parts[2], 1, 31, "day_of_month")?,
            months: parse_field(parts[3], 1, 12, "month")?,
            days_of_week: parse_field(parts[4], 0, 6, "day_of_week")?,
        })
    }

    /// Check if the given UTC time matches this cron expression.
    pub fn matches_now(&self) -> bool {
        let now = Utc::now();
        self.matches_time(
            now.minute(),
            now.hour(),
            now.day(),
            now.month(),
            now.weekday().num_days_from_sunday(),
        )
    }

    /// Check if specific time components match.
    pub fn matches_time(&self, minute: u32, hour: u32, day: u32, month: u32, weekday: u32) -> bool {
        self.minutes.contains(&minute)
            && self.hours.contains(&hour)
            && self.days_of_month.contains(&day)
            && self.months.contains(&month)
            && self.days_of_week.contains(&weekday)
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Human-readable description
    pub fn describe(&self) -> String {
        if self.raw == "* * * * *" {
            return "every minute".to_string();
        }

        let mut parts = Vec::new();

        // Minutes
        if self.minutes.len() == 60 {
            // every minute
        } else if self.minutes.len() == 1 {
            parts.push(format!("at minute {}", self.minutes[0]));
        } else if is_step(&self.minutes, 0, 59) {
            let step = self.minutes[1] - self.minutes[0];
            parts.push(format!("every {} minutes", step));
        }

        // Hours
        if self.hours.len() == 24 {
            // every hour
        } else if self.hours.len() == 1 {
            parts.push(format!("at {:02}:00", self.hours[0]));
        } else if is_step(&self.hours, 0, 23) {
            let step = self.hours[1] - self.hours[0];
            parts.push(format!("every {} hours", step));
        } else {
            parts.push(format!("at hours {}", format_list(&self.hours)));
        }

        // Days
        if self.days_of_month.len() < 31 {
            parts.push(format!("on day {}", format_list(&self.days_of_month)));
        }

        // Months
        if self.months.len() < 12 {
            let names = ["", "Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
            let month_names: Vec<&str> = self.months.iter()
                .filter_map(|m| names.get(*m as usize).copied())
                .collect();
            parts.push(format!("in {}", month_names.join(", ")));
        }

        // Weekdays
        if self.days_of_week.len() < 7 {
            let names = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"];
            let day_names: Vec<&str> = self.days_of_week.iter()
                .filter_map(|d| names.get(*d as usize).copied())
                .collect();
            parts.push(format!("on {}", day_names.join(", ")));
        }

        if parts.is_empty() {
            "every minute".to_string()
        } else {
            parts.join(", ")
        }
    }
}

impl std::fmt::Display for CronExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CronError {
    #[error("invalid cron format: {0}")]
    InvalidFormat(String),
    #[error("invalid value in {field}: {detail}")]
    InvalidValue { field: String, detail: String },
}

fn parse_field(field: &str, min: u32, max: u32, name: &str) -> Result<Vec<u32>, CronError> {
    let mut values = Vec::new();

    for part in field.split(',') {
        let part = part.trim();

        if part == "*" {
            // All values
            values.extend(min..=max);
        } else if let Some(step_str) = part.strip_prefix("*/") {
            // Step: */5 means every 5
            let step: u32 = step_str.parse().map_err(|_| CronError::InvalidValue {
                field: name.to_string(),
                detail: format!("invalid step: {}", step_str),
            })?;
            if step == 0 {
                return Err(CronError::InvalidValue {
                    field: name.to_string(),
                    detail: "step cannot be 0".to_string(),
                });
            }
            let mut v = min;
            while v <= max {
                values.push(v);
                v += step;
            }
        } else if part.contains('/') {
            // Range with step: 1-30/5
            let parts: Vec<&str> = part.split('/').collect();
            if parts.len() != 2 {
                return Err(CronError::InvalidValue {
                    field: name.to_string(),
                    detail: format!("invalid range/step: {}", part),
                });
            }
            let range = parse_range(parts[0], min, max, name)?;
            let step: u32 = parts[1].parse().map_err(|_| CronError::InvalidValue {
                field: name.to_string(),
                detail: format!("invalid step: {}", parts[1]),
            })?;
            if step == 0 {
                return Err(CronError::InvalidValue {
                    field: name.to_string(),
                    detail: "step cannot be 0".to_string(),
                });
            }
            for (i, v) in range.iter().enumerate() {
                if i as u32 % step == 0 {
                    values.push(*v);
                }
            }
        } else if part.contains('-') {
            // Range: 1-5
            values.extend(parse_range(part, min, max, name)?);
        } else {
            // Single value
            let v: u32 = part.parse().map_err(|_| CronError::InvalidValue {
                field: name.to_string(),
                detail: format!("invalid number: {}", part),
            })?;
            if v < min || v > max {
                return Err(CronError::InvalidValue {
                    field: name.to_string(),
                    detail: format!("{} out of range {}-{}", v, min, max),
                });
            }
            values.push(v);
        }
    }

    values.sort();
    values.dedup();

    if values.is_empty() {
        return Err(CronError::InvalidValue {
            field: name.to_string(),
            detail: "no values produced".to_string(),
        });
    }

    Ok(values)
}

fn parse_range(s: &str, min: u32, max: u32, name: &str) -> Result<Vec<u32>, CronError> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(CronError::InvalidValue {
            field: name.to_string(),
            detail: format!("invalid range: {}", s),
        });
    }
    let start: u32 = parts[0].parse().map_err(|_| CronError::InvalidValue {
        field: name.to_string(),
        detail: format!("invalid range start: {}", parts[0]),
    })?;
    let end: u32 = parts[1].parse().map_err(|_| CronError::InvalidValue {
        field: name.to_string(),
        detail: format!("invalid range end: {}", parts[1]),
    })?;

    if start < min || end > max || start > end {
        return Err(CronError::InvalidValue {
            field: name.to_string(),
            detail: format!("range {}-{} out of bounds {}-{}", start, end, min, max),
        });
    }

    Ok((start..=end).collect())
}

fn is_step(values: &[u32], _min: u32, _max: u32) -> bool {
    if values.len() < 2 { return false; }
    let step = values[1] - values[0];
    values.windows(2).all(|w| w[1] - w[0] == step)
}

fn format_list(values: &[u32]) -> String {
    values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_minute() {
        let c = CronExpr::parse("* * * * *").unwrap();
        assert!(c.matches_time(0, 0, 1, 1, 0));
        assert!(c.matches_time(30, 12, 15, 6, 3));
        assert!(c.matches_time(59, 23, 31, 12, 6));
    }

    #[test]
    fn test_specific_time() {
        let c = CronExpr::parse("30 8 * * *").unwrap();
        assert!(c.matches_time(30, 8, 1, 1, 0));
        assert!(!c.matches_time(31, 8, 1, 1, 0));
        assert!(!c.matches_time(30, 9, 1, 1, 0));
    }

    #[test]
    fn test_step() {
        let c = CronExpr::parse("*/15 * * * *").unwrap();
        assert!(c.matches_time(0, 0, 1, 1, 0));
        assert!(c.matches_time(15, 0, 1, 1, 0));
        assert!(c.matches_time(30, 0, 1, 1, 0));
        assert!(c.matches_time(45, 0, 1, 1, 0));
        assert!(!c.matches_time(10, 0, 1, 1, 0));
    }

    #[test]
    fn test_range() {
        let c = CronExpr::parse("0 9-17 * * 1-5").unwrap();
        // 9 AM Monday
        assert!(c.matches_time(0, 9, 1, 1, 1));
        // 5 PM Friday
        assert!(c.matches_time(0, 17, 1, 1, 5));
        // 8 AM (before range)
        assert!(!c.matches_time(0, 8, 1, 1, 1));
        // Sunday (day 0, not in 1-5)
        assert!(!c.matches_time(0, 12, 1, 1, 0));
    }

    #[test]
    fn test_list() {
        let c = CronExpr::parse("0 8,12,18 * * *").unwrap();
        assert!(c.matches_time(0, 8, 1, 1, 0));
        assert!(c.matches_time(0, 12, 1, 1, 0));
        assert!(c.matches_time(0, 18, 1, 1, 0));
        assert!(!c.matches_time(0, 10, 1, 1, 0));
    }

    #[test]
    fn test_monthly() {
        let c = CronExpr::parse("0 0 1 * *").unwrap();
        // First of month at midnight
        assert!(c.matches_time(0, 0, 1, 6, 3));
        // Second of month
        assert!(!c.matches_time(0, 0, 2, 6, 3));
    }

    #[test]
    fn test_range_with_step() {
        let c = CronExpr::parse("0-30/10 * * * *").unwrap();
        assert!(c.matches_time(0, 0, 1, 1, 0));
        assert!(c.matches_time(10, 0, 1, 1, 0));
        assert!(c.matches_time(20, 0, 1, 1, 0));
        assert!(c.matches_time(30, 0, 1, 1, 0));
        assert!(!c.matches_time(5, 0, 1, 1, 0));
        assert!(!c.matches_time(40, 0, 1, 1, 0));
    }

    #[test]
    fn test_invalid_expressions() {
        assert!(CronExpr::parse("").is_err());
        assert!(CronExpr::parse("* *").is_err());
        assert!(CronExpr::parse("60 * * * *").is_err());
        assert!(CronExpr::parse("* 25 * * *").is_err());
        assert!(CronExpr::parse("* * 32 * *").is_err());
        assert!(CronExpr::parse("* * * 13 *").is_err());
        assert!(CronExpr::parse("* * * * 7").is_err());
        assert!(CronExpr::parse("*/0 * * * *").is_err());
    }

    #[test]
    fn test_describe() {
        let c = CronExpr::parse("0 8 * * *").unwrap();
        let desc = c.describe();
        assert!(desc.contains("08:00"));

        let c = CronExpr::parse("*/30 * * * *").unwrap();
        let desc = c.describe();
        assert!(desc.contains("30 minutes"));

        let c = CronExpr::parse("0 9-17 * * 1-5").unwrap();
        let desc = c.describe();
        assert!(desc.contains("Mon"));
        assert!(desc.contains("Fri"));
    }
}
