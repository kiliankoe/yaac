//! Just enough CSS to honour what card templates typically do: align a block, shrink or
//! embolden text, colour it, or hide it. Anything else is ignored.

use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};

/// The terminal-relevant part of a declaration block.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Decl {
    pub fg: Option<Color>,
    pub add: Modifier,
    pub remove: Modifier,
    pub align: Option<Alignment>,
    pub hidden: bool,
}

impl Decl {
    /// Applies this declaration on top of an inherited style.
    pub fn apply(&self, style: Style) -> Style {
        let mut style = style.remove_modifier(self.remove).add_modifier(self.add);
        if let Some(fg) = self.fg {
            style = style.fg(fg);
        }
        style
    }

    fn merge(&mut self, other: &Decl) {
        if other.fg.is_some() {
            self.fg = other.fg;
        }
        self.add = (self.add - other.remove) | other.add;
        self.remove = (self.remove - other.add) | other.remove;
        if other.align.is_some() {
            self.align = other.align;
        }
        self.hidden |= other.hidden;
    }

    /// Parses `prop: value; prop: value` as found in rule bodies and `style` attributes.
    pub fn parse(body: &str) -> Decl {
        let mut decl = Decl::default();
        for item in body.split(';') {
            let Some((prop, value)) = item.split_once(':') else {
                continue;
            };
            let prop = prop.trim().to_ascii_lowercase();
            let value = value
                .trim()
                .trim_end_matches("!important")
                .trim()
                .to_ascii_lowercase();
            match prop.as_str() {
                "color" => match terminal_color(&value) {
                    Some(Tone::Color(color)) => decl.fg = Some(color),
                    Some(Tone::Faint) => decl.add |= Modifier::DIM,
                    None => {}
                },
                "font-weight" => {
                    let bold = value == "bold"
                        || value == "bolder"
                        || value.parse::<u32>().is_ok_and(|w| w >= 600);
                    if bold {
                        decl.add |= Modifier::BOLD;
                    } else if value == "normal" || value == "lighter" {
                        decl.remove |= Modifier::BOLD;
                    }
                }
                "font-style" => {
                    if value == "italic" || value == "oblique" {
                        decl.add |= Modifier::ITALIC;
                    } else if value == "normal" {
                        decl.remove |= Modifier::ITALIC;
                    }
                }
                "text-decoration" | "text-decoration-line" => {
                    if value.contains("underline") {
                        decl.add |= Modifier::UNDERLINED;
                    } else if value.contains("line-through") {
                        decl.add |= Modifier::CROSSED_OUT;
                    } else if value == "none" {
                        decl.remove |= Modifier::UNDERLINED | Modifier::CROSSED_OUT;
                    }
                }
                "font-size" => match size_class(&value) {
                    Some(Size::Small) => decl.add |= Modifier::DIM,
                    Some(Size::Large) => decl.add |= Modifier::BOLD,
                    None => {}
                },
                "text-align" => {
                    decl.align = match value.as_str() {
                        "left" | "start" => Some(Alignment::Left),
                        "right" | "end" => Some(Alignment::Right),
                        "center" => Some(Alignment::Center),
                        _ => None,
                    }
                }
                "display" if value == "none" => decl.hidden = true,
                "visibility" if value == "hidden" => decl.hidden = true,
                _ => {}
            }
        }
        decl
    }
}

enum Size {
    Small,
    Large,
}

