//! Card stylesheets, reduced to what a terminal can show. Parsing and selector
//! matching are simplecss's job; this module only interprets the handful of
//! properties with a terminal equivalent: alignment, weight, slant, decoration,
//! size, colour, hiding, and shrink-to-fit boxes.

use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use simplecss::{AttributeOperator, Declaration, DeclarationTokenizer, PseudoClass, Selector};

/// The terminal-relevant part of a declaration block.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Decl {
    pub fg: Option<Color>,
    pub add: Modifier,
    pub remove: Modifier,
    pub align: Option<Alignment>,
    pub hidden: bool,
    /// An inline-level box: as wide as its content instead of as wide as the area.
    pub shrink: bool,
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
        self.shrink |= other.shrink;
    }

    /// Parses `prop: value; prop: value` as found in `style` attributes.
    pub fn parse(body: &str) -> Decl {
        let mut decl = Decl::default();
        for declaration in DeclarationTokenizer::from(body) {
            decl.set(&declaration);
        }
        decl
    }

    fn set(&mut self, declaration: &Declaration<'_>) {
        let value = declaration.value.trim().to_ascii_lowercase();
        match declaration.name.to_ascii_lowercase().as_str() {
            "color" => match terminal_color(&value) {
                Some(Tone::Color(color)) => self.fg = Some(color),
                Some(Tone::Faint) => self.add |= Modifier::DIM,
                None => {}
            },
            "font-weight" => {
                let bold = value == "bold"
                    || value == "bolder"
                    || value.parse::<u32>().is_ok_and(|w| w >= 600);
                if bold {
                    self.add |= Modifier::BOLD;
                } else if value == "normal" || value == "lighter" {
                    self.remove |= Modifier::BOLD;
                }
            }
            "font-style" => {
                if value == "italic" || value == "oblique" {
                    self.add |= Modifier::ITALIC;
                } else if value == "normal" {
                    self.remove |= Modifier::ITALIC;
                }
            }
            "text-decoration" | "text-decoration-line" => {
                if value.contains("underline") {
                    self.add |= Modifier::UNDERLINED;
                } else if value.contains("line-through") {
                    self.add |= Modifier::CROSSED_OUT;
                } else if value == "none" {
                    self.remove |= Modifier::UNDERLINED | Modifier::CROSSED_OUT;
                }
            }
            "font-size" => match size_class(&value) {
                Some(Size::Small) => self.add |= Modifier::DIM,
                Some(Size::Large) => self.add |= Modifier::BOLD,
                None => {}
            },
            "text-align" => {
                self.align = match value.as_str() {
                    "left" | "start" => Some(Alignment::Left),
                    "right" | "end" => Some(Alignment::Right),
                    "center" => Some(Alignment::Center),
                    _ => None,
                }
            }
            "display" => match value.as_str() {
                "none" => self.hidden = true,
                // `inline-block` and its siblings shrink to fit, so the alignment
                // around them places the box and their own aligns the text inside it.
                // Plain `inline` is not a box of its own and stays out of this.
                other if other.starts_with("inline-") => self.shrink = true,
                _ => {}
            },
            "visibility" if value == "hidden" => self.hidden = true,
            _ => {}
        }
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
        let digits: Vec<u8> = hex
            .trim()
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

/// What the stylesheet needs to know about an element: its tag and attributes, with
/// names lowercased.
#[derive(Debug, Clone, Copy)]
pub struct ElementRef<'a> {
    pub tag: &'a str,
    pub attrs: &'a [(String, String)],
}

impl ElementRef<'_> {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// An element inside its ancestor chain (outermost first), as simplecss wants to walk it.
#[derive(Clone, Copy)]
struct Node<'a> {
    chain: &'a [ElementRef<'a>],
    index: usize,
}

impl simplecss::Element for Node<'_> {
    fn parent_element(&self) -> Option<Self> {
        self.index.checked_sub(1).map(|index| Node {
            chain: self.chain,
            index,
        })
    }

    /// Siblings are not tracked, so `:first-child` and friends never match.
    fn prev_sibling_element(&self) -> Option<Self> {
        None
    }

    fn has_local_name(&self, name: &str) -> bool {
        self.chain[self.index].tag == name
    }

    fn attribute_matches(&self, local_name: &str, operator: AttributeOperator<'_>) -> bool {
        let Some(value) = self.chain[self.index].attr(local_name) else {
            return false;
        };
        match operator {
            AttributeOperator::Exists => true,
            AttributeOperator::Matches(expected) => value == expected,
            AttributeOperator::Contains(word) => value.split_whitespace().any(|w| w == word),
            AttributeOperator::StartsWith(prefix) => value.starts_with(prefix),
        }
    }

    fn pseudo_class_matches(&self, _class: PseudoClass<'_>) -> bool {
        false
    }
}

struct Rule<'a> {
    selector: Selector<'a>,
    normal: Decl,
    important: Decl,
}

/// A parsed stylesheet; borrows the CSS text like simplecss does.
#[derive(Default)]
pub struct Stylesheet<'a> {
    /// In specificity order, so later matches override earlier ones.
    rules: Vec<Rule<'a>>,
}

