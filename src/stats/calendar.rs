//! Calendar days for the review heatmap and the streaks. rslib reports reviews as "days
//! ago" on Anki days that begin at the rollover hour; this maps them onto dates with
//! whole-day arithmetic, which is all a grid of cells needs.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Serialize, Serializer};

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A calendar day in local time, counted from 1970-01-01.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct LocalDay(pub i64);

impl LocalDay {
    /// The day that `secs` of Unix time falls on, `offset_secs` east of UTC.
    pub fn from_unix(secs: i64, offset_secs: i64) -> Self {
        Self((secs + offset_secs).div_euclid(86_400))
    }

    // Conversions after Howard Hinnant's date algorithms, on a calendar that starts
    // in March so leap days land at the end of the year.
    pub fn from_ymd(year: i64, month: u32, day: u32) -> Self {
        let year = if month <= 2 { year - 1 } else { year };
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month = i64::from(if month > 2 { month - 3 } else { month + 9 });
        let day_of_year = (153 * month + 2) / 5 + i64::from(day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        Self(era * 146_097 + day_of_era - 719_468)
    }

    pub fn ymd(self) -> (i64, u32, u32) {
        let z = self.0 + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        } as u32;
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        (year, month, day)
    }

    /// 0 is Sunday, as in Anki's first-day-of-week setting.
    pub fn weekday(self) -> u32 {
        (self.0 + 4).rem_euclid(7) as u32
    }

    pub fn offset(self, days: i64) -> Self {
        Self(self.0 + days)
    }
}

impl fmt::Display for LocalDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.ymd();
        write!(f, "{year:04}-{month:02}-{day:02}")
    }
}

impl Serialize for LocalDay {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Reviews per calendar day and the streaks they form.
#[derive(Serialize)]
pub struct Calendar {
    pub today: LocalDay,
    /// Days with at least one review.
    pub days: BTreeMap<LocalDay, u32>,
    /// Consecutive days studied up to today, or up to yesterday while today is empty.
    pub current_streak: u32,
    /// The streak runs past the fetched history, so the number is a lower bound.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub current_streak_truncated: bool,
    pub longest_streak: u32,
    /// The heatmap's first row, 0 for Sunday.
    #[serde(skip)]
    pub first_weekday: u32,
}

impl Calendar {
    /// `earliest` is the first day the review history covers.
    pub fn new(
        today: LocalDay,
        earliest: LocalDay,
        first_weekday: u32,
        days: BTreeMap<LocalDay, u32>,
    ) -> Self {
        let days: BTreeMap<_, _> = days.into_iter().filter(|(_, n)| *n > 0).collect();
        let studied = |day: LocalDay| days.contains_key(&day);

        let mut day = if studied(today) {
            today
        } else {
            today.offset(-1)
        };
        let mut current_streak = 0;
        while studied(day) {
            current_streak += 1;
            day = day.offset(-1);
        }
        let current_streak_truncated = current_streak > 0 && day < earliest;

        let mut longest_streak = 0;
        let mut run = 0;
        let mut previous: Option<LocalDay> = None;
        for &day in days.keys() {
            run = match previous {
                Some(previous) if previous.offset(1) == day => run + 1,
                _ => 1,
            };
            longest_streak = longest_streak.max(run);
            previous = Some(day);
        }

        Self {
            today,
            days,
            current_streak,
            current_streak_truncated,
            longest_streak,
            first_weekday: first_weekday % 7,
        }
    }

    /// The heatmap for `start..=end`: a line of month names, then one per weekday with
    /// a column per week. Cells outside the range stay blank.
    pub fn grid(&self, start: LocalDay, end: LocalDay) -> Vec<String> {
        let first_column = start.offset(-i64::from((start.weekday() + 7 - self.first_weekday) % 7));
        let columns = ((end.0 - first_column.0) / 7 + 1).max(0) as usize;
        let max = self
            .days
            .range(start..=end)
            .map(|(_, &n)| n)
            .max()
            .unwrap_or(0);
        let mut lines = vec![self.month_names(first_column, columns, start, end)];
        for row in 0..7 {
            let mut line = format!("{}  ", WEEKDAYS[((self.first_weekday + row) % 7) as usize]);
            for column in 0..columns {
                let day = first_column.offset(column as i64 * 7 + i64::from(row));
                line.push(if day < start || day > end {
                    ' '
                } else {
                    cell(self.days.get(&day).copied().unwrap_or(0), max)
                });
            }
            lines.push(line.trim_end().to_string());
        }
        lines
    }