/// Smaller or larger than the usual card text; the exact number does not matter.
fn size_class(value: &str) -> Option<Size> {
    match value {
        "xx-small" | "x-small" | "small" | "smaller" => return Some(Size::Small),
        "large" | "x-large" | "xx-large" | "larger" => return Some(Size::Large),
        _ => {}
    }
    let number: f32 = value
        .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%')
        .parse()
        .ok()?;
    let unit = value.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    let (small, large) = match unit {
        "%" => (85.0, 130.0),
        "px" => (14.0, 26.0),
        "pt" => (10.0, 20.0),
        "em" | "rem" => (0.85, 1.3),
        _ => return None,
    };
    if number <= small {
        Some(Size::Small)
    } else if number >= large {
        Some(Size::Large)
    } else {
        None
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Tone {
    Color(Color),
    /// Light greys: readable as dim text on either background.
    Faint,
}

/// Maps a CSS colour onto the terminal's own palette so it follows the user's theme.
/// Saturated colours pick the nearest ANSI hue; light greys become dim; dark greys,
/// black, and white are left to the theme because a card assumes a white page.
pub fn terminal_color(value: &str) -> Option<Tone> {
    let (r, g, b) = rgb(value)?;
    let max = r.max(g).max(b) as i32;
    let min = r.min(g).min(b) as i32;
    if max - min < 40 {
        return if (140..=230).contains(&max) {
            Some(Tone::Faint)
        } else {
            None
        };
    }
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let (max, min) = (max as f32, min as f32);
    let delta = max - min;
    let hue = if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    Some(Tone::Color(match hue as u32 {
        0..=20 | 340..=360 => Color::Red,
        21..=70 => Color::Yellow,
        71..=160 => Color::Green,
        161..=200 => Color::Cyan,
        201..=265 => Color::Blue,
        _ => Color::Magenta,
    }))
}

fn rgb(value: &str) -> Option<(u8, u8, u8)> {
    if let Some(hex) = value.strip_prefix('#') {
        let hex = hex.trim();
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()?;
        return match digits.len() {
            3 | 4 => Some((digits[0] * 17, digits[1] * 17, digits[2] * 17)),
            6 | 8 => Some((
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
            )),
            _ => None,
        };
    }
    if let Some(inner) = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
    {
        let parts: Vec<u8> = inner
            .trim_end_matches(')')
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .take(3)
            .map(|p| {
                p.trim()
                    .trim_end_matches('%')
                    .parse::<f32>()
                    .ok()
                    .map(|n| n as u8)
            })
            .collect::<Option<_>>()?;
        return (parts.len() == 3).then(|| (parts[0], parts[1], parts[2]));
    }
    Some(match value {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" | "crimson" | "firebrick" | "tomato" | "darkred" => (220, 20, 60),
        "green" | "darkgreen" | "forestgreen" | "seagreen" | "limegreen" | "lime" => (0, 128, 0),
        "blue" | "navy" | "royalblue" | "dodgerblue" | "steelblue" | "mediumblue" => (0, 0, 255),
        "yellow" | "gold" | "goldenrod" | "khaki" => (255, 215, 0),
        "orange" | "darkorange" | "coral" | "orangered" => (255, 140, 0),
        "purple" | "violet" | "indigo" | "blueviolet" | "orchid" | "plum" => (128, 0, 128),
        "pink" | "hotpink" | "deeppink" | "magenta" | "fuchsia" => (255, 20, 147),
        "cyan" | "aqua" | "teal" | "turquoise" | "darkcyan" => (0, 180, 200),
        "brown" | "maroon" | "chocolate" | "sienna" => (150, 60, 30),
        "gray" | "grey" | "darkgray" | "darkgrey" | "dimgray" | "dimgrey" => (128, 128, 128),
        "lightgray" | "lightgrey" | "silver" | "gainsboro" => (200, 200, 200),
        _ => return None,
    })
}

/// One compound selector such as `div.source#main`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Compound {
    tag: Option<String>,
    classes: Vec<String>,
    id: Option<String>,
}

impl Compound {
    fn parse(text: &str) -> Option<Self> {
        let mut compound = Compound::default();
        let mut rest = text;
        while !rest.is_empty() {
            let kind = rest.chars().next()?;
            let body = if matches!(kind, '.' | '#') {
                &rest[1..]
            } else {
                rest
            };
            let end = body.find(['.', '#', ':', '[']).unwrap_or(body.len());
            let name = body[..end].to_ascii_lowercase();
            if name.is_empty() {
                return None;
            }
            match kind {
                '.' => compound.classes.push(name),
                '#' => compound.id = Some(name),
                _ if name == "*" => {}
                _ => compound.tag = Some(name),
            }
            let consumed = if matches!(kind, '.' | '#') {
                end + 1
            } else {
                end
            };
            rest = &rest[consumed..];
            // Pseudo-classes and attribute selectors have no terminal meaning; skip them.
            if let Some(stripped) = rest.strip_prefix(':') {
                let end = stripped.find(['.', '#', '[']).unwrap_or(stripped.len());
                rest = &stripped[end..];
            }
            if let Some(stripped) = rest.strip_prefix('[') {
                let end = stripped.find(']').map(|e| e + 1).unwrap_or(stripped.len());
                rest = &stripped[end..];
            }
        }
        Some(compound)
    }

    fn matches(&self, element: &ElementRef<'_>) -> bool {
        self.tag.as_deref().is_none_or(|tag| tag == element.tag)
            && self.id.as_deref().is_none_or(|id| Some(id) == element.id)
            && self
                .classes
                .iter()
                .all(|class| element.classes.contains(&class.as_str()))
    }

    fn specificity(&self) -> u32 {
        u32::from(self.id.is_some()) * 100
            + self.classes.len() as u32 * 10
            + u32::from(self.tag.is_some())
    }
}

/// What a stylesheet needs to know about an element and its ancestors.
#[derive(Debug, Clone, Copy)]
pub struct ElementRef<'a> {
    pub tag: &'a str,
    pub classes: &'a [&'a str],
    pub id: Option<&'a str>,
}

#[derive(Debug)]
struct Rule {
    /// Outermost first; the last compound must match the element itself.
    path: Vec<Compound>,
    decl: Decl,
    specificity: u32,
}

#[derive(Debug, Default)]
pub struct Stylesheet {
    rules: Vec<Rule>,
}

