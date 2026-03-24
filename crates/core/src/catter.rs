use std::{
    error::Error,
    fs::{self, File},
    io::{Cursor, Write, stdout},
    path::Path,
    process::{Command, Stdio},
};

use clap::error::Result;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode},
    tty::IsTty,
};
use image::{DynamicImage, ImageFormat};
use rasteroid::{
    InlineEncoder,
    image_extended::{InlineImage, ZoomPanViewport},
    term_misc,
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::{
    config::{McatConfig, MdImageRender, MdMermaidRender},
    converter::{self},
    image_viewer::{clear_screen, run_interactive_viewer, show_help_prompt},
    markdown_viewer,
};

pub enum CatType {
    Markdown,
    Pretty,
    Html,
    Image,
    Video,
    InlineImage,
    InlineVideo,
    Interactive,
}

pub fn get_album(path: &Path) -> Option<Vec<DynamicImage>> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    // pdf
    if matches!(ext.as_ref(), "pdf" | "tex" | "typ") && converter::get_pdf_command().is_ok() {
        let (path, _tmpfile, _tmpfolder) = converter::get_pdf(path);
        let images = converter::pdf_to_vec(&path.to_string_lossy().to_string()).ok()?;
        if !images.is_empty() {
            return Some(images);
        }
    }

    return None;
}

