use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Write},
    ops::Deref,
    path::Path,
};

use comrak::nodes::{AstNode, NodeValue};
use image::{DynamicImage, GenericImageView, ImageFormat};
use itertools::Itertools;
use rasteroid::{
    InlineEncoder,
    image_extended::InlineImage,
    inline_an_image,
    term_misc::{self},
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use tempfile::NamedTempFile;

use crate::{
    config::{McatConfig, MdImageRender},
    converter::svg_to_image,
    scrapy::{MediaScrapeOptions, scrape_biggest_media},
};

use super::render::UNDERLINE_OFF;

fn is_local_path(url: &str) -> bool {
    !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("data:")
}

fn handle_local_image(
    path: &str,
    markdown_file_dir: Option<&Path>,
) -> Result<NamedTempFile, Box<dyn std::error::Error>> {
    let original_path = Path::new(path);

    let extension = original_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    // Try absolute or CWD-relative path first
    if original_path.exists() {
        let file_data = fs::read(original_path)?;
        let mut temp_file = NamedTempFile::with_suffix(&format!(".{}", extension))?;
        temp_file.write_all(&file_data)?;
        temp_file.flush()?;
        return Ok(temp_file);
    }

    // If that fails and we have a markdown file directory, try relative to that
    if let Some(md_dir) = markdown_file_dir {
        let relative_path = md_dir.join(path);
        if relative_path.exists() {
            let file_data = fs::read(&relative_path)?;
            let mut temp_file = NamedTempFile::with_suffix(&format!(".{}", extension))?;
            temp_file.write_all(&file_data)?;
            temp_file.flush()?;
            return Ok(temp_file);
        } else {
            return Err(format!(
                "Local image file not found: {} (tried {} and {})",
                path,
                path,
                relative_path.display()
            )
            .into());
        }
    }

    Err(format!("Local image file not found: {}", path).into())
}

pub struct ImagePreprocessor {
    pub mapper: HashMap<String, ImageElement>,
    pub next_element_id: usize,
}

pub(crate) fn markdown_image_render_mode(conf: &McatConfig) -> MdImageRender {
    if conf.md_image_render != MdImageRender::Auto {
        return conf.md_image_render;
    }

    match conf.inline_encoder {
        InlineEncoder::Kitty => MdImageRender::All,
        InlineEncoder::Iterm => MdImageRender::Small,
        InlineEncoder::Sixel => MdImageRender::Small,
        InlineEncoder::Ascii => MdImageRender::None,
    }
}

pub(crate) fn resize_markdown_image_for_inline(
    img: DynamicImage,
    render_mode: MdImageRender,
    width_override: Option<u16>,
    height_override: Option<u16>,
) -> Option<(Vec<u8>, u32)> {
    if render_mode == MdImageRender::None {
        return None;
    }

    let (width, height) = img.dimensions();
    let width = width_override.map(|v| v as u32).unwrap_or(width);
    let height = height_override.map(|v| v as u32).unwrap_or(height);
    let width_fm = if width as f32 > term_misc::get_wininfo().spx_width as f32 * 0.8 {
        "80%"
    } else {
        &format!("{width}px")
    };
    let height_fm = if render_mode == MdImageRender::Small {
        let px = term_misc::dim_to_px("1c", term_misc::SizeDirection::Height)
            .unwrap_or_default()
            .saturating_sub(1); // it ceils, so we must make sure 1c
        &format!("{px}px")
    } else if height as f32 > term_misc::get_wininfo().spx_height as f32 * 0.4 {
        "40%"
    } else {
        &format!("{height}px")
    };

    let (img, _, new_width, _) = img
        .resize_plus(Some(width_fm), Some(height_fm), false, false)
        .ok()?;
    Some((img, new_width))
}

impl ImagePreprocessor {
    pub fn new<'a>(
        node: &'a AstNode<'a>,
        conf: &McatConfig,
        markdown_file_path: Option<&Path>,
    ) -> Self {
        let mut urls = Vec::new();
        extract_image_urls(node, &mut urls);

        let render_mode = markdown_image_render_mode(conf);
        let markdown_dir = markdown_file_path.and_then(|p| p.parent());
        let mut scrape_opts = MediaScrapeOptions::default();
        scrape_opts.silent = conf.silent;
        scrape_opts.videos = false;
        scrape_opts.documents = false;
        scrape_opts.max_content_length = match render_mode {
            MdImageRender::All => None,
            _ => Some(50_000), // filter complex images - won't scale down good
        };

        let items: Vec<(&ImageUrl, Vec<u8>, u32)> = urls
            .par_iter()
            .filter_map(|url| {
                // fail everything early if needed.
                if render_mode == MdImageRender::None {
                    return None;
                }

                let tmp = if is_local_path(&url.base_url) {
                    handle_local_image(&url.base_url, markdown_dir).ok()?
                } else {
                    scrape_biggest_media(&url.base_url, &scrape_opts).ok()?
                };
                let img = render_image(tmp, url.width, url.height)?;
                let (img, new_width) =
                    resize_markdown_image_for_inline(img, render_mode, url.width, url.height)?;

                Some((url, img, new_width))
            })
            .collect();

        let mut mapper: HashMap<String, ImageElement> = HashMap::new();
        for (i, (url, img, width)) in items.iter().enumerate() {
            match try_encode_inline_image_element_from_bytes(img, i, conf, *width) {
                Ok(img) => {
                    mapper.insert(url.original_url.clone(), img);
                }
                Err(e) => {
                    if !conf.silent {
                        eprintln!("Failed to encode image '{}': {}", url.original_url, e);
                    }
                }
            }
        }

        ImagePreprocessor {
            mapper,
            // Reserve IDs for every markdown image candidate that reached inline encoding,
            // even if it deduped in the map or later failed to encode.
            next_element_id: items.len(),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn encode_inline_image_element(
    img: &DynamicImage,
    element_id: usize,
    conf: &McatConfig,
    width: u32,
) -> Option<ImageElement> {
    let mut png = Cursor::new(Vec::new());
    img.write_to(&mut png, ImageFormat::Png).ok()?;
    encode_inline_image_element_from_bytes(png.get_ref(), element_id, conf, width)
}

pub(crate) fn encode_inline_image_element_from_bytes(
    img: &[u8],
    element_id: usize,
    conf: &McatConfig,
    width: u32,
) -> Option<ImageElement> {
    try_encode_inline_image_element_from_bytes(img, element_id, conf, width).ok()
}

fn try_encode_inline_image_element_from_bytes(
    img: &[u8],
    element_id: usize,
    conf: &McatConfig,
    width: u32,
) -> Result<ImageElement, String> {
    let mut buffer = Vec::new();
    inline_an_image(img, &mut buffer, None, None, &conf.inline_encoder)
        .map_err(|e| e.to_string())?;
    let img_str = String::from_utf8(buffer).unwrap_or_default();
    Ok(ImageElement {
        is_ok: true,
        placeholder: create_placeholder(&img_str, element_id, &conf.inline_encoder, width),
        img: img_str,
    })
}

fn create_placeholder(img: &str, id: usize, inline_encoder: &InlineEncoder, width: u32) -> String {
    let fg_color = 16 + (id % 216);
    let bg_color = 16 + ((id / 216) % 216);

    let (width, height) = match inline_encoder {
        InlineEncoder::Kitty => {
            let placeholder = "\u{10EEEE}";
            let first_line = img.lines().next().unwrap_or("");
            let width = first_line.matches(placeholder).count();
            let count = img.lines().count();
            (width, count)
        }
        _ => {
            let width =
                term_misc::dim_to_cells(&format!("{width}px"), term_misc::SizeDirection::Width)
                    .unwrap_or(1) as usize;
            (width, 1)
        }
    };

    let line = format!(
        "\x1b[38;5;{}m\x1b[48;5;{}m{}\x1b[0m",
        fg_color,
        bg_color,
        "█".repeat(width)
    );
    vec![line; height].join("\n")
}

fn render_image(
    tmp: NamedTempFile,
    width: Option<u16>,
    height: Option<u16>,
) -> Option<DynamicImage> {
    let width = width.map(|v| v.to_string());
    let height = height.map(|v| v.to_string());
    let ext = tmp.path().extension().unwrap_or_default().to_string_lossy();
    let dyn_img = if ext == "svg" {
        let buf = fs::read(tmp).ok()?;
        svg_to_image(buf.as_slice(), width.as_deref(), height.as_deref()).ok()?
    } else if ImageFormat::from_extension(ext.deref()).is_some() {
        let buf = fs::read(tmp).ok()?;
        image::load_from_memory(&buf).ok()?
    } else {
        return None;
    };

    Some(dyn_img)
}

pub struct ImageElement {
    pub is_ok: bool,
    pub placeholder: String,
    pub img: String,
}

impl ImageElement {
    pub fn insert_into_text(&self, text: &mut String) {
        if !self.is_ok {
            return;
        }

        let img = self
            .img
            .lines()
            .map(|line| format!("{UNDERLINE_OFF}{}", line))
            .join("\n");
        let placeholder_line = self.placeholder.lines().nth(0).unwrap_or_default();

        for img_line in img.lines() {
            *text = text.replacen(placeholder_line, img_line, 1);
        }
    }
}

#[derive(Debug)]
struct ImageUrl {
    base_url: String,
    original_url: String,
    width: Option<u16>,
    height: Option<u16>,
}
fn extract_image_urls<'a>(node: &'a AstNode<'a>, urls: &mut Vec<ImageUrl>) {
    let data = node.data.borrow();

    if let NodeValue::Image(image_node) = &data.value {
        // regex for; <URL>#<Width>x<Height>
        // width and height are optional.
        let regex = Regex::new(r"^(.+?)(?:#(\d+)?x(\d+)?)?$").unwrap();
        if let Some(captures) = regex.captures(&image_node.url) {
            if let Some(base_url) = captures.get(1) {
                let width = captures.get(2).and_then(|v| v.as_str().parse::<u16>().ok());
                let height = captures.get(3).and_then(|v| v.as_str().parse::<u16>().ok());
                urls.push(ImageUrl {
                    base_url: base_url.as_str().to_owned(),
                    original_url: image_node.url.clone(),
                    width,
                    height,
                });
            }
        }
    }

    for child in node.children() {
        extract_image_urls(child, urls);
    }
}

#[cfg(test)]
mod tests {
    use comrak::Arena;
    use image::{Rgba, RgbaImage};
    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn kitty_placeholder_uses_payload_geometry() {
        let payload = "\u{10EEEE}\u{10EEEE}\n\u{10EEEE}\u{10EEEE}";
        let placeholder = create_placeholder(payload, 7, &InlineEncoder::Kitty, 20);

        assert_eq!(placeholder.lines().count(), 2);
    }

    #[test]
    fn insert_into_text_replaces_each_placeholder_line() {
        let element = ImageElement {
            is_ok: true,
            placeholder: "AAA\nAAA".into(),
            img: "one\ntwo".into(),
        };
        let mut text = "AAA\nAAA".to_string();

        element.insert_into_text(&mut text);

        assert_eq!(text, format!("{UNDERLINE_OFF}one\n{UNDERLINE_OFF}two"));
    }

    #[test]
    fn encode_inline_image_element_builds_placeholder_without_markdown_url() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255])));
        let mut conf = McatConfig::default();
        conf.inline_encoder = InlineEncoder::Kitty;

        let element = encode_inline_image_element(&image, 9, &conf, 16).unwrap();

        assert!(element.is_ok);
        assert!(!element.placeholder.is_empty());
        assert!(!element.img.is_empty());
    }

    #[test]
    fn byte_helper_matches_dynamic_helper_output() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(3, 2, Rgba([0, 255, 0, 255])));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        let mut conf = McatConfig::default();
        conf.inline_encoder = InlineEncoder::Kitty;

        let from_dynamic = encode_inline_image_element(&image, 3, &conf, 18).unwrap();
        let from_bytes =
            encode_inline_image_element_from_bytes(png.get_ref(), 3, &conf, 18).unwrap();
        let expected_base64 = term_misc::image_to_base64(png.get_ref());

        assert_eq!(from_dynamic.placeholder, from_bytes.placeholder);
        assert!(from_dynamic.img.contains(&expected_base64));
        assert!(from_bytes.img.contains(&expected_base64));
    }

    #[test]
    fn byte_helper_can_encode_without_decoding_image_bytes() {
        let mut conf = McatConfig::default();
        conf.inline_encoder = InlineEncoder::Iterm;

        let element = encode_inline_image_element_from_bytes(b"not-a-real-image", 5, &conf, 12)
            .expect("byte helper should not require decode/re-encode on markdown byte path");

        assert!(element.is_ok);
        assert!(element.img.contains("1337;File=inline=1"));
    }

    #[test]
    fn kitty_inline_byte_helper_uses_payload_geometry() {
        const CHILD_ENV: &str = "MCAT_TEST_KITTY_INLINE_PAYLOAD";
        if std::env::var(CHILD_ENV).as_deref() != Ok("1") {
            let test_name = "markdown_viewer::image_preprocessor::tests::kitty_inline_byte_helper_uses_payload_geometry";
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg(test_name)
                .arg("--nocapture")
                .env(CHILD_ENV, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "isolated inline child failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let spx = term_misc::Size {
            width: 1920,
            height: 1080,
            force: true,
        };
        let sc = term_misc::Size {
            width: 100,
            height: 20,
            force: true,
        };
        let _ = term_misc::init_wininfo(&spx, &sc, None, None, false, true);
        let wininfo = term_misc::get_wininfo();
        assert!(
            wininfo.needs_inline || wininfo.is_tmux,
            "kitty inline payload test requires inline-capable markdown rendering state"
        );

        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255])));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        let mut conf = McatConfig::default();
        conf.inline_encoder = InlineEncoder::Kitty;

        let element = encode_inline_image_element_from_bytes(png.get_ref(), 8, &conf, 24).unwrap();
        let img_placeholder = '\u{10EEEE}';

        assert!(
            element.img.contains(img_placeholder),
            "expected kitty inline payload placeholder marker"
        );
        let expected_rows = element.img.lines().count();
        let expected_cols = element
            .img
            .lines()
            .next()
            .unwrap_or_default()
            .matches(img_placeholder)
            .count();
        let actual_rows = element.placeholder.lines().count();
        let actual_cols = element
            .placeholder
            .lines()
            .next()
            .unwrap_or_default()
            .matches('█')
            .count();
        assert_eq!(actual_rows, expected_rows);
        assert_eq!(actual_cols, expected_cols);
    }

    #[test]
    fn next_element_id_counts_duplicate_markdown_image_candidates() {
        let file = NamedTempFile::with_suffix(".png").unwrap();
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 3, Rgba([1, 2, 3, 255])));
        image.save(file.path()).unwrap();

        let markdown = format!("![first]({0})\n\n![second]({0})\n", file.path().display());
        let arena = Arena::new();
        let options = super::super::comrak_options();
        let root = comrak::parse_document(&arena, &markdown, &options);
        let spx = term_misc::Size {
            width: 1920,
            height: 1080,
            force: true,
        };
        let sc = term_misc::Size {
            width: 100,
            height: 20,
            force: true,
        };
        let _ = term_misc::init_wininfo(&spx, &sc, None, None, false, true);

        let mut conf = McatConfig::default();
        conf.inline_encoder = InlineEncoder::Kitty;

        let preprocessor = ImagePreprocessor::new(root, &conf, None);

        assert_eq!(preprocessor.mapper.len(), 1);
        assert_eq!(preprocessor.next_element_id, 2);
    }
}
