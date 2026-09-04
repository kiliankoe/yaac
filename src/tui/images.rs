//! Images on screen: decoding, protocol probing, and a cache of encoded images so a
//! redraw costs nothing. Kitty goes through our own implementation in [`crate::tui::kitty`];
//! the other protocols through ratatui-image.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::DynamicImage;
use ratatui::layout::{Rect, Size};
use ratatui_image::Resize;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Capability, Picker, ProtocolType};
use ratatui_image::protocol::Protocol;

use crate::render::image::load;
use crate::render::latex::{self, Math};
use crate::render::occlusion::{self, Mask};
use crate::tui::kitty::{self, MAX_ID, Placement};

/// The same check ratatui-image uses to decide on tmux passthrough.
pub fn in_tmux() -> bool {
    std::env::var("TERM").is_ok_and(|term| term.starts_with("tmux"))
        || std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "tmux")
}

/// Protocol name from the config, or `auto`.
pub fn probe(setting: Option<&str>) -> Option<Picker> {
    let setting = setting.unwrap_or("auto").to_ascii_lowercase();
    if setting == "off" {
        return None;
    }
    // Querying the terminal also learns the cell size in pixels, which even a forced
    // protocol needs for scaling, and the background colour, which decides the ink
    // for formulas; half-blocks are the fallback for terminals that do not answer.
    let options = QueryStdioOptions {
        terminal_background_color_osc: true,
        ..QueryStdioOptions::default()
    };
    let mut picker =
        Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
    let forced = match setting.as_str() {
        "kitty" => Some(ProtocolType::Kitty),
        "sixel" => Some(ProtocolType::Sixel),
        "iterm2" => Some(ProtocolType::Iterm2),
        "halfblocks" => Some(ProtocolType::Halfblocks),
        _ => None,
    };
    if let Some(protocol) = forced {
        picker.set_protocol_type(protocol);
    }
    Some(picker)
}

/// The terminal's background, when the probe learned it.
pub fn background(picker: &Picker) -> Option<(u8, u8, u8)> {
    picker.capabilities().iter().find_map(|cap| match cap {
        Capability::Background(r, g, b) => Some((*r, *g, *b)),
        _ => None,
    })
}

/// Ink for formulas: black on a light background, white on a dark one, and a grey that
/// reads on either when the terminal did not say.
pub fn math_colour_for(background: Option<(u8, u8, u8)>) -> [u8; 3] {
    match background {
        Some((r, g, b)) => {
            let luminance = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
            if luminance > 128.0 { [0; 3] } else { [255; 3] }
        }
        None => [0x88; 3],
    }
}

/// An image ready to draw at one cell size.
pub enum Encoded {
    Native(Protocol),
    Kitty(Placement),
}

pub struct Images {
    picker: Option<Picker>,
    media_dir: PathBuf,
    /// Ink for typeset formulas.
    math_colour: [u8; 3],
    decoded: HashMap<String, Result<Arc<DynamicImage>, String>>,
    /// Encoded for a specific cell size; keyed by file and size because the encoding
    /// depends on both.
    encoded: HashMap<(String, u16, u16), Encoded>,
    next_kitty_id: u32,
    /// Where images landed in the frame being drawn and in the previous one, with the
    /// Kitty placement when there is one.
    placed: Vec<(String, Rect, Option<Placement>)>,
    placed_before: Vec<(String, Rect, Option<Placement>)>,
    refresh_requested: bool,
    /// Frames on which the current placements still get sent to the terminal directly.
    resends_left: u8,
    /// Running under tmux; decided once, overridable in tests.
    tmux: bool,
    /// Screen position of the pane, fixed in tests instead of asked from tmux.
    fixed_origin: Option<(u16, u16)>,
    /// Where Kitty transmissions go: the terminal, or a buffer in tests.
    sink: Box<dyn Write + Send>,
}

impl Images {
    pub fn new(picker: Option<Picker>, media_dir: impl Into<PathBuf>) -> Self {
        Self::with_sink(picker, media_dir, Box::new(std::io::stdout()), in_tmux())
    }

    pub fn with_sink(
        picker: Option<Picker>,
        media_dir: impl Into<PathBuf>,
        sink: Box<dyn Write + Send>,
        tmux: bool,
    ) -> Self {
        let math_colour = math_colour_for(picker.as_ref().and_then(background));
        Self {
            picker,
            media_dir: media_dir.into(),
            math_colour,
            decoded: HashMap::new(),
            encoded: HashMap::new(),
            next_kitty_id: 1,
            placed: Vec::new(),
            placed_before: Vec::new(),
            refresh_requested: false,
            resends_left: 0,
            tmux,
            fixed_origin: None,
            sink,
        }
    }

    /// Labels only: no protocol, nothing decoded.
    pub fn disabled(media_dir: impl Into<PathBuf>) -> Self {
        Self::new(None, media_dir)
    }