pub fn cat(
    paths: Vec<&Path>,
    out: &mut impl Write,
    opts: &McatConfig,
) -> Result<CatType, Box<dyn std::error::Error>> {
    let path = paths
        .get(0)
        .ok_or("This is most likely a bug - no paths are included in the cat function")?;

    //interactive mode
    if opts.output.clone().unwrap_or_default() == "interactive" {
        if paths.len() > 1 {
            let mut new_opts = opts.clone();
            new_opts.output = Some("image".to_owned());

            let images = paths
                .par_iter()
                .filter_map(|path| {
                    let mut buffer = Vec::new();
                    cat(vec![path], &mut buffer, &new_opts).ok()?;

                    let dyn_img = image::load_from_memory(&buffer).ok()?;
                    Some(dyn_img)
                })
                .collect();

            interact_with_image(images, opts, out)?;
            return Ok(CatType::Interactive);
        }
        if let Some(images) = get_album(path) {
            interact_with_image(images, opts, out)?;
            return Ok(CatType::Interactive);
        }
    }

    if !path.exists() {
        return Err(format!("invalid path: {}", path.display()).into());
    }

    let (result, from, to) = load(path, out, opts)?;
    let (string_result, image_result) = match result {
        LoadResult::Image(dynamic_image) => (None, Some(dynamic_image)),
        LoadResult::Text(text) => (Some(text), None),
        LoadResult::Handled(cat_type) => return Ok(cat_type),
    };

    // converting
    match (from.as_ref(), to.as_ref()) {
        ("md", "md") => {
            out.write_all(string_result.unwrap().as_bytes())?;
            Ok(CatType::Markdown)
        }
        ("md", "html") => {
            let rendered = markdown_viewer::md_to_html_rendered(
                &string_result.unwrap(),
                if opts.style_html {
                    Some(opts.theme.as_ref())
                } else {
                    None
                },
                opts.md_mermaid_render,
            );
            out.write_all(rendered.content.as_bytes())?;
            if let Some(err) = rendered.strict_failure {
                out.flush()?;
                return Err(err.into());
            }
            Ok(CatType::Html)
        }
        ("md", "image") => {
            let rendered = markdown_viewer::md_to_html_rendered(
                &string_result.unwrap(),
                Some(opts.theme.as_ref()),
                opts.md_mermaid_render,
            );
            let image = converter::html_to_image(&rendered.content)?;
            out.write_all(&image)?;
            if let Some(err) = rendered.strict_failure {
                out.flush()?;
                return Err(err.into());
            }
            Ok(CatType::Image)
        }
        ("md", "inline") => {
            let rendered = markdown_viewer::md_to_html_rendered(
                &string_result.unwrap(),
                Some(opts.theme.as_ref()),
                opts.md_mermaid_render,
            );
            let image = converter::html_to_image(&rendered.content)?;
            let dyn_img = image::load_from_memory(&image)?;
            print_image(out, dyn_img, opts)?;
            if let Some(err) = rendered.strict_failure {
                out.flush()?;
                return Err(err.into());
            }
            Ok(CatType::InlineImage)
        }
        ("md", "interactive") => {
            let rendered = markdown_viewer::md_to_html_rendered(
                &string_result.unwrap(),
                Some(opts.theme.as_ref()),
                opts.md_mermaid_render,
            );
            let img_bytes = converter::html_to_image(&rendered.content)?;
            let img = image::load_from_memory(&img_bytes)?;
            interact_with_image(vec![img], opts, out)?;
            if let Some(err) = rendered.strict_failure {
                out.flush()?;
                return Err(err.into());
            }
            Ok(CatType::Interactive)
        }
        ("html", "image") => {
            let image = converter::html_to_image(&string_result.unwrap())?;
            out.write_all(&image)?;
            Ok(CatType::Image)
        },
        ("html", "inline") => {
            let image = converter::html_to_image(&string_result.unwrap())?;
            let dyn_img = image::load_from_memory(&image)?;
            print_image(out, dyn_img, opts)?;
            Ok(CatType::InlineImage)
        },
        ("html", "interactive") => {
            let html = &string_result.unwrap();
            let img_bytes = converter::html_to_image(&html)?;
            let img = image::load_from_memory(&img_bytes)?;
            interact_with_image(vec![img], opts, out)?;
            Ok(CatType::Interactive)
        },
        ("image", "image") => {
            let img = image_result.unwrap();
            let mut cursor = Cursor::new(Vec::new());
            img.write_to(&mut cursor, ImageFormat::Png)?;
            out.write_all(&cursor.into_inner())?;
            Ok(CatType::Image)
        },
        ("image", "interactive") => {
            let img = image_result.unwrap();
            interact_with_image(vec![img], opts, out)?;
            Ok(CatType::Interactive)
        },
        ("md" | "html", _) => {
            //default for md, html
            let mut res = string_result.unwrap();
            if from == "html" {
                res = format!("```html\n{res}\n```");
            }
            let is_tty = stdout().is_tty();
            let use_color = opts.color.should_use(is_tty);
            let strict_mermaid_mode = matches!(opts.md_mermaid_render, MdMermaidRender::Always);
            let (content, strict_failure) = if strict_mermaid_mode {
                let rendered = if use_color {
                    markdown_viewer::md_to_ansi_rendered(&res, &opts, Some(path))
                } else {
                    let mut strict_opts = opts.clone();
                    strict_opts.md_image_render = MdImageRender::None;
                    markdown_viewer::md_to_ansi_rendered(&res, &strict_opts, Some(path))
                };
                let content = if use_color {
                    rendered.content
                } else {
                    strip_ansi_escapes::strip_str(&rendered.content).to_string()
                };
                (content, rendered.strict_failure)
            } else {
                match use_color {
                    true => (markdown_viewer::md_to_ansi(&res, &opts, Some(path)), None),
                    false if opts.md_mermaid_render.should_try_rendering() => (
                        markdown_viewer::plain_mermaid::rewrite_markdown(
                            &res,
                            opts.md_mermaid_render,
                        ),
                        None,
                    ),
                    false => (res, None),
                }
            };
            let use_pager = opts.paging.should_use(
                is_tty && content.lines().count() > term_misc::get_wininfo().sc_height as usize,
            );
            if use_pager {
                if let Some(pager) = Pager::new(opts.pager.as_ref()) {
                    if pager.page(&content).is_err() {
                        out.write_all(content.as_bytes())?;
                    }
                } else {
                    out.write_all(content.as_bytes())?;
                }
            } else {
                out.write_all(content.as_bytes())?;
            }
            if let Some(err) = strict_failure {
                out.flush()?;
                return Err(err.into());
            }

            if use_pager {
                Ok(CatType::Pretty)
            } else {
                Ok(CatType::Markdown)
            }
        },
        ("image", _) => {
            // default for image
            print_image(out, image_result.unwrap(), opts)?;
            Ok(CatType::InlineImage)
        },
        _ => Err(format!(
            "converting: {} to: {}, is not supported.\nsupported pipeline is: any -> md -> html -> image -> inline_image / interactive_image\nor video -> inline_video",
            from, to
        ).into()),
    }
}

