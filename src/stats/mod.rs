//! Collection statistics as the desktop's stats screen computes them. rslib does the
//! counting through the same service the desktop calls; this module keeps the periods
//! the desktop shows by default (a month for due and added cards, a year for buttons
//! and hours) and `text` lays the numbers out for a terminal.

mod calendar;
mod text;

use std::collections::{BTreeMap, HashMap};

use anki::collection::Collection;
use anki::services::StatsService;
use anki::timestamp::TimestampSecs;
use anki_proto::stats::GraphsRequest;
use anki_proto::stats::graphs_response as graph;
use anyhow::Result;
use serde::Serialize;

pub use calendar::{Calendar, LocalDay};

use crate::session::AnkiResultExt;

/// How far back the review history is fetched unless all of it is wanted, the
/// desktop's "last 12 months".
pub const HISTORY_DAYS: u32 = 365;

#[derive(Serialize)]
pub struct Stats {
    /// The search the numbers are limited to; empty for the whole collection.
    pub search: String,
    pub all_history: bool,
    /// FSRS is on, which swaps ease for difficulty, stability, and retrievability.
    pub fsrs: bool,
    pub today: Today,
    pub future_due: FutureDue,
    pub calendar: Calendar,
    pub reviews: ReviewStats,
    pub card_counts: CardCounts,
    pub intervals: Intervals,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<Intervals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ease: Option<Median>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Median>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrievability: Option<Retrievability>,
    pub retention: Retention,
    /// One entry per hour of the day, local time.
    pub hours: Vec<Hour>,
    pub buttons: Buttons,
    pub added: AddedStats,
}

#[derive(Serialize, Default)]
pub struct Today {
    pub cards: u32,
    pub millis: u32,
    /// Answered with anything but Again.
    pub correct: u32,
    pub mature: u32,
    pub mature_correct: u32,
    pub learn: u32,
    pub review: u32,
    pub relearn: u32,
    pub filtered: u32,
}

#[derive(Serialize)]
pub struct FutureDue {
    /// Cards due today and on each of the next 30 days.
    pub due_by_day: Vec<u32>,
    pub overdue: u32,
    /// rslib's forecast of reviews per day from the current intervals.
    pub daily_load: u32,
}

/// The same numbers over the periods the desktop offers: 31 days, a year, and with all
/// history the time since the oldest entry.
#[derive(Serialize)]
pub struct Periods<T> {
    pub month: T,
    pub year: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_time: Option<T>,
}