    /// Overrides the ink for formulas, from the config.
    pub fn with_math_colour(mut self, colour: [u8; 3]) -> Self {
        self.math_colour = colour;
        self
    }

    pub fn math_colour(&self) -> [u8; 3] {
        self.math_colour
    }

    pub fn enabled(&self) -> bool {
        self.picker.is_some()
    }

    pub fn media_dir(&self) -> &Path {
        &self.media_dir
    }

    fn kitty(&self) -> bool {
        self.picker
            .as_ref()
            .is_some_and(|picker| picker.protocol_type() == ProtocolType::Kitty)
    }

    fn next_kitty_id(&mut self) -> u32 {
        let id = self.next_kitty_id;
        self.next_kitty_id = if id >= MAX_ID { 1 } else { id + 1 };
        id
    }

    fn decoded(&mut self, src: &str) -> Result<Arc<DynamicImage>, String> {
        if !self.decoded.contains_key(src) {
            let result = load(&self.media_dir, src)
                .map(Arc::new)
                .map_err(|err| format!("{err:#}"));
            self.decoded.insert(src.to_string(), result);
        }
        self.decoded[src].clone()
    }

    /// Makes the formula available under the key it returns, so that the image
    /// methods find it; typeset once, like a file is decoded once.
    pub fn math(&mut self, math: &Math) -> String {
        let key = math.key();
        if !self.decoded.contains_key(&key) {
            let result = self.typeset(math).map(Arc::new);
            self.decoded.insert(key.clone(), result);
        }
        key
    }

    /// The desktop's cached render when the media folder has one, since real TeX
    /// covers more than the in-process typesetter, otherwise the latter.
    fn typeset(&self, math: &Math) -> Result<DynamicImage, String> {
        let Some(picker) = &self.picker else {
            return Err("images are off".to_string());
        };
        // Half-block cells are far too coarse for a formula; the text stands in.
        if picker.protocol_type() == ProtocolType::Halfblocks {
            return Err("half-blocks cannot show a formula".to_string());
        }
        if let Some(stem) = &math.cached {
            for ext in ["png", "svg"] {
                if let Ok(image) = load(&self.media_dir, &format!("{stem}.{ext}")) {
                    return Ok(latex::recolour(image, self.math_colour));
                }
            }
        }
        let em_px = f32::from(picker.font_size().height) * latex::EM_PER_CELL;
        latex::render(math, self.math_colour, em_px)
    }

    /// Why an image cannot be shown, if it cannot.
    pub fn problem(&mut self, src: &str) -> Option<String> {
        if self.picker.is_none() {
            return Some("images are off".to_string());
        }
        self.decoded(src).err()
    }

    /// Cells the image will occupy when fitted into `available`, keeping its aspect.
    pub fn size_for(&mut self, src: &str, available: Size) -> Option<Size> {
        let picker = self.picker.as_ref()?;
        let font_size = picker.font_size();
        let image = self.decoded(src).ok()?;
        let size = Resize::Fit(None).size_for(&image, font_size, available);
        (size.width > 0 && size.height > 0).then_some(size)
    }

    /// The encoded image for exactly `size` cells with these occlusion masks painted,
    /// built (and for Kitty, transmitted) once per combination.
    pub fn protocol(&mut self, src: &str, size: Size, masks: &[Mask]) -> Option<&Encoded> {
        let mask_key = occlusion::key(masks);
        let key = (format!("{src}#{mask_key}"), size.width, size.height);
        if !self.encoded.contains_key(&key) {
            let image = self.decoded(src).ok()?;
            let image = if mask_key.is_empty() {
                image
            } else {
                Arc::new(occlusion::apply(&image, masks))
            };
            let encoded = if self.kitty() {
                let font_size = self.picker.as_ref()?.font_size();
                let fitted = Resize::Fit(None).resize(&image, font_size, size, None);
                let png = kitty::encode_png(&fitted).ok()?;
                let id = self.next_kitty_id();
                let burst = kitty::burst_for(crossterm::terminal::size().unwrap_or((80, 24)));
                kitty::transmit(&mut *self.sink, id, &png, self.tmux, burst).ok()?;
                Encoded::Kitty(Placement { id, size })
            } else {
                let picker = self.picker.as_ref()?;
                Encoded::Native(
                    picker
                        .new_protocol((*image).clone(), size, Resize::Fit(None))
                        .ok()?,
                )
            };
            // Sizes change with the window; a handful of stale entries is fine, a
            // session's worth is not.
            if self.encoded.len() > 64 {
                self.clear();
            }
            self.encoded.insert(key.clone(), encoded);
        }
        self.encoded.get(&key)
    }

