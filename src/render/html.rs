//! A small HTML-to-styled-lines converter for card content. Cards are simple HTML
//! (formatting, line breaks, lists, images, cloze spans), so a hand-written tokenizer
//! covers what matters without pulling in a full parser. Styling comes from browser
//! defaults for the common tags, the notetype's stylesheet, and inline `style`.

use std::path::Path;

use anki::template::RenderedNode;
use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::render::css::{Decl, ElementRef, Stylesheet};

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

/// A card is text interrupted by images; each image is a block of its own so the
/// screen can place a picture where the `<img>` sat.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(Vec<Line<'static>>),
    Image {
        src: String,
        align: Option<Alignment>,
    },
}

/// Text-only view of the card: images become `[image: name]` labels on their own line.
pub fn html_to_lines(html: &str, sheet: &Stylesheet<'_>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in html_to_blocks(html, sheet) {
        match block {
            Block::Text(text) => lines.extend(text),
            Block::Image { src, align } => {
                let mut line = Line::from(Span::styled(
                    image_label(&src),
                    Style::new().fg(Color::Cyan),
                ));
                if let Some(align) = align {
                    line = line.alignment(align);
                }
                lines.push(line);
            }
        }
    }
    lines
}

pub fn image_label(src: &str) -> String {
    let name = Path::new(src)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(src);
    format!("[image: {name}]")
}

/// Renders HTML to text blocks with inline styling and per-line alignment, split by
/// images. Whitespace collapses like a browser's, block elements end lines without
/// adding blank ones, and `<br>` runs never produce more than one blank line.
pub fn html_to_blocks(html: &str, sheet: &Stylesheet<'_>) -> Vec<Block> {
    let mut out = Builder::new(sheet);
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
            out.label(&format!("[audio: {}]", &after[..end]));
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

const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "tr",
    "table",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "section",
    "article",
    "header",
    "footer",
];

/// An open element: what the stylesheet needs to match it, and what text inside it
/// looks like.
struct Element {
    tag: String,
    attrs: Vec<(String, String)>,
    style: Style,
    /// Own `text-align`; None inherits from the enclosing block.
    align: Option<Alignment>,
    hidden: bool,
}

struct Builder<'s, 'c> {
    sheet: &'s Stylesheet<'c>,
    blocks: Vec<Block>,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    elements: Vec<Element>,
    /// Pending whitespace collapses to one space, and never starts a line.
    space_pending: bool,
    list_depth: usize,
}

impl<'s, 'c> Builder<'s, 'c> {
    fn new(sheet: &'s Stylesheet<'c>) -> Self {
        let mut builder = Self {
            sheet,
            blocks: Vec::new(),
            lines: Vec::new(),
            current: Vec::new(),
            elements: Vec::new(),
            space_pending: false,
            list_depth: 0,
        };
        // Anki wraps every card in `<div class="card">`. Its alignment and colours
        // assume a white page and the screen already centers, so only text styling
        // (weight, size) is taken from it.
        builder.open("div", "class=\"card\"");
        if let Some(root) = builder.elements.first_mut() {
            root.align = None;
            root.style.fg = None;
        }
        builder
    }

    fn style(&self) -> Style {
        self.elements.last().map(|e| e.style).unwrap_or_default()
    }

    fn hidden(&self) -> bool {
        self.elements.last().is_some_and(|e| e.hidden)
    }

    fn alignment(&self) -> Option<Alignment> {
        self.elements.iter().rev().find_map(|e| e.align)
    }

    fn text(&mut self, text: &str) {
        if self.hidden() {
            return;
        }
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

    /// Media placeholders, in a colour of their own.
    fn label(&mut self, text: &str) {
        self.open("span", "style=\"color: cyan\"");
        self.text(text);
        self.close("span");
    }

    fn push(&mut self, text: &str) {
        let style = self.style();
        match self.current.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push_str(text),
            _ => self.current.push(Span::styled(text.to_string(), style)),
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
            ("img", false) => {
                let attrs = attrs(body);
                let src = attrs
                    .iter()
                    .find(|(name, _)| name == "src")
                    .map(|(_, value)| value.as_str())
                    .unwrap_or_default();
                if !self.hidden() && !src.is_empty() {
                    self.image(src);
                }
            }
            ("td" | "th", true) => self.text(" "),
            (_, false) => self.open(&name, body),
            (_, true) => self.close(&name),
        }
        None
    }

