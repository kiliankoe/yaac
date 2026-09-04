//! LaTeX in cards. Anki has two kinds: the legacy `[latex]`, `[$]`, and `[$$]` markup,
//! which the desktop runs through a TeX install and caches as an image in the media
//! folder, and MathJax's `\(` and `\[` delimiters, which its web view typesets while
//! showing the card. Here both become a [`Math`], which is drawn as an image typeset
//! in-process (or the desktop's cached one, when there is one) or shown as Unicode
//! text when that loses nothing.

use std::panic::{self, AssertUnwindSafe};

use anki::latex::extract_latex;
use anki::text::strip_html;
use image::DynamicImage;
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;

/// Pixels per em, relative to the height of a cell. Text sits smaller than its cell,
/// so this makes formulas a little larger than the words around them, which is
/// what reads best when the formula is the point of the card.
pub const EM_PER_CELL: f32 = 1.2;

/// A formula found in card HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Math {
    /// The formula without its delimiters, HTML stripped.
    pub latex: String,
    /// Display style sits on a line of its own, like `\[` and `[$$]`.
    pub display: bool,
    /// Stem of the media file the desktop caches its render under, for the legacy
    /// markup; the extension is the notetype's choice of PNG or SVG.
    pub cached: Option<String>,
}

impl Math {
    /// Key for the image cache; formulas are rendered once per text and style.
    pub fn key(&self) -> String {
        let style = if self.display { "display" } else { "inline" };
        format!("math:{style}:{}", self.latex)
    }