    /// Forgets every encoded image, freeing them in the terminal, so the next frame
    /// transmits afresh. Bound to a key for images that got stuck on the way.
    pub fn clear(&mut self) {
        let tmux = self.tmux;
        for encoded in self.encoded.values() {
            if let Encoded::Kitty(placement) = encoded {
                let _ = kitty::delete(&mut *self.sink, placement.id, tmux);
            }
        }
        self.encoded.clear();
        self.refresh_requested = true;
    }

    /// Sends the current placements again on the next frame, after something drawn
    /// over an image was removed: ratatui repaints the cells underneath, which loses
    /// their marks under tmux.
    pub fn refresh(&mut self) {
        self.refresh_requested = true;
    }

    /// Call at the start of a frame; placements are recorded until `end_frame`.
    pub fn begin_frame(&mut self) {
        self.placed_before = std::mem::take(&mut self.placed);
    }

    pub fn mark_placed(&mut self, src: &str, area: Rect, placement: Option<Placement>) {
        self.placed.push((src.to_string(), area, placement));
    }

    /// Call after the frame was written, on the same thread, so terminal writes stay in
    /// frame order. Under tmux, Kitty placeholder cells reach the terminal without their
    /// marks (tmux forwards a cell before the combining marks arrive and never re-sends
    /// it), so the cells are sent to the outer terminal directly: on the frame that
    /// placed or moved an image and on the next two, while the same placements are
    /// still on screen, since tmux may drop any single send when a pane floods it.
    pub fn end_frame(&mut self) {
        if !(self.kitty() && self.tmux) {
            return;
        }
        if self.placed != self.placed_before || self.refresh_requested {
            self.resends_left = 3;
        }
        self.refresh_requested = false;
        if self.resends_left == 0 || self.placed.is_empty() {
            self.resends_left = 0;
            return;
        }
        self.resends_left -= 1;
        let Some((left, top)) = self.fixed_origin.or_else(pane_origin) else {
            return;
        };
        for (_, area, placement) in &self.placed {
            if let Some(placement) = placement {
                let origin = (left + area.x, top + area.y);
                let _ = kitty::place_direct(&mut *self.sink, placement, origin, true);
            }
        }
    }
}

/// Screen position of this pane's top-left cell, from tmux. Panes sit below the status
/// line when it is at the top.
fn pane_origin() -> Option<(u16, u16)> {
    let pane = std::env::var("TMUX_PANE").ok()?;
    let output = std::process::Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{pane_left} #{pane_top} #{status-position} #{status}",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let mut parts = text.split_whitespace();
    let left: u16 = parts.next()?.parse().ok()?;
    let top: u16 = parts.next()?.parse().ok()?;
    let status_lines = match (parts.next(), parts.next()) {
        (Some("top"), Some("on")) => 1,
        (Some("top"), Some(lines)) => lines.parse().unwrap_or(0),
        _ => 0,
    };
    Some((left, top + status_lines))
}

