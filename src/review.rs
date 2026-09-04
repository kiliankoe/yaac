//! The review loop over rslib's scheduler queue, independent of any UI. The TUI drives
//! it with key presses; tests drive it directly.

use std::time::{Duration, Instant};

use anki::card::CardId;
use anki::collection::Collection;
use anki::decks::DeckId;
use anki::error::AnkiError;
use anki::notes::NoteId;
use anki::ops::Op;
use anki::scheduler::answering::{CardAnswer, Rating};
use anki::scheduler::states::SchedulingStates;
use anki::timestamp::TimestampMillis;
use anki_proto::cards::Card as CardProto;
use anki_proto::scheduler::bury_or_suspend_cards_request::Mode as BuryOrSuspendMode;
use anyhow::{Result, anyhow};

use crate::editor::{self, Editor, Outcome};
use crate::notes;
use crate::render::html::nodes_to_html;
use crate::session::{AnkiResultExt, anki_error};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    New,
    Learning,
    Review,
}

/// The card on screen. Question and answer are HTML as rslib rendered them; the answer
/// side normally repeats the question above an `<hr id=answer>`.
pub struct Current {
    pub card_id: CardId,
    pub note_id: NoteId,
    pub kind: Kind,
    pub question: String,
    pub answer: String,
    /// The notetype's stylesheet, for alignment and text styling.
    pub css: String,
    /// Next-interval descriptions for Again, Hard, Good, Easy, e.g. "<1m", "4d".
    pub labels: [String; 4],
    pub flag: u32,
    /// The note carries Anki's "marked" tag.
    pub marked: bool,
    pub revealed: bool,
    states: SchedulingStates,
    shown_at: Instant,
}

/// Cards still due in the current deck, after daily limits.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    pub new: usize,
    pub learning: usize,
    pub review: usize,
}

pub struct Reviewer<'a> {
    col: &'a mut Collection,
    pub deck: String,
    pub counts: Counts,
    pub current: Option<Current>,
    pub answered: usize,
    started: Instant,
}

impl<'a> Reviewer<'a> {
    /// Makes `deck` the current deck, as clicking it on the desktop would, and fetches
    /// the first card.
    pub fn start(col: &'a mut Collection, deck: DeckId) -> Result<Self> {
        let name = notes::deck_name(col, deck)?;
        col.set_current_deck(deck).ctx("selecting deck")?;
        let mut reviewer = Self {
            col,
            deck: name,
            counts: Counts::default(),
            current: None,
            answered: 0,
            started: Instant::now(),
        };
        reviewer.fetch()?;
        Ok(reviewer)
    }

    fn fetch(&mut self) -> Result<()> {
        let queued = self
            .col
            .get_queued_cards(1, false)
            .ctx("fetching the next card")?;
        self.counts = Counts {
            new: queued.new_count,
            learning: queued.learning_count,
            review: queued.review_count,
        };
        self.current = match queued.cards.into_iter().next() {
            None => None,
            Some(queued) => {
                let card_id = queued.card.id();
                let note_id = queued.card.note_id();
                let marked = notes::is_marked(&notes::get_note(self.col, note_id)?.tags);
                let (question, answer, css) = render(self.col, card_id)?;
                let labels: [String; 4] = self
                    .col
                    .describe_next_states(&queued.states)
                    .ctx("describing answers")?
                    .try_into()
                    .map_err(|_| anyhow!("expected four answer descriptions"))?;
                let proto: CardProto = queued.card.clone().into();
                Some(Current {
                    card_id,
                    note_id,
                    // Queue numbers: 0 new, 1 and 3 learning, 2 review; 4 is preview.
                    kind: match proto.queue {
                        0 => Kind::New,
                        1 | 3 | 4 => Kind::Learning,
                        _ => Kind::Review,
                    },
                    question,
                    answer,
                    css,
                    labels,
                    flag: proto.flags,
                    marked,
                    revealed: false,
                    states: queued.states,
                    shown_at: Instant::now(),
                })
            }
        };
        Ok(())
    }