    fn open(&mut self, tag: &str, body: &str) {
        // Whitespace before a block vanishes; before an inline element it belongs to
        // the enclosing style.
        if BLOCK_TAGS.contains(&tag) {
            self.block_break();
        } else {
            self.flush_space();
        }
        let attrs = attrs(body);
        let classes: Vec<&str> = attrs
            .iter()
            .find(|(name, _)| name == "class")
            .map(|(_, value)| value.split_whitespace().collect())
            .unwrap_or_default();

        let builtin = builtin_decl(tag, &classes);
        let sheet_decl = {
            let chain: Vec<ElementRef<'_>> = self
                .elements
                .iter()
                .map(|e| ElementRef {
                    tag: &e.tag,
                    attrs: &e.attrs,
                })
                .chain(std::iter::once(ElementRef { tag, attrs: &attrs }))
                .collect();
            self.sheet.declaration_for(&chain)
        };
        let inline = attrs
            .iter()
            .find(|(name, _)| name == "style")
            .map(|(_, value)| Decl::parse(value))
            .unwrap_or_default();

        let parent = self.elements.last();
        let mut style = parent.map(|e| e.style).unwrap_or_default();
        for decl in [&builtin, &sheet_decl, &inline] {
            style = decl.apply(style);
        }
        let hidden = parent.is_some_and(|e| e.hidden) || sheet_decl.hidden || inline.hidden;
        let align = inline.align.or(sheet_decl.align);

        if tag == "ul" || tag == "ol" {
            self.list_depth += 1;
        }
        self.elements.push(Element {
            tag: tag.to_string(),
            attrs,
            style,
            align,
            hidden,
        });
        if tag == "li" {
            let indent = "  ".repeat(self.list_depth.saturating_sub(1));
            self.push(&format!("{indent}• "));
        }
    }

