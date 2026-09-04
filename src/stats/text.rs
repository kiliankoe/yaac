//! The statistics as text, section by section in the desktop's order, within 80
//! columns. Sparklines stand in for the graphs and a shaded grid for the calendar.

use std::fmt::{self, Write as _};

use super::{
    AddedStats, Buttons, Calendar, CardCounts, FutureDue, HISTORY_DAYS, Hour, LocalDay, Passes,
    Periods, Retention, ReviewStats, Stats, Today,
};

const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Width of the label column in period tables, indent included.
const LABEL: usize = 28;
const COLUMN: usize = 17;

/// One character per value, scaled to the largest; zeros sit on the baseline.
fn sparkline(values: &[u32]) -> String {
    let max = values.iter().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|&value| {
            if value == 0 {
                BARS[0]
            } else {
                BARS[((f64::from(value) / f64::from(max)) * 7.0).ceil() as usize]
            }
        })
        .collect()
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scope = if self.search.is_empty() {
            "Collection"
        } else {
            &self.search
        };
        writeln!(f, "{scope}, {}", history(self.all_history))?;
        today(f, &self.today)?;
        future_due(f, &self.future_due)?;
        calendar(f, &self.calendar, self.all_history)?;
        reviews(f, &self.reviews)?;
        card_counts(f, &self.card_counts)?;
        section(f, "Review intervals")?;
        writeln!(
            f,
            "  Median interval  {}",
            days_span(self.intervals.median_days)
        )?;
        if let Some(stability) = &self.stability {
            section(f, "Card stability")?;
            writeln!(
                f,
                "  Median stability  {}",
                days_span(stability.median_days)
            )?;
        }
        if let Some(ease) = &self.ease {
            section(f, "Card ease")?;
            writeln!(f, "  Median ease  {:.0}%", ease.median_percent)?;
        }
        if let Some(difficulty) = &self.difficulty {
            section(f, "Card difficulty")?;
            writeln!(f, "  Median difficulty  {:.0}%", difficulty.median_percent)?;
        }
        if let Some(retrievability) = &self.retrievability {
            section(f, "Card retrievability")?;
            writeln!(
                f,
                "  Average retrievability     {:.1}%",
                retrievability.average_percent
            )?;
            writeln!(
                f,
                "  Estimated total knowledge  {} / {}",
                plural(retrievability.cards_known.round() as u32, "card"),
                plural(retrievability.notes_known.round() as u32, "note")
            )?;
        }
        retention(f, &self.retention)?;
        hours(f, &self.hours, self.all_history)?;
        buttons(f, &self.buttons, self.all_history)?;
        added(f, &self.added)
    }
}

fn history(all: bool) -> &'static str {
    if all { "all history" } else { "last 12 months" }
}

fn section(f: &mut fmt::Formatter<'_>, title: &str) -> fmt::Result {
    write!(f, "\n{title}\n")
}

fn today(f: &mut fmt::Formatter<'_>, today: &Today) -> fmt::Result {
    section(f, "Today")?;
    if today.cards == 0 {
        return writeln!(f, "  No cards have been studied today.");
    }
    let secs = f64::from(today.millis) / 1000.0;
    writeln!(
        f,
        "  Studied {} in {} today ({:.1}s/card)",
        plural(today.cards, "card"),
        study_time(secs),
        secs / f64::from(today.cards)
    )?;
    let again = today.cards - today.correct;
    writeln!(
        f,
        "  Again count: {again} ({})",
        percent(again, today.cards)
    )?;
    writeln!(
        f,
        "  Learn: {}, Review: {}, Relearn: {}, Filtered: {}",
        today.learn, today.review, today.relearn, today.filtered
    )?;
    if today.mature == 0 {
        writeln!(f, "  No mature cards were studied today.")
    } else {
        writeln!(
            f,
            "  Correct answers on mature cards: {}/{} ({})",
            today.mature_correct,
            today.mature,
            percent(today.mature_correct, today.mature)
        )
    }
}

fn future_due(f: &mut fmt::Formatter<'_>, due: &FutureDue) -> fmt::Result {
    section(f, "Future due")?;
    writeln!(f, "  {}  next 31 days", sparkline(&due.due_by_day))?;
    let total: u32 = due.due_by_day.iter().sum();
    let days = due.due_by_day.len() as u32;
    writeln!(f, "  Total         {}", plural(total, "review"))?;
    writeln!(
        f,
        "  Average       {}",
        per_day(average(total, days), "review")
    )?;
    writeln!(
        f,
        "  Due tomorrow  {}",
        plural(due.due_by_day.get(1).copied().unwrap_or(0), "review")
    )?;
    writeln!(f, "  Daily load    {}", per_day(due.daily_load, "review"))?;
    if due.overdue > 0 {
        writeln!(f, "  Overdue       {}", plural(due.overdue, "review"))?;
    }
    Ok(())
}