    /// A month's name sits over the week that holds its first day.
    fn month_names(
        &self,
        first_column: LocalDay,
        columns: usize,
        start: LocalDay,
        end: LocalDay,
    ) -> String {
        let mut names = vec![' '; columns + 3];
        let mut free = 0;
        for column in 0..columns {
            let week = first_column.offset(column as i64 * 7);
            let first_of_month = (0..7)
                .map(|day| week.offset(day))
                .find(|day| *day >= start && *day <= end && day.ymd().2 == 1);
            if let Some(day) = first_of_month {
                if column >= free {
                    let name = MONTHS[(day.ymd().1 - 1) as usize];
                    for (i, c) in name.chars().enumerate() {
                        names[column + i] = c;
                    }
                    free = column + 4;
                }
            }
        }
        let names: String = names.into_iter().collect();
        format!("     {}", names.trim_end())
    }
}

fn cell(count: u32, max: u32) -> char {
    if count == 0 {
        return '·';
    }
    // Square root like the desktop, so a few heavy days do not wash out the rest.
    let level = ((f64::from(count) / f64::from(max)).sqrt() * 4.0).ceil() as usize;
    ['░', '▒', '▓', '█'][level.clamp(1, 4) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(LocalDay::from_ymd(1970, 1, 1), LocalDay(0));
        assert_eq!(LocalDay(0).ymd(), (1970, 1, 1));
        assert_eq!(LocalDay::from_ymd(2000, 3, 1), LocalDay(11_017));
        for (year, month, day) in [(1999, 12, 31), (2000, 2, 29), (2026, 9, 4), (1969, 12, 31)] {
            let local = LocalDay::from_ymd(year, month, day);
            assert_eq!(local.ymd(), (year, month, day));
            assert_eq!(local.to_string(), format!("{year}-{month:02}-{day:02}"));
        }
        // 2026-09-04 is a Friday.
        assert_eq!(LocalDay::from_ymd(2026, 9, 4).weekday(), 5);
        assert_eq!(LocalDay(0).weekday(), 4, "1970 began on a Thursday");
    }

    #[test]
    fn a_day_belongs_to_the_local_date() {
        // 23:30 UTC on 1970-01-01 is already the 2nd one hour east of UTC.
        assert_eq!(LocalDay::from_unix(84_600, 3600), LocalDay(1));
        assert_eq!(LocalDay::from_unix(84_600, -3600), LocalDay(0));
        assert_eq!(LocalDay::from_unix(1800, -3600), LocalDay(-1));
    }

    fn calendar(today: LocalDay, earliest: LocalDay, studied: &[i64]) -> Calendar {
        let days = studied
            .iter()
            .map(|&offset| (today.offset(offset), 1))
            .collect();
        Calendar::new(today, earliest, 1, days)
    }

    #[test]
    fn streaks_count_consecutive_days_and_forgive_an_empty_today() {
        let today = LocalDay::from_ymd(2026, 9, 4);
        let earliest = today.offset(-365);

        let c = calendar(today, earliest, &[0, -1, -2]);
        assert_eq!((c.current_streak, c.longest_streak), (3, 3));
        assert!(!c.current_streak_truncated);

        let c = calendar(today, earliest, &[-1, -2]);
        assert_eq!(c.current_streak, 2, "today is not over yet");

        let c = calendar(today, earliest, &[-1, -3, -4, -5]);
        assert_eq!((c.current_streak, c.longest_streak), (1, 3));

        let c = calendar(today, earliest, &[-2, -3]);
        assert_eq!(c.current_streak, 0, "a day was missed");
        assert_eq!(c.longest_streak, 2);

        let all: Vec<i64> = (-365..=0).collect();
        let c = calendar(today, earliest, &all);
        assert_eq!(c.current_streak, 366);
        assert!(c.current_streak_truncated, "may go on before the history");
    }

    #[test]
    fn the_grid_has_a_column_per_week_and_shades_by_count() {
        let today = LocalDay::from_ymd(2026, 9, 4);
        let mut days = BTreeMap::new();
        days.insert(LocalDay::from_ymd(2026, 8, 31), 1); // Monday
        days.insert(LocalDay::from_ymd(2026, 9, 1), 4); // Tuesday
        days.insert(LocalDay::from_ymd(2026, 9, 4), 16); // Friday
        let calendar = Calendar::new(today, today.offset(-365), 1, days);

        let lines = calendar.grid(LocalDay::from_ymd(2026, 8, 26), today);
        assert_eq!(
            lines,
            [
                "      Sep",
                "Mon   ░",
                "Tue   ▒",
                "Wed  ··",
                "Thu  ··",
                "Fri  ·█",
                "Sat  ·",
                "Sun  ·",
            ]
        );
    }

    #[test]
    fn the_grid_starts_the_week_on_the_configured_day() {
        let today = LocalDay::from_ymd(2026, 9, 6); // a Sunday
        let mut days = BTreeMap::new();
        days.insert(today, 1);
        let calendar = Calendar::new(today, today.offset(-365), 0, days);
        let lines = calendar.grid(today.offset(-1), today);
        assert_eq!(
            lines[1], "Sun   █",
            "Sunday heads the rows and a new column"
        );
        assert_eq!(lines[7], "Sat  ·");
    }
}