impl<T> Periods<T> {
    fn new(all_history: bool, oldest_day: i32, summarize: impl Fn(u32) -> T) -> Self {
        Self {
            month: summarize(31),
            year: summarize(HISTORY_DAYS),
            all_time: all_history.then(|| summarize((1 - oldest_day.min(0)) as u32)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        [Some(&self.month), Some(&self.year), self.all_time.as_ref()]
            .into_iter()
            .flatten()
    }

    pub fn labels(&self) -> impl Iterator<Item = &'static str> {
        ["last 31 days", "last 12 months", "all time"]
            .into_iter()
            .take(if self.all_time.is_some() { 3 } else { 2 })
    }
}

#[derive(Serialize)]
pub struct ReviewStats {
    /// Reviews on each of the last 31 days, today last.
    pub by_day: Vec<u32>,
    #[serde(flatten)]
    pub periods: Periods<Reviews>,
}

#[derive(Serialize)]
pub struct Reviews {
    pub period_days: u32,
    pub days_studied: u32,
    pub count: u32,
    pub millis: u64,
}

#[derive(Serialize)]
pub struct AddedStats {
    /// Cards added on each of the last 31 days, today last.
    pub by_day: Vec<u32>,
    #[serde(flatten)]
    pub periods: Periods<Added>,
}

#[derive(Serialize)]
pub struct Added {
    pub period_days: u32,
    pub cards: u32,
}

#[derive(Serialize)]
pub struct CardCounts {
    pub new: u32,
    pub learning: u32,
    pub relearning: u32,
    pub young: u32,
    pub mature: u32,
    pub suspended: u32,
    pub buried: u32,
    /// Suspended and buried cards are counted on their own instead of by type, the
    /// desktop's "separate suspended/buried cards" checkbox.
    pub separate_inactive: bool,
}

#[derive(Serialize)]
pub struct Intervals {
    pub median_days: u32,
}

#[derive(Serialize)]
pub struct Median {
    pub median_percent: f32,
}

#[derive(Serialize)]
pub struct Retrievability {
    pub average_percent: f32,
    /// Retrievability summed over cards, the desktop's "estimated total knowledge".
    pub cards_known: f32,
    pub notes_known: f32,
}

#[derive(Serialize)]
pub struct Retention {
    pub today: Passes,
    pub yesterday: Passes,
    pub week: Passes,
    pub month: Passes,
    pub year: Passes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_time: Option<Passes>,
}

/// Reviews of cards with an interval of at least a day, by outcome and maturity.
#[derive(Serialize, Default, Clone, Copy)]
pub struct Passes {
    pub young_passed: u32,
    pub young_failed: u32,
    pub mature_passed: u32,
    pub mature_failed: u32,
}

#[derive(Serialize, Default, Clone, Copy)]
pub struct Hour {
    pub reviews: u32,
    pub correct: u32,
}

/// Presses of Again, Hard, Good, and Easy.
#[derive(Serialize, Default)]
pub struct Buttons {
    pub learning: [u32; 4],
    pub young: [u32; 4],
    pub mature: [u32; 4],
}

pub fn collect(col: &mut Collection, search: &str, all_history: bool) -> Result<Stats> {
    let request = GraphsRequest {
        search: search.to_string(),
        days: if all_history { 0 } else { HISTORY_DAYS },
    };
    let graphs = StatsService::graphs(col, request).ctx("computing statistics")?;
    let prefs = StatsService::get_graph_preferences(col).ctx("reading statistics preferences")?;
    let now = TimestampSecs::now();
    let offset = i64::from(
        now.local_utc_offset()
            .ctx("determining the time zone")?
            .local_minus_utc(),
    );
    // Anki days begin at the rollover hour, so the current one has the date it was
    // that many hours ago.
    let today = LocalDay::from_unix(now.0 - i64::from(graphs.rollover_hour) * 3600, offset);

    let review_log = graphs.reviews.unwrap_or_default();
    let sum = |r: &graph::review_counts_and_times::Reviews| {
        r.learn + r.relearn + r.young + r.mature + r.filtered
    };
    let count_by_day: BTreeMap<i32, u32> = review_log
        .count
        .iter()
        .map(|(&day, r)| (day, sum(r)))
        .collect();
    let millis_by_day: BTreeMap<i32, u64> = review_log
        .time
        .iter()
        .map(|(&day, r)| (day, u64::from(sum(r))))
        .collect();
    let oldest_review = count_by_day.keys().next().copied().unwrap_or(0);
    let earliest = if all_history {
        today.offset(i64::from(oldest_review))
    } else {
        today.offset(-i64::from(HISTORY_DAYS))
    };
    let first_weekday = u32::try_from(prefs.calendar_first_day_of_week).unwrap_or(0);
    let calendar = Calendar::new(
        today,
        earliest,
        first_weekday,
        count_by_day
            .iter()
            .map(|(&day, &n)| (today.offset(i64::from(day)), n))
            .collect(),
    );

    let reviews = ReviewStats {
        by_day: last_month(&count_by_day),
        periods: Periods::new(all_history, oldest_review, |period_days| {
            let from = 1 - period_days as i32;
            let mut summary = Reviews {
                period_days,
                days_studied: 0,
                count: 0,
                millis: millis_by_day.range(from..=0).map(|(_, &ms)| ms).sum(),
            };
            for (_, &n) in count_by_day.range(from..=0) {
                if n > 0 {
                    summary.days_studied += 1;
                    summary.count += n;
                }
            }
            summary
        }),
    };

    let added_by_day: BTreeMap<i32, u32> = graphs
        .added
        .map(|added| added.added.into_iter().collect())
        .unwrap_or_default();
    let oldest_added = added_by_day.keys().next().copied().unwrap_or(0);
    let added = AddedStats {
        by_day: last_month(&added_by_day),
        periods: Periods::new(all_history, oldest_added, |period_days| Added {
            period_days,
            cards: added_by_day
                .range(1 - period_days as i32..=0)
                .map(|(_, &n)| n)
                .sum(),
        }),
    };

    let future = graphs.future_due.unwrap_or_default();
    let future_due = FutureDue {
        due_by_day: (0..31)
            .map(|day| future.future_due.get(&day).copied().unwrap_or(0))
            .collect(),
        overdue: future
            .future_due
            .iter()
            .filter(|(day, _)| **day < 0)
            .map(|(_, &n)| n)
            .sum(),
        daily_load: future.daily_load,
    };

    let separate_inactive = prefs.card_counts_separate_inactive;
    let counts = graphs
        .card_counts
        .and_then(|counts| {
            if separate_inactive {
                counts.excluding_inactive
            } else {
                counts.including_inactive
            }
        })
        .unwrap_or_default();
    let card_counts = CardCounts {
        new: counts.new_cards,
        learning: counts.learn,
        relearning: counts.relearn,
        young: counts.young,
        mature: counts.mature,
        suspended: counts.suspended,
        buried: counts.buried,
        separate_inactive,
    };

    let t = graphs.today.unwrap_or_default();
    let today = Today {
        cards: t.answer_count,
        millis: t.answer_millis,
        correct: t.correct_count,
        mature: t.mature_count,
        mature_correct: t.mature_correct,
        learn: t.learn_count,
        review: t.review_count,
        relearn: t.relearn_count,
        filtered: t.early_review_count,
    };

    let fsrs = graphs.fsrs;
    let median_of = |intervals: Option<graph::Intervals>| Intervals {
        median_days: median(&intervals.map(|i| i.intervals).unwrap_or_default()),
    };
    // rslib calls the field "average" but computes the median, like the desktop shows.
    let median_percent = |eases: Option<graph::Eases>| Median {
        median_percent: eases.map(|e| e.average).unwrap_or_default(),
    };
    let retrievability = fsrs.then(|| {
        let r = graphs.retrievability.unwrap_or_default();
        Retrievability {
            average_percent: r.average,
            cards_known: r.sum_by_card,
            notes_known: r.sum_by_note,
        }
    });

    let true_retention = graphs.true_retention.unwrap_or_default();
    let passes = |p: Option<graph::true_retention_stats::TrueRetention>| {
        let p = p.unwrap_or_default();
        Passes {
            young_passed: p.young_passed,
            young_failed: p.young_failed,
            mature_passed: p.mature_passed,
            mature_failed: p.mature_failed,
        }
    };
    let retention = Retention {
        today: passes(true_retention.today),
        yesterday: passes(true_retention.yesterday),
        week: passes(true_retention.week),
        month: passes(true_retention.month),
        year: passes(true_retention.year),
        all_time: all_history.then(|| passes(true_retention.all_time)),
    };

    // The desktop's hour and button graphs default to a year, or to everything when
    // all history is loaded.
    let hours_log = graphs.hours.unwrap_or_default();
    let hours_bucket = if all_history {
        hours_log.all_time
    } else {
        hours_log.one_year
    };
    let hours = (0..24)
        .map(|hour| {
            hours_bucket
                .get(hour)
                .map(|h| Hour {
                    reviews: h.total,
                    correct: h.correct,
                })
                .unwrap_or_default()
        })
        .collect();
    let buttons_log = graphs.buttons.unwrap_or_default();
    let buttons_bucket = if all_history {
        buttons_log.all_time
    } else {
        buttons_log.one_year
    }
    .unwrap_or_default();
    let four = |counts: Vec<u32>| {
        let mut out = [0; 4];
        for (slot, n) in out.iter_mut().zip(counts) {
            *slot = n;
        }
        out
    };
    let buttons = Buttons {
        learning: four(buttons_bucket.learning),
        young: four(buttons_bucket.young),
        mature: four(buttons_bucket.mature),
    };

    Ok(Stats {
        search: search.to_string(),
        all_history,
        fsrs,
        today,
        future_due,
        calendar,
        reviews,
        card_counts,
        intervals: median_of(graphs.intervals),
        stability: fsrs.then(|| median_of(graphs.stability)),
        ease: (!fsrs).then(|| median_percent(graphs.eases)),
        difficulty: fsrs.then(|| median_percent(graphs.difficulty)),
        retrievability,
        retention,
        hours,
        buttons,
        added,
    })
}

/// Counts for the 31 days ending today, oldest first.
fn last_month(by_day: &BTreeMap<i32, u32>) -> Vec<u32> {
    (-30..=0)
        .map(|day| by_day.get(&day).copied().unwrap_or(0))
        .collect()
}

/// The median of a histogram of cards per value, taking the mean of the middle two
/// like the desktop does.
fn median(histogram: &HashMap<u32, u32>) -> u32 {
    let total: u64 = histogram.values().map(|&n| u64::from(n)).sum();
    if total == 0 {
        return 0;
    }
    let mut sorted: Vec<(u32, u32)> = histogram.iter().map(|(&value, &n)| (value, n)).collect();
    sorted.sort_unstable();
    let at = |position: u64| {
        let mut seen = 0;
        for &(value, n) in &sorted {
            seen += u64::from(n);
            if seen > position {
                return value;
            }
        }
        sorted[sorted.len() - 1].0
    };
    let (lower, upper) = (at((total - 1) / 2), at(total / 2));
    ((f64::from(lower) + f64::from(upper)) / 2.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_median_weighs_values_by_their_counts() {
        assert_eq!(median(&HashMap::new()), 0);
        assert_eq!(median(&HashMap::from([(5, 1)])), 5);
        assert_eq!(median(&HashMap::from([(1, 3), (10, 1)])), 1);
        assert_eq!(
            median(&HashMap::from([(1, 1), (10, 1)])),
            6,
            "rounded up from 5.5"
        );
        assert_eq!(median(&HashMap::from([(1, 2), (3, 2), (100, 1)])), 3);
    }
}