    /// Closes the nearest open element with this name, tolerating unbalanced markup.
    fn close(&mut self, tag: &str) {
        let Some(pos) = self.elements.iter().rposition(|e| e.tag == tag) else {
            return;
        };
        if pos == 0 {
            return;
        }
        if BLOCK_TAGS.contains(&tag) {
            self.block_break();
        } else {
            self.flush_space();
        }
        while self.elements.len() > pos {
            let closed = self.elements.pop().expect("element to close");
            if closed.tag == "ul" || closed.tag == "ol" {
                self.list_depth = self.list_depth.saturating_sub(1);
            }
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
        let mut line = Line::from(std::mem::take(&mut self.current));
        if let Some(alignment) = self.alignment() {
            line = line.alignment(alignment);
        }
        let blank = line.width() == 0;
        let last_blank = self.lines.last().is_some_and(|l| l.width() == 0);
        if !(blank && (last_blank || self.lines.is_empty())) {
            self.lines.push(line);
        }
    }

    /// Closes the text so far and records the image as a block of its own.
    fn image(&mut self, src: &str) {
        self.block_break();
        self.flush_text();
        self.blocks.push(Block::Image {
            src: src.to_string(),
            align: self.alignment(),
        });
    }

    fn flush_text(&mut self) {
        while self.lines.last().is_some_and(|l| l.width() == 0) {
            self.lines.pop();
        }
        if !self.lines.is_empty() {
            self.blocks
                .push(Block::Text(std::mem::take(&mut self.lines)));
        }
    }

    fn rule(&mut self) {
        self.block_break();
        self.lines.push(Line::from(Span::styled(
            "──────────",
            Style::new().fg(Color::DarkGray),
        )));
    }

    fn finish(mut self) -> Vec<Block> {
        self.newline();
        self.flush_text();
        self.blocks
    }
}

/// What a browser does with a tag before any stylesheet is involved.
fn builtin_decl(tag: &str, classes: &[&str]) -> Decl {
    let mut decl = Decl::default();
    match tag {
        "b" | "strong" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => decl.add |= Modifier::BOLD,
        "i" | "em" => decl.add |= Modifier::ITALIC,
        "u" => decl.add |= Modifier::UNDERLINED,
        "s" | "del" | "strike" => decl.add |= Modifier::CROSSED_OUT,
        "code" | "pre" | "kbd" => decl.fg = Some(Color::Yellow),
        "a" => {
            decl.fg = Some(Color::Blue);
            decl.add |= Modifier::UNDERLINED;
        }
        "small" | "sub" | "sup" => decl.add |= Modifier::DIM,
        _ => {}
    }
    if classes.contains(&"cloze") {
        decl.fg = Some(Color::Blue);
        decl.add |= Modifier::BOLD;
    }
    decl
}

/// Every attribute inside a tag body as lowercased name and unquoted value; bare
/// attributes get an empty value.
fn attrs(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = body
        .trim_start()
        .trim_start_matches(|c: char| !c.is_whitespace())
        .trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let name = rest[..name_end].trim_end_matches('/').to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let consumed = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &after[1..];
                    let end = inner.find(quote).unwrap_or(inner.len());
                    value = inner[..end].to_string();
                    (end + 2).min(after.len())
                }
                _ => {
                    let end = after
                        .find(|c: char| c.is_whitespace())
                        .unwrap_or(after.len());
                    value = after[..end].to_string();
                    end
                }
            };
            rest = after[consumed..].trim_start();
        }
        if !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(html: &str) -> Vec<String> {
        html_to_lines(html, &Stylesheet::default())
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
        let lines = html_to_lines("<b>bold <i>both</i></b> plain", &Stylesheet::default());
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
        assert_eq!(spans[2].style.add_modifier, Modifier::empty());
    }

    #[test]
    fn media_becomes_labels_and_cloze_is_highlighted() {
        let lines = html_to_lines(
            r#"<img src="/abs/path/map.png"> [sound:hello.mp3] <span class="cloze">[...]</span>"#,
            &Stylesheet::default(),
        );
        assert_eq!(lines[0].to_string(), "[image: map.png]");
        assert_eq!(lines[1].to_string(), "[audio: hello.mp3] [...]");
        let cloze = lines[1].spans.last().unwrap();
        assert_eq!(cloze.style.fg, Some(Color::Blue));
    }

    #[test]
    fn images_split_the_text_into_blocks_and_keep_their_alignment() {
        let sheet = Stylesheet::parse(".right { text-align: right }");
        let blocks = html_to_blocks(
            "Question<div class=\"right\"><img src=\"map.png\"></div>Answer<img src=\"flag.svg\">",
            &sheet,
        );
        assert_eq!(blocks.len(), 4);
        assert!(matches!(&blocks[0], Block::Text(lines) if lines[0].to_string() == "Question"));
        assert_eq!(
            blocks[1],
            Block::Image {
                src: "map.png".to_string(),
                align: Some(Alignment::Right)
            }
        );
        assert!(matches!(&blocks[2], Block::Text(lines) if lines[0].to_string() == "Answer"));
        assert_eq!(
            blocks[3],
            Block::Image {
                src: "flag.svg".to_string(),
                align: None
            }
        );
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
    fn stylesheet_aligns_shrinks_and_restyles_blocks() {
        let sheet = Stylesheet::parse(
            ".card { text-align: left; color: black; font-size: 20px }\n\
             .source { font-size: 50%; text-align: right }\n\
             .source a { text-decoration: none }",
        );
        let lines = html_to_lines(
            "Jette<div class='source'><a href='x'>Julius</a></div>",
            &sheet,
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0].to_string(),
            "Jette",
            "no stray space before the block"
        );
        assert_eq!(lines[0].alignment, None, "card-level alignment is ignored");
        assert_eq!(
            lines[0].spans[0].style.fg, None,
            "card-level colour is ignored"
        );
        let source = &lines[1];
        assert_eq!(source.alignment, Some(Alignment::Right));
        let link = &source.spans[0];
        assert_eq!(link.content, "Julius");
        assert_eq!(link.style.fg, Some(Color::Blue), "browser link colour");
        assert!(link.style.add_modifier.contains(Modifier::DIM));
        assert!(!link.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn hidden_elements_and_inline_styles_are_honoured() {
        let sheet = Stylesheet::parse(".secret { display: none }");
        let lines = html_to_lines(
            "a<span class=\"secret\">b</span>c<div style=\"text-align: right; font-weight: bold\">d</div>",
            &sheet,
        );
        assert_eq!(lines[0].to_string(), "ac");
        assert_eq!(lines[1].to_string(), "d");
        assert_eq!(lines[1].alignment, Some(Alignment::Right));
        assert!(
            lines[1].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn attributes_parse_quoted_unquoted_and_bare() {
        let parsed = attrs("div class='a b' ID=main hidden data-x=\"1\"/");
        assert_eq!(
            parsed,
            [
                ("class".to_string(), "a b".to_string()),
                ("id".to_string(), "main".to_string()),
                ("hidden".to_string(), String::new()),
                ("data-x".to_string(), "1".to_string()),
            ]
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