impl Stylesheet {
    pub fn parse(css: &str) -> Self {
        let mut rules = Vec::new();
        let mut rest = strip_comments(css);
        let mut text = rest.as_str();
        while let Some(open) = text.find('{') {
            let selectors = text[..open].trim();
            let after = &text[open + 1..];
            if selectors.starts_with('@') {
                // Skip at-rules (@font-face, @media ...) together with their block.
                text = skip_block(after);
                continue;
            }
            let Some(close) = after.find('}') else { break };
            let decl = Decl::parse(&after[..close]);
            for selector in selectors.split(',') {
                let path: Option<Vec<Compound>> = selector
                    .split_whitespace()
                    .filter(|part| *part != ">")
                    .map(Compound::parse)
                    .collect();
                if let Some(path) = path.filter(|p| !p.is_empty()) {
                    let specificity = path.iter().map(Compound::specificity).sum();
                    rules.push(Rule {
                        path,
                        decl,
                        specificity,
                    });
                }
            }
            text = &after[close + 1..];
        }
        rest.clear();
        // Stable sort keeps source order among equals, so later rules win like in CSS.
        rules.sort_by_key(|rule| rule.specificity);
        Self { rules }
    }

    /// Combined declaration for the last element in `chain` (outermost first).
    pub fn declaration_for(&self, chain: &[ElementRef<'_>]) -> Decl {
        let mut decl = Decl::default();
        let Some(element) = chain.last() else {
            return decl;
        };
        for rule in &self.rules {
            let (last, ancestors) = rule.path.split_last().expect("non-empty path");
            if last.matches(element) && ancestors_match(ancestors, &chain[..chain.len() - 1]) {
                decl.merge(&rule.decl);
            }
        }
        decl
    }
}

/// Descendant matching: every ancestor compound must match some ancestor, in order.
fn ancestors_match(compounds: &[Compound], ancestors: &[ElementRef<'_>]) -> bool {
    let mut remaining = compounds;
    for ancestor in ancestors {
        if let Some((first, rest)) = remaining.split_first()
            && first.matches(ancestor)
        {
            remaining = rest;
        }
    }
    remaining.is_empty()
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = match rest[start + 2..].find("*/") {
            Some(end) => &rest[start + 2 + end + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn skip_block(text: &str) -> &str {
    let mut depth = 1usize;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[i + 1..];
                }
            }
            _ => {}
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el<'a>(tag: &'a str, classes: &'a [&'a str], id: Option<&'a str>) -> ElementRef<'a> {
        ElementRef { tag, classes, id }
    }

    #[test]
    fn parses_alignment_size_and_decoration() {
        let sheet = Stylesheet::parse(
            "/* comment */ .source { padding-top: 20px; font-size: 50%; text-align: right }\n\
             .source a { text-decoration: none }",
        );
        let source = sheet.declaration_for(&[el("div", &["source"], None)]);
        assert_eq!(source.align, Some(Alignment::Right));
        assert!(source.add.contains(Modifier::DIM));
        let link = sheet.declaration_for(&[el("div", &["source"], None), el("a", &[], None)]);
        assert!(link.remove.contains(Modifier::UNDERLINED));
        let other_link = sheet.declaration_for(&[el("div", &[], None), el("a", &[], None)]);
        assert_eq!(other_link, Decl::default());
    }

    #[test]
    fn later_and_more_specific_rules_win() {
        let sheet = Stylesheet::parse("b { color: red } .x { color: blue } b { color: green }");
        let plain = sheet.declaration_for(&[el("b", &[], None)]);
        assert_eq!(plain.fg, Some(Color::Green));
        let classed = sheet.declaration_for(&[el("b", &["x"], None)]);
        assert_eq!(classed.fg, Some(Color::Blue));
    }

    #[test]
    fn colours_follow_the_terminal_palette() {
        assert_eq!(terminal_color("#1a73e8"), Some(Tone::Color(Color::Blue)));
        assert_eq!(
            terminal_color("rgb(200, 30, 30)"),
            Some(Tone::Color(Color::Red))
        );
        assert_eq!(terminal_color("orange"), Some(Tone::Color(Color::Yellow)));
        assert_eq!(terminal_color("#bbbbbb"), Some(Tone::Faint));
        assert_eq!(terminal_color("black"), None);
        assert_eq!(terminal_color("white"), None);
        assert_eq!(terminal_color("#333"), None);
    }

    #[test]
    fn at_rules_and_pseudo_classes_are_skipped() {
        let sheet = Stylesheet::parse(
            "@font-face { font-family: x; src: url(y) } a:hover { color: red } \
             @media (max-width: 600px) { .card { display: none } } .hidden { display: none }",
        );
        let link = sheet.declaration_for(&[el("a", &[], None)]);
        assert_eq!(link.fg, Some(Color::Red));
        assert!(!sheet.declaration_for(&[el("div", &["card"], None)]).hidden);
        assert!(
            sheet
                .declaration_for(&[el("div", &["hidden"], None)])
                .hidden
        );
    }

    #[test]
    fn inline_declarations_parse_the_same_way() {
        let decl = Decl::parse("font-weight: bold; text-align: center; font-size: 12px");
        assert!(decl.add.contains(Modifier::BOLD | Modifier::DIM));
        assert_eq!(decl.align, Some(Alignment::Center));
    }
}