fn calendar(f: &mut fmt::Formatter<'_>, calendar: &Calendar, all: bool) -> fmt::Result {
    section(f, "Calendar")?;
    if all {
        let this_year = calendar.today.ymd().0;
        let first_year = calendar
            .days
            .keys()
            .next()
            .map_or(this_year, |day| day.ymd().0);
        for year in (first_year..=this_year).rev() {
            if year != this_year {
                writeln!(f)?;
            }
            writeln!(f, "  {year}")?;
            let end = LocalDay::from_ymd(year, 12, 31).min(calendar.today);
            for line in calendar.grid(LocalDay::from_ymd(year, 1, 1), end) {
                writeln!(f, "  {line}")?;
            }
        }
    } else {
        let start = calendar.today.offset(1 - i64::from(HISTORY_DAYS));
        for line in calendar.grid(start, calendar.today) {
            writeln!(f, "  {line}")?;
        }
    }
    let current = if calendar.current_streak_truncated {
        format!("{}+ days", calendar.current_streak)
    } else {
        plural(calendar.current_streak, "day")
    };
    writeln!(f, "  Current streak  {current}")?;
    writeln!(
        f,
        "  Longest streak  {}",
        plural(calendar.longest_streak, "day")
    )
}

fn reviews(f: &mut fmt::Formatter<'_>, reviews: &ReviewStats) -> fmt::Result {
    section(f, "Reviews")?;
    writeln!(f, "  {}  last 31 days", sparkline(&reviews.by_day))?;
    let periods = &reviews.periods;
    period_header(f, periods)?;
    row(f, "Days studied", periods, |p| {
        format!(
            "{} of {} ({:.0}%)",
            p.days_studied,
            p.period_days,
            ratio(p.days_studied, p.period_days) * 100.0
        )
    })?;
    row(f, "Total", periods, |p| plural(p.count, "review"))?;
    row(f, "Average over period", periods, |p| {
        per_day(average(p.count, p.period_days), "review")
    })?;
    row(f, "Average for days studied", periods, |p| {
        per_day(average(p.count, p.days_studied), "review")
    })?;
    row(f, "Time", periods, |p| time_span(p.millis as f64 / 1000.0))?;
    row(f, "Average answer time", periods, |p| {
        if p.count == 0 {
            return "-".to_string();
        }
        let secs = p.millis as f64 / 1000.0;
        format!(
            "{:.1}s ({:.1}/min)",
            secs / f64::from(p.count),
            f64::from(p.count) * 60.0 / secs
        )
    })
}

fn card_counts(f: &mut fmt::Formatter<'_>, counts: &CardCounts) -> fmt::Result {
    section(f, "Card counts")?;
    let total = counts.new
        + counts.learning
        + counts.relearning
        + counts.young
        + counts.mature
        + counts.suspended
        + counts.buried;
    let mut rows = vec![
        ("New", counts.new),
        ("Learning", counts.learning),
        ("Relearning", counts.relearning),
        ("Young", counts.young),
        ("Mature", counts.mature),
    ];
    if counts.separate_inactive {
        rows.push(("Suspended", counts.suspended));
        rows.push(("Buried", counts.buried));
    }
    for (label, count) in rows {
        writeln!(f, "  {label:<12}{count:>7}  {:>6}", percent(count, total))?;
    }
    writeln!(f, "  {:<12}{total:>7}", "Total")
}

fn retention(f: &mut fmt::Formatter<'_>, retention: &Retention) -> fmt::Result {
    section(f, "Retention")?;
    writeln!(f, "  Pass rate of cards with an interval of a day or more")?;
    writeln!(
        f,
        "  {:<12}{:>8}{:>8}{:>8}{:>8}",
        "", "Young", "Mature", "Total", "Count"
    )?;
    let mut rows = vec![
        ("Today", &retention.today),
        ("Yesterday", &retention.yesterday),
        ("Last week", &retention.week),
        ("Last month", &retention.month),
        ("Last year", &retention.year),
    ];
    if let Some(all_time) = &retention.all_time {
        rows.push(("All time", all_time));
    }
    for (label, passes) in rows {
        let Passes {
            young_passed,
            young_failed,
            mature_passed,
            mature_failed,
        } = *passes;
        let passed = young_passed + mature_passed;
        let total = passed + young_failed + mature_failed;
        writeln!(
            f,
            "  {label:<12}{:>8}{:>8}{:>8}{total:>8}",
            percent(young_passed, young_passed + young_failed),
            percent(mature_passed, mature_passed + mature_failed),
            percent(passed, total),
        )?;
    }
    Ok(())
}

