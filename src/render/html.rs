//! A small HTML-to-styled-lines converter for card content. Cards are simple HTML
//! (formatting, line breaks, lists, images, cloze spans), so a hand-written tokenizer
//! covers what matters without pulling in a full parser.

use anki::template::RenderedNode;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Joins rslib's rendered nodes into HTML. Filters rslib leaves to the frontend get
/// terminal equivalents: a type-answer field is hidden on the question side, hints are
/// collapsed there, and text-to-speech is dropped.
pub fn nodes_to_html(nodes: &[RenderedNode], answer_side: bool) -> String {
    let mut html = String::new();
    for node in nodes {
        match node {
            RenderedNode::Text { text } => html.push_str(text),
            RenderedNode::Replacement {
                field_name,
                current_text,
                filters,
            } => {
                let filter = filters.last().map(String::as_str).unwrap_or_default();
                match (filter, answer_side) {
                    ("type", false) => html.push_str("<i>[type the answer]</i>"),
                    ("hint", false) => html.push_str(&format!("<i>[hint: {field_name}]</i>")),
                    (name, _) if name.starts_with("tts") => {}
                    _ => html.push_str(current_text),
                }
            }
        }
    }
    html
}

/// Renders HTML to lines with inline styling. Whitespace collapses like a browser's,
/// block elements end lines without adding blank ones, and `<br>` runs never produce
/// more than one blank line.
pub fn html_to_lines(html: &str) -> Vec<Line<'static>> {
    let mut out = Builder::default();
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('<') {
            let Some(end) = find_tag_end(after) else {
                out.text("<");
                rest = after;
                continue;
            };
            let tag = &after[..end];
            rest = &after[end + 1..];
            if let Some(skip_to) = out.tag(tag) {
                rest = skip_past(rest, skip_to);
            }
        } else if let Some(after) = rest.strip_prefix('&') {
            let (text, consumed) = decode_entity(after);
            out.text(&text);
            rest = &after[consumed..];
        } else if let Some(after) = rest.strip_prefix("[sound:") {
            let end = after.find(']').unwrap_or(after.len());
            out.styled(
                &format!("[audio: {}]", &after[..end]),
                Style::new().fg(Color::Cyan),
            );
            rest = after.get(end + 1..).unwrap_or("");
        } else {
            let end = rest.find(['<', '&', '[']).unwrap_or(rest.len());
            let end = if end == 0 { 1 } else { end };
            out.text(&rest[..end]);
            rest = &rest[end..];
        }
    }
    out.finish()
}

/// Position of the closing `>`, respecting quoted attribute values.
fn find_tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn skip_past<'a>(rest: &'a str, closing: &str) -> &'a str {
    let lower = rest.to_ascii_lowercase();
    match lower.find(closing) {
        Some(pos) => &rest[pos + closing.len()..],
        None => "",
    }
}

fn decode_entity(s: &str) -> (String, usize) {
    let Some(end) = s.find(';').filter(|&end| end <= 10) else {
        return ("&".to_string(), 0);
    };
    let name = &s[..end];
    let decoded = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => name
            .strip_prefix("#x")
            .or_else(|| name.strip_prefix("#X"))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
            .and_then(char::from_u32),
    };
    match decoded {
        Some(c) => (c.to_string(), end + 1),
        None => ("&".to_string(), 0),
    }
}

#[derive(Default)]
struct Builder {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    style: Style,
    stack: Vec<Style>,
    /// Pending whitespace collapses to one space, and never starts a line.
    space_pending: bool,
    list_depth: usize,
}

