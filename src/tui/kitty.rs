//! Kitty graphics with unicode placeholders, written here rather than taken from
//! ratatui-image because tmux needs three things its implementation does not do:
//! image ids below 2^24, since tmux drops the third combining mark of a placeholder cell
//! on its live output path; PNG payloads written in paced chunks, since tmux discards
//! pending output once it exceeds a few bytes per screen cell; and plain cells with
//! explicit row and column marks instead of one cell carrying a whole row.

use std::io::{self, Write};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::DynamicImage;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Color;
use ratatui::widgets::Widget;

/// Largest id that fits into the placeholder's colour bytes without a third mark.
pub const MAX_ID: u32 = (1 << 24) - 1;

/// Base64 characters per Kitty chunk, the protocol's maximum.
const CHUNK_CHARS: usize = 4096;
const PACE_PAUSE: Duration = Duration::from_millis(2);

/// Bytes to write between pauses under tmux. tmux discards pending client output
/// beyond roughly eight bytes per screen cell, so bursts stay at half that.
pub fn burst_for(terminal: (u16, u16)) -> usize {
    (usize::from(terminal.0) * usize::from(terminal.1) * 4).clamp(4 * 1024, 64 * 1024)
}

/// Placeholder cells for an image that was transmitted under `id` with a virtual
/// placement; the terminal draws the picture over them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub id: u32,
    pub size: Size,
}

impl Widget for &Placement {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [_, r, g, b] = self.id.to_be_bytes();
        let color = Color::Rgb(r, g, b);
        for y in 0..self.size.height.min(area.height) {
            for x in 0..self.size.width.min(area.width) {
                let symbol = format!("\u{10EEEE}{}{}", diacritic(y), diacritic(x));
                if let Some(cell) = buf.cell_mut((area.x + x, area.y + y)) {
                    cell.set_symbol(&symbol).set_fg(color);
                }
            }
        }
    }
}

pub fn encode_png(image: &DynamicImage) -> io::Result<Vec<u8>> {
    let mut png = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::NoFilter);
    image
        .to_rgba8()
        .write_with_encoder(encoder)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(png)
}

/// Sends `png` as image `id` with a virtual placement, quietly, wrapped for tmux when
/// asked. Under tmux the bytes go out in small paced bursts so tmux never has to drop any.
pub fn transmit(
    out: &mut dyn Write,
    id: u32,
    png: &[u8],
    tmux: bool,
    burst: usize,
) -> io::Result<()> {
    let data = BASE64.encode(png);
    let mut sequence = String::with_capacity(data.len() + data.len() / CHUNK_CHARS * 64 + 64);
    let chunks: Vec<&str> = data
        .as_bytes()
        .chunks(CHUNK_CHARS)
        .map(|chunk| std::str::from_utf8(chunk).expect("base64 is ASCII"))
        .collect();
    for (index, chunk) in chunks.iter().enumerate() {
        let more = u8::from(index + 1 < chunks.len());
        let head = if index == 0 {
            format!("q=2,i={id},a=T,U=1,f=100,m={more};")
        } else {
            format!("q=2,m={more};")
        };
        sequence.push_str(&wrap(&format!("{head}{chunk}"), tmux));
    }
    write_paced(out, sequence.as_bytes(), tmux, burst)
}

/// Writes the placeholder cells straight to the outer terminal, bypassing tmux's grid,
/// with the outer cursor saved and restored around them. tmux forwards a cell to the
/// terminal before its combining marks arrive and never re-sends it, so the marks only
/// reach the terminal on a full redraw; this delivers them right away. `origin` is the
/// screen position (column, row, zero-based) of the placement's top-left cell.
pub fn place_direct(
    out: &mut dyn Write,
    placement: &Placement,
    origin: (u16, u16),
    tmux: bool,
) -> io::Result<()> {
    let [_, r, g, b] = placement.id.to_be_bytes();
    let mut seq = String::new();
    seq.push_str("\x1b7");
    seq.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
    for y in 0..placement.size.height {
        seq.push_str(&format!("\x1b[{};{}H", origin.1 + y + 1, origin.0 + 1));
        for x in 0..placement.size.width {
            seq.push('\u{10EEEE}');
            seq.push(diacritic(y));
            seq.push(diacritic(x));
        }
    }
    seq.push_str("\x1b[39m\x1b8");
    let bytes = if tmux {
        format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
    } else {
        seq
    };
    out.write_all(bytes.as_bytes())?;
    out.flush()
}

/// Frees image `id` in the terminal.
pub fn delete(out: &mut dyn Write, id: u32, tmux: bool) -> io::Result<()> {
    out.write_all(wrap(&format!("q=2,a=d,d=I,i={id}"), tmux).as_bytes())?;
    out.flush()
}

/// One Kitty APC command, inside a tmux passthrough with ESC doubled when needed.
fn wrap(apc: &str, tmux: bool) -> String {
    if tmux {
        format!(
            "\x1bPtmux;\x1b\x1b_G{}\x1b\x1b\\\x1b\\",
            apc.replace('\x1b', "\x1b\x1b")
        )
    } else {
        format!("\x1b_G{apc}\x1b\\")
    }
}

fn write_paced(out: &mut dyn Write, bytes: &[u8], tmux: bool, burst: usize) -> io::Result<()> {
    if !tmux {
        out.write_all(bytes)?;
        return out.flush();
    }
    for burst in bytes.chunks(burst.max(1)) {
        out.write_all(burst)?;
        out.flush()?;
        std::thread::sleep(PACE_PAUSE);
    }
    Ok(())
}

fn diacritic(index: u16) -> char {
    DIACRITICS[usize::from(index).min(DIACRITICS.len() - 1)]
}