pub enum LoadResult {
    Image(DynamicImage),
    Text(String),
    Handled(CatType),
}
pub fn load(
    path: &Path,
    out: &mut impl Write,
    opts: &McatConfig,
) -> Result<(LoadResult, String, String), Box<dyn std::error::Error>> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let to = opts.output.as_deref().unwrap_or("unknown").to_owned();

    //video
    if is_video(&ext) {
        if to == "video" {
            let content = fs::read(path)?;
            out.write_all(&content)?;

            let res = LoadResult::Handled(CatType::Video);
            return Ok((res, "video".to_owned(), to));
        }
        converter::inline_a_video(
            path.to_string_lossy(),
            out,
            &opts.inline_encoder,
            opts.inline_options.width.as_deref(),
            opts.inline_options.height.as_deref(),
            opts.inline_options.center,
            opts.silent,
        )?;

        let res = LoadResult::Handled(CatType::InlineVideo);
        return Ok((res, "video".to_owned(), to));
    }

    // pdf to images
    if matches!(ext.as_ref(), "pdf" | "tex" | "typ")
        && matches!(to.as_ref(), "inline" | "image" | "interactive")
        && converter::get_pdf_command().is_ok()
    {
        let (path, _tmpfile, _tmpfolder) = converter::get_pdf(path);

        // goes back to normal parsing if fails.
        if let Ok(img_data) = converter::pdf_to_image(&path.to_string_lossy().to_owned(), 1) {
            match to.as_ref() {
                "image" => {
                    let res = LoadResult::Handled(CatType::Image);
                    out.write_all(&img_data)?;
                    return Ok((res, "image".to_owned(), to));
                }
                _ => {
                    let dyn_img = image::load_from_memory(&img_data)?;
                    let res = LoadResult::Image(dyn_img);
                    return Ok((res, "image".to_owned(), to));
                }
            }
        }
    }

    //svg
    if ext == "svg" {
        let file = File::open(path)?;
        let dyn_img = converter::svg_to_image(
            file,
            opts.inline_options.width.as_deref(),
            opts.inline_options.height.as_deref(),
        )?;

        let res = LoadResult::Image(dyn_img);
        return Ok((res, "image".to_owned(), to));
    }

    // .url
    if ext == "url" {
        let dyn_img =
            converter::url_file_to_image(path).ok_or("Url file doesn't contain an icon")?;
        let res = LoadResult::Image(dyn_img);
        return Ok((res, "image".to_owned(), to));
    }

    // exe
    if ext == "exe" {
        let dyn_img = converter::exe_to_image(path).ok_or("Failed to get exe icon")?;
        let res = LoadResult::Image(dyn_img);
        return Ok((res, "image".to_owned(), to));
    }

    // lnk
    if ext == "lnk" {
        let dyn_img = converter::lnk_to_image(path).ok_or("Failed to get lnk icon")?;
        let res = LoadResult::Image(dyn_img);
        return Ok((res, "image".to_owned(), to));
    }

    //image
    if ImageFormat::from_extension(&ext).is_some() {
        let buf = fs::read(path)?;
        let dyn_img = image::load_from_memory(&buf)?;

        let res = LoadResult::Image(dyn_img);
        return Ok((res, "image".to_owned(), to));
    }

    // local file or dir
    match ext.as_ref() {
        "md" | "html" => {
            let r = fs::read_to_string(path)?;

            let res = LoadResult::Text(r);
            return Ok((res, ext, to));
        }
        _ => {
            let f = markdownify::convert(path)?;

            let res = LoadResult::Text(f);
            return Ok((res, "md".to_owned(), to));
        }
    }
}