impl Drop for Images {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A sink tests can read back after handing it to `Images`.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn kitty_images(dir: &Path, sink: Shared) -> Images {
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Kitty);
        Images::with_sink(Some(picker), dir, Box::new(sink), false)
    }

    fn math(latex: &str, cached: Option<&str>) -> Math {
        Math {
            latex: latex.to_string(),
            display: true,
            cached: cached.map(str::to_string),
        }
    }

    #[test]
    fn math_is_typeset_unless_the_desktop_left_a_render_in_the_media_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut images = kitty_images(dir.path(), Shared::default()).with_math_colour([1, 2, 3]);
        let key = images.math(&math(r"\frac{a}{b}", None));
        let size = images.size_for(&key, Size::new(80, 10)).unwrap();
        assert!(size.height >= 2, "a fraction spans rows: {size:?}");
        let typeset = images.decoded[&key].as_ref().unwrap().to_rgba8();
        assert!(typeset.pixels().any(|p| p.0 == [1, 2, 3, 255]));

        let mut png = image::RgbaImage::new(4, 4);
        png.put_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        png.save(dir.path().join("latex-abc.png")).unwrap();
        let key = images.math(&math("ignored", Some("latex-abc")));
        let cached = images.decoded[&key].as_ref().unwrap().to_rgba8();
        assert_eq!(cached.dimensions(), (4, 4), "the desktop's file is used");
        assert_eq!(cached.get_pixel(1, 1).0, [1, 2, 3, 255], "in our colour");

        assert_eq!(
            images.math(&math("x", Some("latex-missing"))),
            math("x", Some("latex-missing")).key()
        );
        assert!(
            images.decoded[&key].is_ok(),
            "a missing file falls back to typesetting"
        );
    }

    #[test]
    fn math_needs_a_graphics_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let mut images = Images::new(Some(Picker::halfblocks()), dir.path());
        let key = images.math(&math("x", None));
        assert_eq!(images.size_for(&key, Size::new(80, 10)), None);
        assert!(images.problem(&key).is_some());
        let mut off = Images::disabled(dir.path());
        let key = off.math(&math("x", None));
        assert_eq!(off.size_for(&key, Size::new(80, 10)), None);
    }

    #[test]
    fn math_colour_follows_the_terminal_background() {
        assert_eq!(math_colour_for(None), [0x88; 3], "unknown background");
        assert_eq!(
            math_colour_for(Some((250, 250, 250))),
            [0; 3],
            "dark ink on a light background"
        );
        assert_eq!(math_colour_for(Some((20, 20, 30))), [255; 3]);
    }

    #[test]
    fn kitty_ids_stay_within_the_colour_bytes_and_wrap() {
        let dir = tempfile::tempdir().unwrap();
        let mut images = Images::new(None, dir.path());
        assert_eq!(images.next_kitty_id(), 1);
        images.next_kitty_id = MAX_ID;
        assert_eq!(images.next_kitty_id(), MAX_ID);
        assert_eq!(
            images.next_kitty_id(),
            1,
            "wraps instead of growing an upper byte"
        );
    }

    #[test]
    fn frames_notice_when_an_image_appears_or_moves() {
        let dir = tempfile::tempdir().unwrap();
        let mut images = Images::new(None, dir.path());
        let area = Rect::new(2, 5, 10, 4);
        images.begin_frame();
        images.mark_placed("map.png", area, None);
        images.end_frame();
        assert_ne!(images.placed, images.placed_before, "first appearance");

        images.begin_frame();
        images.mark_placed("map.png", area, None);
        images.end_frame();
        assert_eq!(images.placed, images.placed_before, "same card, same spot");

        images.begin_frame();
        images.mark_placed("map.png", Rect::new(2, 9, 10, 4), None);
        images.end_frame();
        assert_ne!(
            images.placed, images.placed_before,
            "moved after the reveal"
        );
    }

    #[test]
    fn under_tmux_placements_are_resent_on_the_next_two_frames_only() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Shared::default();
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Kitty);
        let mut images = Images::with_sink(Some(picker), dir.path(), Box::new(sink.clone()), true);
        images.fixed_origin = Some((10, 2));
        let placement = Placement {
            id: 3,
            size: Size::new(2, 1),
        };
        let sends = |sink: &Shared| {
            String::from_utf8(sink.0.lock().unwrap().clone())
                .unwrap()
                .matches("\x1b\x1b7")
                .count()
        };

        let frame = |images: &mut Images, area: Option<Rect>| {
            images.begin_frame();
            if let Some(area) = area {
                images.mark_placed("a.png", area, Some(placement.clone()));
            }
            images.end_frame();
        };
        let area = Rect::new(4, 6, 2, 1);
        frame(&mut images, Some(area));
        assert_eq!(sends(&sink), 1, "sent with the frame that placed it");
        assert!(
            String::from_utf8(sink.0.lock().unwrap().clone())
                .unwrap()
                .contains("\x1b\x1b[9;15H"),
            "positioned at pane origin plus area"
        );
        frame(&mut images, Some(area));
        frame(&mut images, Some(area));
        assert_eq!(sends(&sink), 3, "two follow-ups");
        frame(&mut images, Some(area));
        frame(&mut images, Some(area));
        assert_eq!(sends(&sink), 3, "then quiet while nothing changes");
        frame(&mut images, None);
        assert_eq!(sends(&sink), 3, "nothing to send once the image is gone");
        frame(&mut images, Some(Rect::new(4, 8, 2, 1)));
        assert_eq!(sends(&sink), 4, "a moved image starts over");
    }

    #[test]
    fn kitty_images_are_transmitted_once_as_png_and_freed_on_clear() {
        let dir = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(28, 32, image::Rgba([0, 0, 255, 255]))
            .save(dir.path().join("blue.png"))
            .unwrap();
        let sink = Shared::default();
        let mut images = kitty_images(dir.path(), sink.clone());
        let size = images.size_for("blue.png", Size::new(10, 5)).expect("fits");

        let placement = match images.protocol("blue.png", size, &[]) {
            Some(Encoded::Kitty(placement)) => placement.clone(),
            _ => panic!("expected a Kitty placement"),
        };
        assert_eq!(placement.id, 1);
        assert_eq!(placement.size, size);
        let sent = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(sent.contains("_Gq=2,i=1,a=T,U=1,f=100,"), "{sent:.40}");

        images.protocol("blue.png", size, &[]);
        assert_eq!(images.next_kitty_id, 2, "cached, no second transmission");
        assert_eq!(sink.0.lock().unwrap().len(), sent.len());

        images.clear();
        let after = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(after.contains("_Gq=2,a=d,d=I,i=1"));
        assert!(images.refresh_requested, "a clear asks for a redraw");
    }
}