    pub fn done(&self) -> bool {
        self.current.is_none()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn reveal(&mut self) {
        if let Some(current) = &mut self.current {
            current.revealed = true;
        }
    }

    /// Opens the current card's note in the editor and shows the card again with the
    /// new text. Scheduling, the timer, and the revealed side are untouched. None when
    /// there is no card.
    pub fn edit(&mut self, editor: &Editor) -> Result<Option<Outcome>> {
        let Some(nid) = self.current.as_ref().map(|current| current.note_id) else {
            return Ok(None);
        };
        let outcome = editor::edit_note(self.col, nid, editor)?;
        if outcome == Outcome::Saved {
            self.rerender()?;
        }
        Ok(Some(outcome))
    }

    /// Renders the current card again, after its note changed.
    pub fn rerender(&mut self) -> Result<()> {
        if let Some(current) = &mut self.current {
            let (question, answer, css) = render(self.col, current.card_id)?;
            current.question = question;
            current.answer = answer;
            current.css = css;
        }
        Ok(())
    }

    /// Answers the current card with the state rslib computed for that rating. Time
    /// taken is measured from when the card was shown; rslib caps it per deck options.
    pub fn answer(&mut self, rating: Rating) -> Result<()> {
        let Some(current) = self.current.take() else {
            return Ok(());
        };
        let new_state = match rating {
            Rating::Again => current.states.again,
            Rating::Hard => current.states.hard,
            Rating::Good => current.states.good,
            Rating::Easy => current.states.easy,
        };
        let mut answer = CardAnswer {
            card_id: current.card_id,
            current_state: current.states.current,
            new_state,
            rating,
            answered_at: TimestampMillis::now(),
            milliseconds_taken: current.shown_at.elapsed().as_millis().min(u32::MAX as u128) as u32,
            custom_data: None,
            from_queue: true,
        };
        self.col.answer_card(&mut answer).ctx("answering card")?;
        self.answered += 1;
        self.fetch()
    }

    /// Reverts the last change made in this session; false when there is none.
    pub fn undo(&mut self) -> Result<bool> {
        match self.col.undo() {
            Ok(output) => {
                if output.output.undone_op == Op::AnswerCard {
                    self.answered = self.answered.saturating_sub(1);
                }
                self.fetch()?;
                Ok(true)
            }
            Err(AnkiError::UndoEmpty) => Ok(false),
            Err(err) => Err(anki_error(err)),
        }
    }

    pub fn suspend(&mut self) -> Result<()> {
        self.set_aside(BuryOrSuspendMode::Suspend, "suspending card")
    }

    pub fn bury(&mut self) -> Result<()> {
        self.set_aside(BuryOrSuspendMode::BuryUser, "burying card")
    }

    fn set_aside(&mut self, mode: BuryOrSuspendMode, what: &str) -> Result<()> {
        let Some(current) = self.current.take() else {
            return Ok(());
        };
        self.col
            .bury_or_suspend_cards(&[current.card_id], mode)
            .ctx(what)?;
        self.fetch()
    }

    /// Steps the current card's flag through Anki's seven colours and back to none.
    pub fn cycle_flag(&mut self) -> Result<()> {
        let Some(current) = &mut self.current else {
            return Ok(());
        };
        let next = (current.flag + 1) % 8;
        self.col
            .set_card_flag(&[current.card_id], next)
            .ctx("setting flag")?;
        current.flag = next;
        Ok(())
    }
}

impl Reviewer<'_> {
    /// Adds or removes the "marked" tag on the current card's note, the way the
    /// desktop's mark toggle does.
    pub fn toggle_mark(&mut self) -> Result<()> {
        let Some((note_id, marked)) = self
            .current
            .as_ref()
            .map(|current| (current.note_id, current.marked))
        else {
            return Ok(());
        };
        if marked {
            self.col
                .remove_tags_from_notes(&[note_id], notes::MARKED_TAG)
                .ctx("unmarking note")?;
        } else {
            self.col
                .add_tags_to_notes(&[note_id], notes::MARKED_TAG)
                .ctx("marking note")?;
        }
        if let Some(current) = &mut self.current {
            current.marked = !marked;
        }
        // Templates can show the tags, so the card is rendered again.
        self.rerender()
    }
}

/// Question and answer HTML plus the notetype's stylesheet for a card.
fn render(col: &mut Collection, card_id: CardId) -> Result<(String, String, String)> {
    let rendered = col
        .render_existing_card(card_id, false, false)
        .ctx("rendering card")?;
    Ok((
        nodes_to_html(&rendered.qnodes, false),
        nodes_to_html(&rendered.anodes, true),
        rendered.css,
    ))
}