fn print_image(
    out: &mut impl Write,
    dyn_img: DynamicImage,
    opts: &McatConfig,
) -> Result<(), Box<dyn Error>> {
    let resize_for_ascii = match opts.inline_encoder {
        rasteroid::InlineEncoder::Ascii => true,
        _ => false,
    };

    let dyn_img = apply_pan_zoom_once(dyn_img, &opts);
    let (img, center, _, _) = dyn_img.resize_plus(
        opts.inline_options.width.as_deref(),
        opts.inline_options.height.as_deref(),
        resize_for_ascii,
        false,
    )?;
    if opts.report {
        rasteroid::term_misc::report_size(
            &opts.inline_options.width.as_deref().unwrap_or(""),
            &opts.inline_options.height.as_deref().unwrap_or(""),
        );
    }
    rasteroid::inline_an_image(
        &img,
        out,
        if opts.inline_options.center {
            Some(center)
        } else {
            None
        },
        None,
        &opts.inline_encoder,
    )?;

    Ok(())
}

fn apply_pan_zoom_once(img: DynamicImage, opts: &McatConfig) -> DynamicImage {
    let zoom = opts.inline_options.zoom.unwrap_or(1);
    let x = opts.inline_options.x.unwrap_or_default();
    let y = opts.inline_options.y.unwrap_or_default();
    if zoom == 1 && x == 0 && y == 0 {
        return img;
    }

    let tinfo = term_misc::get_wininfo();
    let container_width = tinfo.spx_width as u32;
    let container_height = tinfo.spx_height as u32;
    let image_width = img.width();
    let image_height = img.height();

    let mut vp = ZoomPanViewport::new(container_width, container_height, image_width, image_height);
    vp.set_zoom(zoom);
    vp.set_pan(x, y);
    vp.apply_to_image(&img)
}

fn interact_with_image(
    images: Vec<DynamicImage>,
    opts: &McatConfig,
    out: &mut impl Write,
) -> Result<(), Box<dyn Error>> {
    if images.is_empty() {
        return Err("Most likely a bug - interact_with_image received 0 paths".into());
    }

    let mut img = &images[0];
    let tinfo = term_misc::get_wininfo();
    let container_width = tinfo.spx_width as u32;
    let container_height = tinfo.spx_height as u32;
    let image_width = img.width();
    let image_height = img.height();

    let resize_for_ascii = match opts.inline_encoder {
        rasteroid::InlineEncoder::Ascii => true,
        _ => false,
    };

    let height_cells = term_misc::dim_to_cells(
        opts.inline_options.height.as_deref().unwrap_or(""),
        term_misc::SizeDirection::Height,
    )?;
    let height = (tinfo.sc_height - 3).min(height_cells as u16);
    let should_disable_raw_mode = match opts.inline_encoder {
        InlineEncoder::Kitty => tinfo.is_tmux,
        InlineEncoder::Ascii => true,
        InlineEncoder::Iterm | InlineEncoder::Sixel => false,
    };
    let mut current_index = 0;
    let max_images = images.len();

    run_interactive_viewer(
        container_width,
        container_height,
        image_width,
        image_height,
        images.len() as u8,
        |vp, current_image| {
            if current_image != current_index {
                current_index = current_image;
                img = &images[current_image as usize];
                let width = img.width();
                let height = img.height();
                vp.update_image_size(width, height);
            }
            let new_img = vp.apply_to_image(&img);
            let (img, center, _, _) = new_img
                .resize_plus(
                    opts.inline_options.width.as_deref(),
                    Some(&format!("{height}c")),
                    resize_for_ascii,
                    false,
                )
                .ok()?;
            if should_disable_raw_mode {
                disable_raw_mode().ok()?;
            }
            let mut buf = Vec::new();
            rasteroid::inline_an_image(
                &img,
                &mut buf,
                if opts.inline_options.center {
                    Some(center)
                } else {
                    None
                },
                None,
                &opts.inline_encoder,
            )
            .ok()?;
            show_help_prompt(
                &mut buf,
                tinfo.sc_width,
                tinfo.sc_height,
                vp,
                current_image,
                max_images as u8,
            )
            .ok()?;
            clear_screen(out, Some(buf)).ok()?;
            out.flush().ok()?;
            if should_disable_raw_mode {
                enable_raw_mode().ok()?;
            }

            Some(())
        },
    )?;
    clear_screen(out, None)?;
    Ok(())
}