impl Builder {
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                self.space_pending = true;
            } else {
                self.flush_space();
                let c = if c == '\u{a0}' { ' ' } else { c };
                self.push(&c.to_string());
            }
        }
    }

    /// Emits a collapsed space in the style that was active when it was seen.
    fn flush_space(&mut self) {
        if self.space_pending && !self.current.is_empty() {
            self.push(" ");
        }
        self.space_pending = false;
    }

    fn styled(&mut self, text: &str, style: Style) {
        self.flush_space();
        let saved = self.style;
        self.style = style;
        self.text(text);
        self.style = saved;
    }

    fn push(&mut self, text: &str) {
        match self.current.last_mut() {
            Some(last) if last.style == self.style => last.content.to_mut().push_str(text),
            _ => self
                .current
                .push(Span::styled(text.to_string(), self.style)),
        }
    }

    /// Handles a tag; returns a closing tag whose content should be skipped entirely.
    fn tag(&mut self, tag: &str) -> Option<&'static str> {
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/').trim_end_matches('/');
        let name = body
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match (name.as_str(), closing) {
            ("style", false) => return Some("</style>"),
            ("script", false) => return Some("</script>"),
            ("br", _) => self.newline(),
            ("hr", false) => self.rule(),
            ("p" | "div" | "tr" | "table" | "blockquote", _) => self.block_break(),
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                self.block_break();
                self.push_style(Style::new().add_modifier(Modifier::BOLD));
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => {
                self.pop_style();
                self.block_break();
            }
            ("ul" | "ol", false) => {
                self.list_depth += 1;
                self.block_break();
            }
            ("ul" | "ol", true) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                self.block_break();
            }
            ("li", false) => {
                self.block_break();
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                self.push(&format!("{indent}• "));
            }
            ("li", true) => self.block_break(),
            ("td" | "th", true) => self.text(" "),
            ("b" | "strong", false) => self.push_style(Style::new().add_modifier(Modifier::BOLD)),
            ("i" | "em", false) => self.push_style(Style::new().add_modifier(Modifier::ITALIC)),
            ("u", false) => self.push_style(Style::new().add_modifier(Modifier::UNDERLINED)),
            ("s" | "del" | "strike", false) => {
                self.push_style(Style::new().add_modifier(Modifier::CROSSED_OUT))
            }
            ("code" | "pre" | "kbd", false) => self.push_style(Style::new().fg(Color::Yellow)),
            ("span" | "font" | "a", false) => {
                let is_cloze = attr(body, "class")
                    .is_some_and(|class| class.split_whitespace().any(|c| c == "cloze"));
                let style = if is_cloze {
                    Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                };
                self.push_style(style);
            }
            (
                "b" | "strong" | "i" | "em" | "u" | "s" | "del" | "strike" | "code" | "pre" | "kbd"
                | "span" | "font" | "a",
                true,
            ) => self.pop_style(),
            ("img", false) => {
                let name = attr(body, "src").unwrap_or_default();
                self.styled(&format!("[image: {name}]"), Style::new().fg(Color::Cyan));
            }
            _ => {}
        }
        None
    }

    fn push_style(&mut self, extra: Style) {
        self.flush_space();
        self.stack.push(self.style);
        self.style = self.style.patch(extra);
    }

    fn pop_style(&mut self) {
        self.flush_space();
        if let Some(style) = self.stack.pop() {
            self.style = style;
        }
    }

    /// A block boundary ends the current line but, unlike `<br>`, never adds a blank one.
    fn block_break(&mut self) {
        if self.current.is_empty() {
            self.space_pending = false;
        } else {
            self.newline();
        }
    }

    fn newline(&mut self) {
        self.space_pending = false;
        let line = Line::from(std::mem::take(&mut self.current));
        let blank = line.width() == 0;
        let last_blank = self.lines.last().is_some_and(|l| l.width() == 0);
        if !(blank && (last_blank || self.lines.is_empty())) {
            self.lines.push(line);
        }
    }

    fn rule(&mut self) {
        self.block_break();
        self.lines.push(Line::from(Span::styled(
            "──────────",
            Style::new().fg(Color::DarkGray),
        )));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.newline();
        while self.lines.last().is_some_and(|l| l.width() == 0) {
            self.lines.pop();
        }
        self.lines
    }
}

/// Value of an attribute inside a tag body, quotes stripped.
fn attr<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let lower = body.to_ascii_lowercase();
    let mut search = 0;
    while let Some(pos) = lower[search..].find(name) {
        let start = search + pos;
        let after = &body[start + name.len()..];
        let boundary_ok = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric();
        if boundary_ok && after.trim_start().starts_with('=') {
            let value = after.trim_start()[1..].trim_start();
            return Some(match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
                _ => value.split_whitespace().next().unwrap_or(""),
            });
        }
        search = start + name.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(html: &str) -> Vec<String> {
        html_to_lines(html)
            .iter()
            .map(|line| line.to_string())
            .collect()
    }

    #[test]
    fn breaks_and_blocks_become_lines_and_blank_lines_do_not_stack() {
        assert_eq!(
            plain("Front<br><br><br>Back<div>third</div><div></div><p>fourth</p>"),
            ["Front", "", "Back", "third", "fourth"]
        );
    }

    #[test]
    fn whitespace_collapses_and_entities_decode() {
        assert_eq!(
            plain("  a &amp;  b\n\t&lt;c&gt;&nbsp;d &#8212; &#x41; &unknown;"),
            ["a & b <c> d — A &unknown;"]
        );
    }

    #[test]
    fn inline_styles_apply_and_nest() {
        let lines = html_to_lines("<b>bold <i>both</i></b> plain");
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "bold ");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, "both");
        assert!(
            spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD | Modifier::ITALIC)
        );
        assert_eq!(spans[2].content, " plain");
        assert_eq!(spans[2].style, Style::new());
    }

    #[test]
    fn media_becomes_labels_and_cloze_is_highlighted() {
        let lines = html_to_lines(
            r#"<img src="map.png"> [sound:hello.mp3] <span class="cloze">[...]</span>"#,
        );
        assert_eq!(
            lines[0].to_string(),
            "[image: map.png] [audio: hello.mp3] [...]"
        );
        let cloze = lines[0].spans.last().unwrap();
        assert_eq!(cloze.style.fg, Some(Color::Blue));
    }

    #[test]
    fn styles_scripts_and_the_answer_rule_are_handled() {
        assert_eq!(
            plain("<style>.card { color: red }</style>Q<hr id=answer>A<script>x()</script>"),
            ["Q", "──────────", "A"]
        );
    }

    #[test]
    fn lists_get_bullets() {
        assert_eq!(
            plain("<ul><li>one</li><li>two</li></ul>"),
            ["• one", "• two"]
        );
    }

    #[test]
    fn frontend_filters_get_terminal_equivalents() {
        let nodes = vec![
            RenderedNode::Text {
                text: "Q ".to_string(),
            },
            RenderedNode::Replacement {
                field_name: "Back".to_string(),
                current_text: "secret".to_string(),
                filters: vec!["type".to_string()],
            },
        ];
        assert_eq!(nodes_to_html(&nodes, false), "Q <i>[type the answer]</i>");
        assert_eq!(nodes_to_html(&nodes, true), "Q secret");
    }
}