fn hours(f: &mut fmt::Formatter<'_>, hours: &[Hour], all: bool) -> fmt::Result {
    section(f, &format!("Hourly breakdown ({})", history(all)))?;
    let reviews: Vec<u32> = hours.iter().map(|hour| hour.reviews).collect();
    writeln!(f, "  Reviews   {}", sparkline(&reviews))?;
    let correct: String = hours
        .iter()
        .map(|hour| {
            if hour.reviews == 0 {
                ' '
            } else {
                BARS[(ratio(hour.correct, hour.reviews) * 7.0).round() as usize]
            }
        })
        .collect();
    writeln!(f, "  Correct   {}", correct.trim_end())?;
    writeln!(f, "            0     6     12    18")
}

fn buttons(f: &mut fmt::Formatter<'_>, buttons: &Buttons, all: bool) -> fmt::Result {
    section(f, &format!("Answer buttons ({})", history(all)))?;
    writeln!(
        f,
        "  {:<12}{:>8}{:>8}{:>8}{:>8}{:>9}",
        "", "Again", "Hard", "Good", "Easy", "Correct"
    )?;
    for (label, counts) in [
        ("Learning", &buttons.learning),
        ("Young", &buttons.young),
        ("Mature", &buttons.mature),
    ] {
        let total: u32 = counts.iter().sum();
        writeln!(
            f,
            "  {label:<12}{:>8}{:>8}{:>8}{:>8}{:>9}",
            counts[0],
            counts[1],
            counts[2],
            counts[3],
            percent(total - counts[0], total)
        )?;
    }
    Ok(())
}

fn added(f: &mut fmt::Formatter<'_>, added: &AddedStats) -> fmt::Result {
    section(f, "Added")?;
    writeln!(f, "  {}  last 31 days", sparkline(&added.by_day))?;
    let periods = &added.periods;
    period_header(f, periods)?;
    row(f, "Total", periods, |p| plural(p.cards, "card"))?;
    row(f, "Average", periods, |p| {
        per_day(average(p.cards, p.period_days), "card")
    })
}

fn period_header<T>(f: &mut fmt::Formatter<'_>, periods: &Periods<T>) -> fmt::Result {
    let mut line = " ".repeat(LABEL);
    for label in periods.labels() {
        write!(line, "{label:<COLUMN$}")?;
    }
    writeln!(f, "{}", line.trim_end())
}

fn row<T>(
    f: &mut fmt::Formatter<'_>,
    label: &str,
    periods: &Periods<T>,
    value: impl Fn(&T) -> String,
) -> fmt::Result {
    let mut line = format!("  {label:<width$}", width = LABEL - 2);
    for period in periods.iter() {
        write!(line, "{:<COLUMN$}", value(period))?;
    }
    writeln!(f, "{}", line.trim_end())
}

fn plural(n: u32, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn per_day(n: u32, noun: &str) -> String {
    format!("{}/day", plural(n, noun))
}

fn ratio(part: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(part) / f64::from(total)
    }
}

fn average(total: u32, days: u32) -> u32 {
    (ratio(total, days)).round() as u32
}

/// "N/A" without a denominator, like the desktop's retention table.
fn percent(part: u32, total: u32) -> String {
    if total == 0 {
        "N/A".to_string()
    } else {
        format!("{:.1}%", ratio(part, total) * 100.0)
    }
}

/// Seconds or minutes, never hours, as in the desktop's "studied today" line.
fn study_time(secs: f64) -> String {
    if secs < 60.0 {
        plural(secs.round() as u32, "second")
    } else {
        plural((secs / 60.0).round() as u32, "minute")
    }
}

fn time_span(secs: f64) -> String {
    if secs < 60.0 {
        plural(secs.round() as u32, "second")
    } else if secs < 3600.0 {
        plural((secs / 60.0).round() as u32, "minute")
    } else {
        format!("{:.1} hours", secs / 3600.0)
    }
}

/// Days, months, or years, using the desktop's month of 30.44 days.
fn days_span(days: u32) -> String {
    let d = f64::from(days);
    if d < 30.0 {
        plural(days, "day")
    } else if d < 365.0 {
        format!("{:.1} months", d / 30.4375)
    } else {
        format!("{:.1} years", d / 365.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparklines_scale_to_the_largest_value() {
        assert_eq!(sparkline(&[0, 1, 7, 14]), "▁▂▅█");
        assert_eq!(sparkline(&[0, 0]), "▁▁");
        assert_eq!(sparkline(&[]), "");
    }

    #[test]
    fn spans_pick_a_natural_unit() {
        assert_eq!(study_time(13.0), "13 seconds");
        assert_eq!(study_time(5400.0), "90 minutes");
        assert_eq!(time_span(5400.0), "1.5 hours");
        assert_eq!(days_span(1), "1 day");
        assert_eq!(days_span(29), "29 days");
        assert_eq!(days_span(61), "2.0 months");
        assert_eq!(days_span(730), "2.0 years");
        assert_eq!(percent(1, 3), "33.3%");
        assert_eq!(percent(0, 0), "N/A");
    }
}