pub fn is_video(ext: &str) -> bool {
    matches!(
        ext,
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "wmv" | "flv" | "m4v" | "ts" | "gif"
    )
}

pub struct Pager {
    command: String,
    args: Vec<String>,
}

impl Pager {
    pub fn command_and_args_from_string(full: &str) -> Option<(String, Vec<String>)> {
        let parts = shell_words::split(full).ok()?;
        let (cmd, args) = parts.split_first()?;
        return Some((cmd.clone(), args.to_vec()));
    }
    pub fn new(def_command: &str) -> Option<Self> {
        let (command, args) = Pager::command_and_args_from_string(def_command)?;
        if which::which(&command).is_ok() {
            return Some(Self { command, args });
        }
        None
    }

    pub fn page(&self, content: &str) -> Result<(), Box<dyn Error>> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .spawn()?;

        if let Some(stdin) = child.stdin.as_mut() {
            // ignoring cuz the pipe will break when the user quits most likely
            let _ = stdin.write_all(content.as_bytes());
        }

        child.wait()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use tempfile::NamedTempFile;

    use super::cat;
    use crate::{
        config::{AlwaysOrNever, McatConfig, MdImageRender, MdMermaidRender},
        markdown_viewer::mermaid::with_image_render_failure_for_tests,
    };

    use super::InlineEncoder;

    fn temp_markdown(contents: &str) -> NamedTempFile {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file
    }

    fn strip_output(out: &[u8]) -> String {
        let plain = String::from_utf8_lossy(out).to_string();
        strip_ansi_escapes::strip_str(&plain).to_string()
    }

    fn inline_mermaid_opts(encoder: InlineEncoder) -> McatConfig {
        let mut opts = McatConfig::default();
        opts.inline_encoder = encoder;
        opts.color = AlwaysOrNever::Always;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Auto;
        opts.md_image_render = MdImageRender::All;
        opts
    }

    struct FlushTrackingWriter {
        bytes: Vec<u8>,
        flush_calls: usize,
    }