/// Row and column marks in the order Kitty defines them:
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders
static DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30D}',
    '\u{30E}',
    '\u{310}',
    '\u{312}',
    '\u{33D}',
    '\u{33E}',
    '\u{33F}',
    '\u{346}',
    '\u{34A}',
    '\u{34B}',
    '\u{34C}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35B}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36A}',
    '\u{36B}',
    '\u{36C}',
    '\u{36D}',
    '\u{36E}',
    '\u{36F}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59C}',
    '\u{59D}',
    '\u{59E}',
    '\u{59F}',
    '\u{5A0}',
    '\u{5A1}',
    '\u{5A8}',
    '\u{5A9}',
    '\u{5AB}',
    '\u{5AC}',
    '\u{5AF}',
    '\u{5C4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65A}',
    '\u{65B}',
    '\u{65D}',
    '\u{65E}',
    '\u{6D6}',
    '\u{6D7}',
    '\u{6D8}',
    '\u{6D9}',
    '\u{6DA}',
    '\u{6DB}',
    '\u{6DC}',
    '\u{6DF}',
    '\u{6E0}',
    '\u{6E1}',
    '\u{6E2}',
    '\u{6E4}',
    '\u{6E7}',
    '\u{6E8}',
    '\u{6EB}',
    '\u{6EC}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73A}',
    '\u{73D}',
    '\u{73F}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74A}',
    '\u{7EB}',
    '\u{7EC}',
    '\u{7ED}',
    '\u{7EE}',
    '\u{7EF}',
    '\u{7F0}',
    '\u{7F1}',
    '\u{7F3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81B}',
    '\u{81C}',
    '\u{81D}',
    '\u{81E}',
    '\u{81F}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82A}',
    '\u{82B}',
    '\u{82C}',
    '\u{82D}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{F82}',
    '\u{F83}',
    '\u{F86}',
    '\u{F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The base64 runs after every `m=0;` / `m=1;`, joined back together.
    fn payload_of(text: &str) -> String {
        text.split("m=")
            .skip(1)
            .filter_map(|part| part.split_once(';').map(|(_, rest)| rest))
            .map(|rest| {
                rest.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn placeholders_carry_row_column_and_id_colour() {
        let placement = Placement {
            id: 0x00010203,
            size: Size::new(3, 2),
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 4));
        (&placement).render(Rect::new(1, 1, 5, 3), &mut buf);
        let cell = &buf[(2, 2)];
        assert_eq!(
            cell.symbol(),
            format!("\u{10EEEE}{}{}", DIACRITICS[1], DIACRITICS[1])
        );
        assert_eq!(cell.fg, Color::Rgb(1, 2, 3));
        assert_eq!(buf[(0, 0)].symbol(), " ", "outside the placement untouched");
        assert_eq!(buf[(4, 3)].symbol(), " ", "beyond the image size untouched");
    }

    #[test]
    fn transmission_is_chunked_wrapped_and_decodes_to_the_png() {
        let image = DynamicImage::new_rgba8(80, 60);
        let png = encode_png(&image).unwrap();
        let mut out = Vec::new();
        transmit(&mut out, 7, &png, true, 8 * 1024).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("\x1bPtmux;\x1b\x1b_Gq=2,i=7,a=T,U=1,f=100,m="));
        assert!(
            text.contains("\x1b\x1b\\\x1b\\"),
            "tmux envelope closes each chunk"
        );
        assert_eq!(BASE64.decode(payload_of(&text)).unwrap(), png);

        let mut plain = Vec::new();
        transmit(&mut plain, 7, &png, false, 8 * 1024).unwrap();
        let plain = String::from_utf8(plain).unwrap();
        assert!(plain.starts_with("\x1b_Gq=2,i=7,a=T,U=1,f=100,"));
        assert!(!plain.contains("Ptmux"));
        assert!(
            plain.ends_with(",m=0;") || plain.contains("m=0;"),
            "last chunk closes"
        );
    }

    #[test]
    fn direct_placement_positions_every_row_and_restores_the_cursor() {
        let placement = Placement {
            id: 5,
            size: Size::new(2, 2),
        };
        let mut out = Vec::new();
        place_direct(&mut out, &placement, (10, 3), false).unwrap();
        let text = String::from_utf8(out).unwrap();
        let cell = |y: usize, x: usize| format!("\u{10EEEE}{}{}", DIACRITICS[y], DIACRITICS[x]);
        assert_eq!(
            text,
            format!(
                "\x1b7\x1b[38;2;0;0;5m\x1b[4;11H{}{}\x1b[5;11H{}{}\x1b[39m\x1b8",
                cell(0, 0),
                cell(0, 1),
                cell(1, 0),
                cell(1, 1)
            )
        );

        let mut wrapped = Vec::new();
        place_direct(&mut wrapped, &placement, (0, 0), true).unwrap();
        let wrapped = String::from_utf8(wrapped).unwrap();
        assert!(wrapped.starts_with("\x1bPtmux;\x1b\x1b7"));
        assert!(wrapped.ends_with("\x1b\x1b8\x1b\\"));
    }

    #[test]
    fn bursts_follow_the_terminal_size_within_bounds() {
        assert_eq!(burst_for((40, 20)), 4 * 1024, "tiny windows get the floor");
        assert_eq!(burst_for((80, 24)), 7680);
        assert_eq!(burst_for((200, 50)), 40_000);
        assert_eq!(
            burst_for((400, 100)),
            64 * 1024,
            "large windows get the ceiling"
        );
    }

    #[test]
    fn deletion_frees_the_image() {
        let mut out = Vec::new();
        delete(&mut out, 9, false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b_Gq=2,a=d,d=I,i=9\x1b\\"
        );
    }
}