impl<'a> Stylesheet<'a> {
    pub fn parse(css: &'a str) -> Self {
        let rules = simplecss::StyleSheet::parse(css)
            .rules
            .into_iter()
            .map(|rule| {
                let mut normal = Decl::default();
                let mut important = Decl::default();
                for declaration in &rule.declarations {
                    let target = if declaration.important {
                        &mut important
                    } else {
                        &mut normal
                    };
                    target.set(declaration);
                }
                Rule {
                    selector: rule.selector,
                    normal,
                    important,
                }
            })
            .collect();
        Self { rules }
    }

    /// Combined declaration for the last element in `chain` (outermost first).
    pub fn declaration_for(&self, chain: &[ElementRef<'_>]) -> Decl {
        let mut decl = Decl::default();
        if chain.is_empty() {
            return decl;
        }
        let node = Node {
            chain,
            index: chain.len() - 1,
        };
        let matching: Vec<&Rule<'_>> = self
            .rules
            .iter()
            .filter(|rule| rule.selector.matches(&node))
            .collect();
        for rule in &matching {
            decl.merge(&rule.normal);
        }
        for rule in &matching {
            decl.merge(&rule.important);
        }
        decl
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn chain<'a>(elements: &'a [(&'a str, &'a [(String, String)])]) -> Vec<ElementRef<'a>> {
        elements
            .iter()
            .map(|(tag, attrs)| ElementRef { tag, attrs })
            .collect()
    }

    #[test]
    fn parses_alignment_size_and_decoration() {
        let sheet = Stylesheet::parse(
            "/* comment */ .source { padding-top: 20px; font-size: 50%; text-align: right }\n\
             .source a { text-decoration: none }",
        );
        let source_attrs = attrs(&[("class", "source")]);
        let none = attrs(&[]);
        let source = sheet.declaration_for(&chain(&[("div", &source_attrs)]));
        assert_eq!(source.align, Some(Alignment::Right));
        assert!(source.add.contains(Modifier::DIM));
        let link = sheet.declaration_for(&chain(&[("div", &source_attrs), ("a", &none)]));
        assert!(link.remove.contains(Modifier::UNDERLINED));
        let other_link = sheet.declaration_for(&chain(&[("div", &none), ("a", &none)]));
        assert_eq!(other_link, Decl::default());
    }

    #[test]
    fn later_more_specific_and_important_rules_win() {
        let sheet = Stylesheet::parse(
            "b { color: red } .x { color: blue } b { color: green } \
             i { color: red !important } i.y { color: blue }",
        );
        let none = attrs(&[]);
        let x = attrs(&[("class", "x")]);
        let y = attrs(&[("class", "y")]);
        assert_eq!(
            sheet.declaration_for(&chain(&[("b", &none)])).fg,
            Some(Color::Green)
        );
        assert_eq!(
            sheet.declaration_for(&chain(&[("b", &x)])).fg,
            Some(Color::Blue)
        );
        assert_eq!(
            sheet.declaration_for(&chain(&[("i", &y)])).fg,
            Some(Color::Red)
        );
    }

    #[test]
    fn child_combinators_and_attribute_selectors_match() {
        let sheet = Stylesheet::parse(
            "div > b { font-weight: normal } a[href] { color: green } \
             span[lang=\"de\"] { font-style: italic }",
        );
        let none = attrs(&[]);
        let nested = sheet.declaration_for(&chain(&[("div", &none), ("p", &none), ("b", &none)]));
        assert_eq!(
            nested,
            Decl::default(),
            "child combinator needs a direct parent"
        );
        let direct = sheet.declaration_for(&chain(&[("div", &none), ("b", &none)]));
        assert!(direct.remove.contains(Modifier::BOLD));
        let href = attrs(&[("href", "x")]);
        assert_eq!(
            sheet.declaration_for(&chain(&[("a", &href)])).fg,
            Some(Color::Green)
        );
        assert_eq!(sheet.declaration_for(&chain(&[("a", &none)])).fg, None);
        let de = attrs(&[("lang", "de")]);
        assert!(
            sheet
                .declaration_for(&chain(&[("span", &de)]))
                .add
                .contains(Modifier::ITALIC)
        );
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
        let none = attrs(&[]);
        let card = attrs(&[("class", "card")]);
        let hidden = attrs(&[("class", "hidden")]);
        assert_eq!(
            sheet.declaration_for(&chain(&[("a", &none)])).fg,
            None,
            "hover never applies"
        );
        assert!(!sheet.declaration_for(&chain(&[("div", &card)])).hidden);
        assert!(sheet.declaration_for(&chain(&[("div", &hidden)])).hidden);
    }

    #[test]
    fn inline_declarations_parse_the_same_way() {
        let decl = Decl::parse("font-weight: bold; text-align: center; font-size: 12px");
        assert!(decl.add.contains(Modifier::BOLD | Modifier::DIM));
        assert_eq!(decl.align, Some(Alignment::Center));
    }

    #[test]
    fn only_inline_level_boxes_shrink_to_fit() {
        assert!(Decl::parse("display: inline-block").shrink);
        assert!(Decl::parse("display: inline-flex").shrink);
        assert!(!Decl::parse("display: inline").shrink);
        assert!(!Decl::parse("display: block").shrink);
        assert!(Decl::parse("display: none").hidden);
    }
}