    impl FlushTrackingWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                flush_calls: 0,
            }
        }

        fn content(&self) -> String {
            String::from_utf8_lossy(&self.bytes).to_string()
        }
    }

    impl Write for FlushTrackingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flush_calls += 1;
            Ok(())
        }
    }

    #[test]
    fn strict_mermaid_failure_flushes_output_before_returning_error() {
        let file = temp_markdown("```mermaid\nnot a diagram\n```");
        let mut opts = McatConfig::default();
        opts.color = AlwaysOrNever::Always;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Always;

        let mut out = FlushTrackingWriter::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = strip_ansi_escapes::strip_str(&out.content()).to_string();
        assert!(
            rendered.contains("Mermaid render failed"),
            "expected explicit Mermaid failure output, got: {:?}",
            rendered
        );
        assert!(
            out.flush_calls > 0,
            "expected writer to be flushed before strict error return"
        );
        assert!(
            result.is_err(),
            "strict Mermaid failure should return an error"
        );
    }

    #[test]
    fn strict_mermaid_failure_in_color_never_still_errors_with_plain_text_output() {
        let file = temp_markdown("```mermaid\nnot a diagram\n```");
        let mut opts = McatConfig::default();
        opts.color = AlwaysOrNever::Never;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Always;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = String::from_utf8_lossy(&out);
        assert!(
            rendered.contains("Mermaid render failed"),
            "expected explicit Mermaid failure output, got: {:?}",
            rendered
        );
        assert!(
            !rendered.contains("\u{1b}["),
            "expected plain text output when color is disabled, got: {:?}",
            rendered
        );
        assert!(
            result.is_err(),
            "strict Mermaid failure should return an error even with --color never"
        );
    }

    #[test]
    fn mermaid_auto_mode_in_color_never_renders_plain_text_diagram() {
        let file = temp_markdown("```mermaid\nflowchart TD\n  A[Start] --> B[End]\n```");
        let mut opts = McatConfig::default();
        opts.color = AlwaysOrNever::Never;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Auto;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = String::from_utf8_lossy(&out);
        assert!(
            result.is_ok(),
            "auto Mermaid rendering should succeed with --color never"
        );
        assert!(
            rendered.contains("Start"),
            "expected rendered Mermaid content in output, got: {:?}",
            rendered
        );
        assert!(
            rendered.contains("End"),
            "expected rendered Mermaid content in output, got: {:?}",
            rendered
        );
        assert!(
            !rendered.contains("```mermaid"),
            "expected rendered Mermaid output instead of raw fence, got: {:?}",
            rendered
        );
        assert!(
            !rendered.trim_end().ends_with('`'),
            "expected Mermaid replacement to consume the full fence, got: {:?}",
            rendered
        );
        assert!(
            !rendered.contains("\u{1b}["),
            "expected plain text output when color is disabled, got: {:?}",
            rendered
        );
    }

    #[test]
    fn mermaid_auto_mode_in_color_never_preserves_non_mermaid_markdown() {
        let file = temp_markdown(
            "# Heading\n\n```mermaid\nflowchart TD\n  A[Start] --> B[End]\n```\n\n```rust\nfn main() {}\n```",
        );
        let mut opts = McatConfig::default();
        opts.color = AlwaysOrNever::Never;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Auto;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = String::from_utf8_lossy(&out);
        assert!(
            result.is_ok(),
            "auto Mermaid rendering should succeed with --color never"
        );
        assert!(
            rendered.contains("# Heading"),
            "expected raw non-Mermaid markdown to be preserved, got: {:?}",
            rendered
        );
        assert!(
            rendered.contains("```rust\nfn main() {}\n```"),
            "expected non-Mermaid fences to remain raw markdown, got: {:?}",
            rendered
        );
        assert!(
            !rendered.contains("```mermaid"),
            "expected Mermaid fence to be replaced, got: {:?}",
            rendered
        );
    }

    #[test]
    fn mermaid_auto_mode_in_color_never_preserves_blockquote_prefixes() {
        let file = temp_markdown("> ```mermaid\n> flowchart TD\n>   A[Start] --> B[End]\n> ```");
        let mut opts = McatConfig::default();
        opts.color = AlwaysOrNever::Never;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Auto;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = String::from_utf8_lossy(&out);
        assert!(
            result.is_ok(),
            "auto Mermaid rendering should succeed with --color never"
        );
        for line in rendered.lines().filter(|line| !line.is_empty()) {
            assert!(
                line.starts_with("> "),
                "expected blockquote prefix on every rendered line, got: {:?}",
                rendered
            );
        }
    }

    #[test]
    fn mermaid_auto_mode_in_color_never_preserves_list_indentation() {
        let file = temp_markdown(
            "- item\n\n    ```mermaid\n    flowchart TD\n      A[Start] --> B[End]\n    ```",
        );
        let mut opts = McatConfig::default();
        opts.color = AlwaysOrNever::Never;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Auto;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = String::from_utf8_lossy(&out);
        assert!(
            result.is_ok(),
            "auto Mermaid rendering should succeed with --color never"
        );
        assert!(
            rendered.contains("- item"),
            "expected list item text to be preserved, got: {:?}",
            rendered
        );
        for line in rendered.lines().skip(2).filter(|line| !line.is_empty()) {
            assert!(
                line.starts_with("    "),
                "expected list indentation on every rendered diagram line, got: {:?}",
                rendered
            );
        }
    }

    #[test]
    fn strict_mermaid_failure_in_html_output_flushes_before_error() {
        let file = temp_markdown("```mermaid\nnot a diagram\n```");
        let mut opts = McatConfig::default();
        opts.output = Some("html".to_owned());
        opts.md_mermaid_render = MdMermaidRender::Always;

        let mut out = FlushTrackingWriter::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = out.content();
        assert!(
            rendered.contains("Mermaid render failed"),
            "expected Mermaid failure block in HTML output, got: {:?}",
            rendered
        );
        assert!(
            rendered.contains("mcat-mermaid-failure"),
            "expected Mermaid failure wrapper in HTML output, got: {:?}",
            rendered
        );
        assert!(
            out.flush_calls > 0,
            "expected writer to be flushed before strict Mermaid error return"
        );
        assert!(
            result.is_err(),
            "strict Mermaid failure should return an error in HTML output"
        );
    }

    #[test]
    fn mermaid_inline_image_in_blockquote_preserves_renderable_output() {
        let file = temp_markdown("> ```mermaid\n> flowchart TD\n>   A[Start] --> B[End]\n> ```");
        let opts = inline_mermaid_opts(InlineEncoder::Kitty);

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);
        let rendered = strip_output(&out);

        assert!(result.is_ok(), "expected Mermaid render success");
        assert!(
            rendered.contains('\u{10eeee}'),
            "expected Kitty inline payload in blockquote output, got: {:?}",
            rendered
        );
        assert!(
            rendered.lines().any(|line| line.starts_with('▌')),
            "expected blockquote guide prefix in output, got: {:?}",
            rendered
        );
    }

    #[test]
    fn mermaid_inline_image_in_list_item_preserves_renderable_output() {
        let file = temp_markdown(
            "- item\n\n    ```mermaid\n    flowchart TD\n      A[Start] --> B[End]\n    ```",
        );
        let opts = inline_mermaid_opts(InlineEncoder::Kitty);

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);
        let rendered = strip_output(&out);

        assert!(result.is_ok(), "expected Mermaid render success");
        assert!(
            rendered.contains('\u{10eeee}'),
            "expected Kitty inline payload in list output, got: {:?}",
            rendered
        );
        assert!(
            rendered.contains("item"),
            "expected list item text to be preserved, got: {:?}",
            rendered
        );
    }

    #[test]
    fn mermaid_ansi_fallback_in_blockquote_preserves_layout() {
        let file = temp_markdown("> ```mermaid\n> flowchart TD\n>   A[Start] --> B[End]\n> ```");
        let mut opts = inline_mermaid_opts(InlineEncoder::Kitty);
        opts.md_image_render = MdImageRender::None;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);
        let rendered = strip_output(&out);

        assert!(result.is_ok(), "expected Mermaid fallback success");
        assert!(
            !rendered.contains('\u{10eeee}'),
            "expected ANSI fallback without inline image payload, got: {:?}",
            rendered
        );
        assert!(
            rendered.lines().any(|line| line.starts_with('▌')),
            "expected blockquote guide prefix in fallback output, got: {:?}",
            rendered
        );
    }

    #[test]
    fn mermaid_plain_fallback_in_list_item_preserves_layout() {
        let file = temp_markdown(
            "- item\n\n    ```mermaid\n    flowchart TD\n      A[Start] --> B[End]\n    ```",
        );
        let mut opts = McatConfig::default();
        opts.color = AlwaysOrNever::Never;
        opts.paging = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Auto;
        opts.md_image_render = MdImageRender::None;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);

        let rendered = String::from_utf8_lossy(&out);
        assert!(
            result.is_ok(),
            "expected plain Mermaid fallback success with --color never"
        );
        assert!(
            rendered.contains("- item"),
            "expected list item text to be preserved, got: {:?}",
            rendered
        );
        for line in rendered.lines().skip(2).filter(|line| !line.is_empty()) {
            assert!(
                line.starts_with("    "),
                "expected list indentation on fallback rendered lines, got: {:?}",
                rendered
            );
        }
    }

    #[test]
    fn mermaid_image_failure_falls_back_in_auto_mode() {
        let file = temp_markdown("```mermaid\nflowchart TD\nA-->B\n```");
        let opts = inline_mermaid_opts(InlineEncoder::Kitty);

        let mut out = Vec::new();
        let result =
            with_image_render_failure_for_tests(|| cat(vec![file.path()], &mut out, &opts));
        let rendered = strip_output(&out);

        assert!(
            result.is_ok(),
            "expected auto mode to fall back instead of erroring on image branch failure"
        );
        assert!(
            !rendered.contains("Mermaid render failed"),
            "expected fallback rendering in auto mode, got: {:?}",
            rendered
        );
    }

    #[test]
    fn mermaid_image_failure_is_strict_in_always_mode() {
        let file = temp_markdown("```mermaid\nflowchart TD\nA-->B\n```");
        let mut opts = inline_mermaid_opts(InlineEncoder::Kitty);
        opts.md_mermaid_render = MdMermaidRender::Always;

        let mut out = Vec::new();
        let result =
            with_image_render_failure_for_tests(|| cat(vec![file.path()], &mut out, &opts));
        let rendered = strip_output(&out);

        assert!(
            rendered.contains("Mermaid render failed"),
            "expected explicit Mermaid failure output in strict mode, got: {:?}",
            rendered
        );
        assert!(
            result.is_err(),
            "expected strict Mermaid mode to return an error on image branch failure"
        );
    }

    #[test]
    fn strict_mermaid_success_in_color_never_uses_plain_diagram_output() {
        let file = temp_markdown("```mermaid\nflowchart TD\n  A[Start] --> B[End]\n```");
        let mut opts = inline_mermaid_opts(InlineEncoder::Kitty);
        opts.color = AlwaysOrNever::Never;
        opts.md_mermaid_render = MdMermaidRender::Always;

        let mut out = Vec::new();
        let result = cat(vec![file.path()], &mut out, &opts);
        let rendered = String::from_utf8_lossy(&out);

        assert!(result.is_ok(), "expected strict Mermaid render success");
        assert!(
            rendered.contains("Start"),
            "expected plain Mermaid diagram text, got: {:?}",
            rendered
        );
        assert!(
            rendered.contains("End"),
            "expected plain Mermaid diagram text, got: {:?}",
            rendered
        );
        assert!(
            !rendered.contains('\u{10eeee}'),
            "expected no Kitty placeholder glyphs in --color never output, got: {:?}",
            rendered
        );
        assert!(
            !rendered.contains("\u{1b}["),
            "expected plain text output when color is disabled, got: {:?}",
            rendered
        );
    }

    #[test]
    fn md_output_image_branch_still_uses_html_screenshot_path() {
        let source = include_str!("catter.rs");

        assert!(
            source.contains("(\"md\", \"image\") =>"),
            "expected dedicated markdown->image conversion branch"
        );
        assert!(
            source.contains("markdown_viewer::md_to_html_rendered"),
            "expected markdown->image branch to keep full-document HTML rendering path"
        );
        assert!(
            source.contains("converter::html_to_image(&rendered.content)?"),
            "expected markdown->image branch to keep browser screenshot conversion path"
        );
    }

    #[test]
    fn md_output_inline_branch_still_uses_html_screenshot_path() {
        let source = include_str!("catter.rs");

        assert!(
            source.contains("(\"md\", \"inline\") =>"),
            "expected dedicated markdown->inline conversion branch"
        );
        assert!(
            source.contains("converter::html_to_image(&rendered.content)?"),
            "expected markdown->inline branch to keep browser screenshot conversion path"
        );
    }

    #[test]
    fn md_output_interactive_branch_still_uses_html_screenshot_path() {
        let source = include_str!("catter.rs");

        assert!(
            source.contains("(\"md\", \"interactive\") =>"),
            "expected dedicated markdown->interactive conversion branch"
        );
        assert!(
            source.contains("converter::html_to_image(&rendered.content)?"),
            "expected markdown->interactive branch to keep browser screenshot conversion path"
        );
    }
}