    /// Unicode approximation: Greek letters, operators, simple sub- and superscripts.
    pub fn text(&self) -> String {
        let mut text = self.latex.clone();
        for command in ["\\mathrm", "\\text", "\\textrm", "\\operatorname"] {
            text = unwrap_argument(&text, command);
        }
        for command in ["\\left", "\\right", "\\displaystyle"] {
            text = remove_command(&text, command);
        }
        for (command, replacement) in [
            ("\\qquad", "  "),
            ("\\quad", " "),
            ("\\,", " "),
            ("\\;", " "),
            ("\\:", " "),
            ("\\!", ""),
        ] {
            text = text.replace(command, replacement);
        }
        let text = unicodeit::replace(&text);
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Whether [`Math::text`] shows the whole formula, so an image adds nothing.
    pub fn fits_text(&self) -> bool {
        !self.text().contains(['\\', '{', '}', '^', '_'])
    }
}

/// `\cmd{arg}` becomes `arg`, at every depth.
fn unwrap_argument(text: &str, command: &str) -> String {
    let opener = format!("{command}{{");
    let mut text = text.to_string();
    while let Some(start) = text.find(&opener) {
        let arg_start = start + opener.len();
        let Some(len) = balanced_len(&text[arg_start..]) else {
            break;
        };
        let inner = text[arg_start..arg_start + len].to_string();
        text.replace_range(start..arg_start + len + 1, &inner);
    }
    text
}

/// Length of the text up to the `}` closing the brace just before it.
fn balanced_len(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => return Some(i),
            '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Drops `\cmd` where it stands alone, leaving `\cmdsuffix` commands like `\leftarrow`.
fn remove_command(text: &str, command: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(command) {
        let after = &rest[at + command.len()..];
        let standalone = !after.starts_with(|c: char| c.is_ascii_alphabetic());
        out.push_str(&rest[..at]);
        if !standalone {
            out.push_str(command);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Legacy,
    LegacyDisplay,
    LegacyInline,
    Display,
    Inline,
}

const FORMS: [(&str, &str, Kind); 5] = [
    ("[latex]", "[/latex]", Kind::Legacy),
    ("[$$]", "[/$$]", Kind::LegacyDisplay),
    ("[$]", "[/$]", Kind::LegacyInline),
    ("\\[", "\\]", Kind::Display),
    ("\\(", "\\)", Kind::Inline),
];

/// The markup at the start of `rest`, if there is any, with how many bytes it spans.
/// Anything unclosed is not markup.
pub fn parse_at(rest: &str) -> Option<(Math, usize)> {
    let (open, close, kind) = FORMS
        .into_iter()
        .find(|(open, _, _)| starts_with_ignore_case(rest, open))?;
    let after = &rest[open.len()..];
    let end = find_ignore_case(after, close)?;
    let len = open.len() + end + close.len();
    let math = match kind {
        Kind::Legacy | Kind::LegacyDisplay | Kind::LegacyInline => {
            // rslib strips the HTML and names the file the way the desktop does, so
            // the cached render is found under the same name.
            let (_, extracted) = extract_latex(&rest[..len], false);
            let extracted = extracted.into_iter().next()?;
            Math {
                latex: unwrap_math_mode(&extracted.latex).trim().to_string(),
                display: kind != Kind::LegacyInline,
                cached: Some(extracted.fname.trim_end_matches(".png").to_string()),
            }
        }
        Kind::Display | Kind::Inline => Math {
            latex: strip_html(&after[..end]).trim().to_string(),
            display: kind == Kind::Display,
            cached: None,
        },
    };
    Some((math, len))
}

/// The text with every formula replaced by its Unicode approximation, for lists and
/// other places that show a field as plain text.
pub fn formulas_to_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some((math, len)) = parse_at(rest) {
            out.push_str(&math.text());
            rest = &rest[len..];
            continue;
        }
        let first = rest.chars().next().map_or(0, char::len_utf8);
        let end = rest[first..]
            .find(['[', '\\'])
            .map_or(rest.len(), |at| first + at);
        out.push_str(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

/// The formula inside `$...$`, `$$...$$`, or a display environment; rslib adds the
/// first and last for the `[$]` forms, and `[latex]` bodies often carry their own.
fn unwrap_math_mode(latex: &str) -> &str {
    let latex = latex.trim();
    for (open, close) in [
        ("$$", "$$"),
        ("$", "$"),
        ("\\begin{displaymath}", "\\end{displaymath}"),
        ("\\begin{equation*}", "\\end{equation*}"),
        ("\\begin{equation}", "\\end{equation}"),
    ] {
        if let Some(inner) = latex
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        {
            return inner;
        }
    }
    latex
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Byte offset of an ASCII `needle`, so the result is always a character boundary.
fn find_ignore_case(text: &str, needle: &str) -> Option<usize> {
    text.as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Typesets the formula at `em_px` pixels per em in `colour` on a transparent background.
pub fn render(math: &Math, colour: [u8; 3], em_px: f32) -> Result<DynamicImage, String> {
    let style = if math.display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let [r, g, b] = colour.map(|channel| f32::from(channel) / 255.0);
    let layout_options = LayoutOptions::default()
        .with_style(style)
        .with_color(Color::rgb(r, g, b));
    let render_options = RenderOptions {
        font_size: em_px,
        padding: 2.0,
        background_color: Color::new(0.0, 0.0, 0.0, 0.0),
        font_dir: String::new(),
        device_pixel_ratio: 1.0,
    };
    let png = quietly(|| {
        let ast = parse(&math.latex).map_err(|err| format!("parsing formula: {err}"))?;
        let list = to_display_list(&layout(&ast, &layout_options));
        render_to_png(&list, &render_options)
    })?;
    image::load_from_memory(&png).map_err(|err| format!("decoding typeset formula: {err}"))
}

/// The typesetter is young; a panic on some construct must neither take the screen
/// down nor print over it, so it is caught with the panic hook silenced meanwhile.
fn quietly<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let result = panic::catch_unwind(AssertUnwindSafe(work));
    panic::set_hook(hook);
    result.unwrap_or_else(|_| Err("typesetting failed".to_string()))
}

/// The desktop renders black on transparent, which vanishes on a dark terminal, so a
/// monochrome image takes the colour formulas are drawn in here. Anything coloured on
/// purpose is left alone.
pub fn recolour(image: DynamicImage, colour: [u8; 3]) -> DynamicImage {
    let mut rgba = image.into_rgba8();
    let ink = |pixel: &image::Rgba<u8>| pixel.0[3] > 0;
    let monochrome = rgba
        .pixels()
        .filter(|pixel| ink(pixel))
        .all(|pixel| pixel.0[..3].iter().all(|&channel| channel < 0x60));
    if monochrome {
        for pixel in rgba.pixels_mut().filter(|pixel| ink(pixel)) {
            pixel.0[..3].copy_from_slice(&colour);
        }
    }
    rgba.into()
}

/// `#rgb`, `#rrggbb`, or a CSS colour name.
pub fn parse_colour(text: &str) -> Option<[u8; 3]> {
    let colour = Color::parse(text.trim())?;
    Some([colour.r, colour.g, colour.b].map(|channel| (channel * 255.0).round() as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn math(latex: &str, display: bool) -> Math {
        Math {
            latex: latex.to_string(),
            display,
            cached: None,
        }
    }

    #[test]
    fn legacy_markup_is_found_with_the_desktops_media_name() {
        // The names are rslib's own test vectors, so the cache lookup matches the desktop.
        let (found, len) = parse_at("[$]<b>hello</b>&nbsp; world[/$] after").unwrap();
        assert_eq!(len, "[$]<b>hello</b>&nbsp; world[/$]".len());
        assert_eq!(
            found,
            Math {
                latex: "hello  world".to_string(),
                display: false,
                cached: Some("latex-060219fbf3ddb74306abddaf4504276ad793b029".to_string()),
            }
        );
        let (found, _) = parse_at("[$$]math &amp; stuff[/$$]").unwrap();
        assert_eq!(
            found,
            Math {
                latex: "math & stuff".to_string(),
                display: true,
                cached: Some("latex-8899f3f849ffdef6e4e9f2f34a923a1f608ebc07".to_string()),
            }
        );
        let (found, _) = parse_at("[latex]one<br>and<div>two[/latex]").unwrap();
        assert_eq!(
            found,
            Math {
                latex: "one\nand\ntwo".to_string(),
                display: true,
                cached: Some("latex-ef30b3f4141c33a5bf7044b0d1961d3399c05d50".to_string()),
            }
        );
    }

    #[test]
    fn mathjax_delimiters_are_found_without_a_cache() {
        let (found, len) = parse_at(r"\(x^2\) and more").unwrap();
        assert_eq!(len, 7);
        assert_eq!(found, math("x^2", false));
        let (found, _) = parse_at(r"\[ \frac{a}{b} \]").unwrap();
        assert_eq!(found, math(r"\frac{a}{b}", true));
        let (found, _) = parse_at(r"\(a &lt; <b>b</b>\)").unwrap();
        assert_eq!(found.latex, "a < b");
    }

    #[test]
    fn unclosed_or_absent_markup_is_text() {
        assert_eq!(parse_at("[$]x"), None);
        assert_eq!(parse_at(r"\(x"), None);
        assert_eq!(parse_at("[sound:a.mp3]"), None);
        assert_eq!(parse_at("hello"), None);
    }

    #[test]
    fn legacy_markers_ignore_case_and_wrappers_in_the_body() {
        let (found, _) = parse_at("[LaTeX]$x$[/LATEX]").unwrap();
        assert_eq!(found.latex, "x");
        assert!(found.display);
    }

    #[test]
    fn simple_formulas_fit_in_text_and_others_do_not() {
        assert_eq!(math(r"\alpha^2 \le \infty", false).text(), "α² ≤ ∞");
        assert!(math(r"\alpha^2", false).fits_text());
        assert_eq!(math(r"\mathrm{V}", false).text(), "V");
        assert_eq!(math(r"\text{where } x \, y", false).text(), "where x y");
        assert_eq!(
            math(r"\left( x \rightarrow y \right)", false).text(),
            "( x → y )"
        );
        let fraction = math(r"\frac{a}{b}", false);
        assert!(!fraction.fits_text());
        assert!(fraction.text().contains(r"\frac"));
        assert!(!math(r"e^{+j\phi}", false).fits_text());
    }

    #[test]
    fn plain_text_shows_formulas_as_unicode() {
        assert_eq!(
            formulas_to_text(r"Roman \(\mathrm{V}\), [$]x^2[/$], and \[\frac{a}{b}\] or [x]"),
            r"Roman V, x², and \frac{a}{b} or [x]"
        );
        assert_eq!(formulas_to_text("no math"), "no math");
    }

    #[test]
    fn formulas_render_in_the_colour_on_a_transparent_background() {
        let image = render(&math(r"\frac{a}{b}", true), [255, 0, 0], 20.0).unwrap();
        let rgba = image.to_rgba8();
        assert!(
            rgba.width() > 4 && rgba.height() > 20,
            "a fraction is taller than wide"
        );
        assert_eq!(rgba.get_pixel(0, 0).0[3], 0, "corner is transparent");
        assert!(
            rgba.pixels()
                .any(|p| p.0[3] == 255 && p.0[..3] == [255, 0, 0]),
            "glyphs are solid red"
        );
        assert!(render(&math(r"\frac{", true), [0; 3], 20.0).is_err());
    }

    #[test]
    fn desktop_renders_take_the_colour_and_coloured_ones_do_not() {
        let mut black = image::RgbaImage::new(2, 1);
        black.put_pixel(0, 0, image::Rgba([0, 0, 0, 200]));
        black.put_pixel(1, 0, image::Rgba([0, 0, 0, 0]));
        let recoloured = recolour(black.into(), [10, 20, 30]).to_rgba8();
        assert_eq!(recoloured.get_pixel(0, 0).0, [10, 20, 30, 200]);
        assert_eq!(recoloured.get_pixel(1, 0).0[3], 0);

        let mut red = image::RgbaImage::new(1, 1);
        red.put_pixel(0, 0, image::Rgba([200, 0, 0, 255]));
        let kept = recolour(red.into(), [10, 20, 30]).to_rgba8();
        assert_eq!(kept.get_pixel(0, 0).0, [200, 0, 0, 255]);
    }

    #[test]
    fn colours_parse_as_hex_or_names() {
        assert_eq!(parse_colour("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_colour("#1a2B3c"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(parse_colour("white"), Some([255, 255, 255]));
        assert_eq!(parse_colour("nope"), None);
    }
}
