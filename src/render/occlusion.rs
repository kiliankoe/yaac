//! Anki's built-in image occlusion: shapes stored as cloze markers, rendered by rslib
//! into hidden `<div class="cloze" data-shape=...>` elements next to the image. The
//! desktop draws them on a canvas; here they are painted into the bitmap.

use image::{DynamicImage, RgbaImage};
use resvg::tiny_skia::{
    Color, FillRule, IntSize, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rect {
        left: f32,
        top: f32,
        width: f32,
        height: f32,
    },
    Ellipse {
        left: f32,
        top: f32,
        rx: f32,
        ry: f32,
    },
    Polygon {
        points: Vec<(f32, f32)>,
    },
}

/// What the card asks for on this side: hide the shape, reveal it (answer side), or
/// treat it as one of the other cards' shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskKind {
    Hidden,
    Revealed,
    Inactive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub shape: Shape,
    pub kind: MaskKind,
    /// "Hide all, guess one": inactive shapes stay covered too.
    pub occlude_inactive: bool,
}

impl Mask {
    /// From one of rslib's occlusion elements. Text labels and malformed shapes are
    /// skipped; unknown classes are not occlusion elements at all.
    pub fn from_element(class: &str, attrs: &[(String, String)]) -> Option<Mask> {
        let kind = match class {
            "cloze" => MaskKind::Hidden,
            "cloze-highlight" => MaskKind::Revealed,
            "cloze-inactive" => MaskKind::Inactive,
            _ => return None,
        };
        let attr = |name: &str| {
            attrs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let number = |name: &str| attr(name).and_then(|value| value.trim().parse::<f32>().ok());
        let shape = match attr("data-shape")? {
            "rect" => Shape::Rect {
                left: number("data-left")?,
                top: number("data-top")?,
                width: number("data-width")?,
                height: number("data-height")?,
            },
            "ellipse" => Shape::Ellipse {
                left: number("data-left")?,
                top: number("data-top")?,
                rx: number("data-rx")?,
                ry: number("data-ry")?,
            },
            "polygon" => {
                let points: Vec<(f32, f32)> = attr("data-points")?
                    .split_whitespace()
                    .filter_map(|pair| {
                        let (x, y) = pair.split_once(',')?;
                        Some((x.parse().ok()?, y.parse().ok()?))
                    })
                    .collect();
                if points.len() < 3 {
                    return None;
                }
                Shape::Polygon { points }
            }
            _ => return None,
        };
        Some(Mask {
            shape,
            kind,
            occlude_inactive: attr("data-occludeinactive") == Some("1"),
        })
    }

    /// Whether this mask changes the picture at all on this side.
    fn painted(&self) -> bool {
        match self.kind {
            MaskKind::Hidden | MaskKind::Revealed => true,
            MaskKind::Inactive => self.occlude_inactive,
        }
    }
}

/// A stable description of a mask set, for cache keys.
pub fn key(masks: &[Mask]) -> String {
    masks
        .iter()
        .filter(|mask| mask.painted())
        .map(|mask| format!("{:?}", mask))
        .collect::<Vec<_>>()
        .join("|")
}

/// Anki's defaults: the shape being asked is light red so it stands out from the other
/// covered shapes, which are yellow; a revealed shape gets a tint and a red edge so the
/// answer is visible but marked.
fn colours(kind: MaskKind) -> (Color, Color) {
    match kind {
        MaskKind::Hidden => (
            Color::from_rgba8(255, 142, 142, 255),
            Color::from_rgba8(200, 60, 60, 255),
        ),
        MaskKind::Revealed => (
            Color::from_rgba8(255, 235, 162, 80),
            Color::from_rgba8(220, 30, 30, 255),
        ),
        MaskKind::Inactive => (
            Color::from_rgba8(255, 235, 162, 255),
            Color::from_rgba8(195, 160, 72, 255),
        ),
    }
}

/// Paints the masks that apply on this side over the image. Coordinates are fractions
/// of the image size; notes from before Anki normalised them use pixels, recognisable
/// by values above one.
pub fn apply(image: &DynamicImage, masks: &[Mask]) -> DynamicImage {
    let masks: Vec<&Mask> = masks.iter().filter(|mask| mask.painted()).collect();
    if masks.is_empty() {
        return image.clone();
    }
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let Some(size) = IntSize::from_wh(width, height) else {
        return image.clone();
    };
    let mut data = rgba.into_raw();
    for pixel in data.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * alpha) / 255) as u8;
        }
    }
    let Some(mut pixmap) = Pixmap::from_vec(data, size) else {
        return image.clone();
    };

    let (w, h) = (width as f32, height as f32);
    let edge_width = (w.max(h) / 300.0).clamp(2.0, 8.0);
    for mask in masks {
        let Some(path) = path_for(&mask.shape, w, h) else {
            continue;
        };
        let (fill, edge) = colours(mask.kind);
        let mut paint = Paint::default();
        paint.set_color(fill);
        paint.anti_alias = true;
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        paint.set_color(edge);
        let stroke = Stroke {
            width: edge_width,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    match RgbaImage::from_raw(width, height, pixmap.take_demultiplied()) {
        Some(painted) => DynamicImage::ImageRgba8(painted),
        None => image.clone(),
    }
}

fn path_for(shape: &Shape, w: f32, h: f32) -> Option<resvg::tiny_skia::Path> {
    // Fractions unless some value is clearly a pixel count.
    let scale = |values: &[f32]| {
        if values.iter().any(|v| *v > 1.0) {
            (1.0, 1.0)
        } else {
            (w, h)
        }
    };
    match shape {
        Shape::Rect {
            left,
            top,
            width,
            height,
        } => {
            let (sx, sy) = scale(&[*left, *top, *width, *height]);
            let rect = Rect::from_xywh(left * sx, top * sy, width * sx, height * sy)?;
            Some(PathBuilder::from_rect(rect))
        }
        Shape::Ellipse { left, top, rx, ry } => {
            let (sx, sy) = scale(&[*left, *top, *rx, *ry]);
            let rect = Rect::from_xywh(left * sx, top * sy, 2.0 * rx * sx, 2.0 * ry * sy)?;
            PathBuilder::from_oval(rect)
        }
        Shape::Polygon { points } => {
            let flat: Vec<f32> = points.iter().flat_map(|(x, y)| [*x, *y]).collect();
            let (sx, sy) = scale(&flat);
            let mut builder = PathBuilder::new();
            for (i, (x, y)) in points.iter().enumerate() {
                if i == 0 {
                    builder.move_to(x * sx, y * sy);
                } else {
                    builder.line_to(x * sx, y * sy);
                }
            }
            builder.close();
            builder.finish()
        }
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

    #[test]
    fn parses_rslibs_occlusion_elements() {
        let rect = attrs(&[
            ("data-ordinal", "1"),
            ("data-shape", "rect"),
            ("data-left", ".25"),
            ("data-top", ".5"),
            ("data-width", ".5"),
            ("data-height", ".25"),
            ("data-occludeinactive", "1"),
        ]);
        let mask = Mask::from_element("cloze", &rect).unwrap();
        assert_eq!(mask.kind, MaskKind::Hidden);
        assert!(mask.occlude_inactive);
        assert_eq!(
            mask.shape,
            Shape::Rect {
                left: 0.25,
                top: 0.5,
                width: 0.5,
                height: 0.25
            }
        );
        assert_eq!(
            Mask::from_element("cloze-highlight", &rect).unwrap().kind,
            MaskKind::Revealed
        );
        assert_eq!(
            Mask::from_element("cloze-inactive", &rect).unwrap().kind,
            MaskKind::Inactive
        );
        assert!(Mask::from_element("hint", &rect).is_none());

        let text = attrs(&[
            ("data-shape", "text"),
            ("data-left", ".1"),
            ("data-top", ".1"),
        ]);
        assert!(
            Mask::from_element("cloze", &text).is_none(),
            "labels are not masks"
        );

        let polygon = attrs(&[("data-shape", "polygon"), ("data-points", "0,0 1,0 1,1")]);
        assert!(matches!(
            Mask::from_element("cloze", &polygon).unwrap().shape,
            Shape::Polygon { ref points } if points.len() == 3
        ));
    }

    #[test]
    fn hidden_shapes_are_covered_and_inactive_ones_only_when_asked() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            100,
            100,
            image::Rgba([0, 0, 255, 255]),
        ));
        let rect = |left: f32, kind: MaskKind, occlude_inactive: bool| Mask {
            shape: Shape::Rect {
                left,
                top: 0.0,
                width: 0.5,
                height: 1.0,
            },
            kind,
            occlude_inactive,
        };

        let question = apply(
            &image,
            &[
                rect(0.0, MaskKind::Hidden, false),
                rect(0.5, MaskKind::Inactive, false),
            ],
        );
        let pixels = question.to_rgba8();
        assert_eq!(
            pixels.get_pixel(25, 50).0,
            [255, 142, 142, 255],
            "asked shape covered in red"
        );
        assert_eq!(
            pixels.get_pixel(75, 50).0,
            [0, 0, 255, 255],
            "inactive shape left alone"
        );

        let hide_all = apply(
            &image,
            &[
                rect(0.0, MaskKind::Hidden, true),
                rect(0.5, MaskKind::Inactive, true),
            ],
        );
        assert_eq!(
            hide_all.to_rgba8().get_pixel(75, 50).0,
            [255, 235, 162, 255],
            "inactive covered too"
        );

        let answer = apply(
            &image,
            &[
                rect(0.0, MaskKind::Revealed, true),
                rect(0.5, MaskKind::Inactive, true),
            ],
        );
        let pixels = answer.to_rgba8();
        let revealed = pixels.get_pixel(25, 50).0;
        assert!(
            revealed[2] > 150 && revealed[0] > 50,
            "revealed shape shows through a tint: {revealed:?}"
        );
        assert_eq!(pixels.get_pixel(75, 50).0, [255, 235, 162, 255]);

        assert_eq!(
            key(&[rect(0.5, MaskKind::Inactive, false)]),
            "",
            "unpainted masks do not change the key"
        );
        assert_ne!(
            key(&[rect(0.0, MaskKind::Hidden, false)]),
            key(&[rect(0.0, MaskKind::Revealed, false)])
        );
    }
}
