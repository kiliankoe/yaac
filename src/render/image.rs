//! Media files as bitmaps: raster formats through `image`, SVG rasterised with resvg.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, RgbaImage};
use resvg::usvg;

/// Longest side to rasterise SVGs at. Terminal cells are roughly ten pixels wide, so
/// this comfortably covers a full-width image without wasting work.
const SVG_TARGET_PX: f32 = 1024.0;

/// Resolves `src` from an `<img>` tag: normally a file in the media folder, but
/// absolute paths pasted from elsewhere are honoured when they exist.
pub fn resolve(media_dir: &Path, src: &str) -> PathBuf {
    let direct = Path::new(src);
    if direct.is_absolute() && direct.is_file() {
        return direct.to_path_buf();
    }
    media_dir.join(src)
}

pub fn load(media_dir: &Path, src: &str) -> Result<DynamicImage> {
    let path = resolve(media_dir, src);
    if !path.is_file() {
        bail!("{src} is not in the media folder");
    }
    let is_svg = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz"));
    if is_svg {
        rasterise_svg(&path, media_dir)
    } else {
        // Animated GIFs decode to their first frame, which is all a card needs.
        image::open(&path).with_context(|| format!("decoding {src}"))
    }
}

/// System fonts are scanned once per process; SVG text is dropped by resvg when no
/// fonts are loaded at all.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

fn rasterise_svg(path: &Path, media_dir: &Path) -> Result<DynamicImage> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let options = usvg::Options {
        resources_dir: Some(media_dir.to_path_buf()),
        font_family: "Helvetica".to_string(),
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("parsing SVG {}", path.display()))?;
    let size = tree.size();
    // Vectors scale losslessly, so always draw at the target size; the floor keeps
    // huge documents from shrinking into illegibility.
    let scale = (SVG_TARGET_PX / size.width().max(size.height())).max(0.25);
    let width = (size.width() * scale).ceil().clamp(1.0, 4096.0) as u32;
    let height = (size.height() * scale).ceil().clamp(1.0, 4096.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .with_context(|| format!("SVG {} has no drawable size", path.display()))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = RgbaImage::from_raw(width, height, pixmap.take_demultiplied())
        .context("SVG pixel buffer has the wrong size")?;
    Ok(DynamicImage::ImageRgba8(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_raster_files_and_rasterises_svg() {
        let dir = tempfile::tempdir().unwrap();
        let png = RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]));
        png.save(dir.path().join("dot.png")).unwrap();
        std::fs::write(
            dir.path().join("flag.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
                 <rect width="20" height="10" fill="#ff0000"/></svg>"##,
        )
        .unwrap();

        let raster = load(dir.path(), "dot.png").unwrap();
        assert_eq!((raster.width(), raster.height()), (3, 2));

        let svg = load(dir.path(), "flag.svg").unwrap();
        assert_eq!(svg.width(), 1024, "scaled up to the target size");
        assert_eq!(svg.height(), 512, "aspect ratio kept");
        let pixel = svg.to_rgba8().get_pixel(10, 10).0;
        assert_eq!(pixel, [255, 0, 0, 255]);

        let missing = load(dir.path(), "nope.png").unwrap_err();
        assert!(missing.to_string().contains("not in the media folder"));
    }

    #[test]
    fn absolute_paths_are_used_only_when_they_exist() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        let pasted = dir.path().join("pasted.png");
        std::fs::write(&pasted, b"not really a png").unwrap();

        assert_eq!(resolve(&media, pasted.to_str().unwrap()), pasted);
        assert_eq!(
            resolve(&media, "/nonexistent/x.png"),
            PathBuf::from("/nonexistent/x.png"),
            "a missing absolute path stays absolute and fails to load"
        );
        assert_eq!(resolve(&media, "x.png"), media.join("x.png"));
    }
}
