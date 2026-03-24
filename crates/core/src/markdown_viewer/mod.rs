pub mod html_mermaid;
pub mod html_preprocessor;
pub mod image_preprocessor;
pub mod mermaid;
pub mod plain_mermaid;
pub mod render;
pub mod themes;
pub mod utils;

use comrak::{
    Arena, markdown_to_html_with_plugins, options, plugins::syntect::SyntectAdapterBuilder,
};
use image_preprocessor::ImagePreprocessor;
use rasteroid::term_misc::{self, break_size_string};
use render::{AnsiContext, RESET, parse_node};
use syntect::{highlighting::ThemeSet, parsing::SyntaxSet};
use themes::CustomTheme;

use crate::{
    UnwrapOrExit,
    config::{McatConfig, MdMermaidRender},
};
use std::path::Path;

pub fn md_to_ansi(md: &str, config: &McatConfig, markdown_file_path: Option<&Path>) -> String {
    md_to_ansi_rendered(md, config, markdown_file_path).content
}

pub struct RenderedMarkdown {
    pub content: String,
    pub strict_failure: Option<String>,
}

pub(crate) fn md_to_ansi_rendered(
    md: &str,
    config: &McatConfig,
    markdown_file_path: Option<&Path>,
) -> RenderedMarkdown {
    let md = html_preprocessor::process(md);

    let arena = Arena::new();
    let opts = comrak_options();
    let root = comrak::parse_document(&arena, &md, &opts);

    // changing to forced inline in case of images rendered
    let _ = term_misc::init_wininfo(
        &break_size_string(&config.inline_options.spx).unwrap_or_exit(),
        &break_size_string(&config.inline_options.spx).unwrap_or_exit(),
        config.inline_options.scalex,
        config.inline_options.scaley,
        config.is_tmux,
        true,
    );

    let ps = SyntaxSet::load_defaults_newlines();
    let theme = CustomTheme::from(config.theme.as_ref());
    let image_preprocessor = ImagePreprocessor::new(root, config, markdown_file_path);
    let mut ctx = AnsiContext {
        config,
        ps,
        theme,
        hide_line_numbers: config.no_linenumbers,
        term_width: term_misc::get_wininfo().sc_width as usize,
        center: false,
        image_preprocessor: &image_preprocessor,
        show_frontmatter: config.yaml_header,
        md_mermaid_render: config.md_mermaid_render,
        strict_mermaid_failure: None,
        mermaid_image_elements: Vec::new(),
        next_inline_image_element_id: image_preprocessor.next_element_id,

        blockquote_fenced_offset: None,
        is_multi_block_quote: false,
        paragraph_collecting_line: None,
        collecting_depth: 0,
        under_header: false,
        force_simple_code_block: 0,
        list_depth: 0,
    };

    let mut output = String::new();
    output.push_str(&ctx.theme.foreground.fg);
    output.push_str(&parse_node(root, &mut ctx));

    let mut res = output.replace(RESET, &format!("{RESET}{}", &ctx.theme.foreground.fg));
    let strict_failure = ctx.strict_mermaid_failure.take();
    let mermaid_image_elements = std::mem::take(&mut ctx.mermaid_image_elements);
    drop(ctx);

    // replace images
    for (_, img) in image_preprocessor.mapper {
        img.insert_into_text(&mut res);
    }
    for img in mermaid_image_elements {
        img.insert_into_text(&mut res);
    }
    RenderedMarkdown {
        content: res,
        strict_failure,
    }
}

#[allow(dead_code)]
pub fn md_to_html(markdown: &str, style: Option<&str>) -> String {
    md_to_html_rendered(markdown, style, MdMermaidRender::Never).content
}

pub(crate) fn md_to_html_rendered(
    markdown: &str,
    style: Option<&str>,
    md_mermaid_render: MdMermaidRender,
) -> RenderedMarkdown {
    let options = comrak_options();
    let rewritten = html_mermaid::rewrite_markdown(markdown, md_mermaid_render, &options);

    let theme = CustomTheme::from(style.unwrap_or_default());
    let mut theme_set = ThemeSet::load_defaults();
    let mut plugins = options::Plugins::default();
    theme_set
        .themes
        .insert("dark".to_string(), theme.to_syntect_theme());
    let adapter = SyntectAdapterBuilder::new()
        .theme("dark")
        .theme_set(theme_set)
        .build();
    if style.is_some() {
        plugins.render.codefence_syntax_highlighter = Some(&adapter);
    }

    let full_css = match style {
        Some(_) => Some(theme.to_html_style()),
        None => None,
    };

    let html = markdown_to_html_with_plugins(&rewritten.markdown, &options, &plugins);
    let content = match full_css {
        Some(css) => format!(
            r#"
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <style>{}</style>
</head>
<body>
  {}
</body>
</html>
"#,
            css, html
        ),
        None => html,
    };
    RenderedMarkdown {
        content,
        strict_failure: rewritten.strict_failure,
    }
}

fn comrak_options<'a>() -> options::Options<'a> {
    let mut options = options::Options::default();
    options.extension.strikethrough = true;
    options.extension.footnotes = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());
    options.extension.superscript = true;
    options.extension.tagfilter = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.description_lists = true;
    options.extension.math_code = true;
    options.extension.alerts = true;
    options.extension.wikilinks_title_after_pipe = true;
    options.extension.spoiler = true;
    options.extension.multiline_block_quotes = true;

    options.parse.smart = true;
    options.parse.relaxed_tasklist_matching = true;

    options.render.r#unsafe = true;

    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose};
    use rasteroid::{InlineEncoder, term_misc};
    use regex::Regex;

    use crate::{
        config::{McatConfig, MdImageRender, MdMermaidRender},
        markdown_viewer::mermaid::with_image_render_failure_for_tests,
    };

    fn render(md: &str) -> String {
        render_with_mermaid_mode(md, MdMermaidRender::Never).content
    }

    fn render_raw_with_config(md: &str, config: McatConfig) -> RenderedMarkdown {
        md_to_ansi_rendered(md, &config, None)
    }

    fn render_with_config(md: &str, config: McatConfig) -> RenderedMarkdown {
        let mut rendered = render_raw_with_config(md, config);
        rendered.content = strip_ansi_escapes::strip_str(&rendered.content).to_string();
        rendered
    }

    fn config_with_inline_encoder(inline_encoder: InlineEncoder) -> McatConfig {
        let mut config = McatConfig::default();
        config.inline_encoder = inline_encoder;
        config.md_mermaid_render = MdMermaidRender::Auto;
        config
    }

    fn render_with_mermaid_mode(md: &str, mode: MdMermaidRender) -> RenderedMarkdown {
        let mut config = config_with_inline_encoder(InlineEncoder::Ascii);
        config.md_mermaid_render = mode;
        render_with_config(md, config)
    }

    fn md_to_html_with_mermaid_mode(
        md: &str,
        mode: MdMermaidRender,
        style: Option<&str>,
    ) -> RenderedMarkdown {
        md_to_html_rendered(md, style, mode)
    }

    #[test]
    fn list_item_with_code_block_on_separate_lines() {
        let md = "1. Step one:\n\n        echo hello\n";
        let output = render(md);

        // The code block header (file icon + "text") must not appear on the
        // same line as the list item text. Before the fix, collect() joined
        // block-level children with no separator, so "Step one:" and the code
        // block header ended up on one line.
        let step_line = output.lines().find(|l| l.contains("Step one"));
        assert!(step_line.is_some(), "should contain \'Step one\'");
        let step_line = step_line.unwrap();

        // \u{f15c} is the file icon used in code block headers
        assert!(
            !step_line.contains("\u{f15c}") && !step_line.contains("text"),
            "code block header should not be on the same line as list item text, got: {:?}",
            step_line,
        );
    }

    #[test]
    fn mermaid_code_block_renders_as_diagram_in_auto_mode() {
        let rendered = render_with_mermaid_mode(
            "```mermaid\nflowchart TD\nA-->B\n```",
            MdMermaidRender::Auto,
        );
        assert!(
            !rendered.content.contains("mermaid\n  flowchart TD"),
            "expected Mermaid fence to render as a diagram, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn mermaid_invalid_input_falls_back_to_code_rendering_in_auto_mode() {
        let rendered =
            render_with_mermaid_mode("```mermaid\nnot a diagram\n```", MdMermaidRender::Auto);
        assert!(
            rendered.content.contains("mermaid"),
            "expected fallback code block header for invalid Mermaid, got: {:?}",
            rendered.content
        );
        assert!(
            rendered.content.contains("not a diagram"),
            "expected fallback code block content for invalid Mermaid, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn mermaid_never_mode_preserves_plain_code_rendering() {
        let rendered = render_with_mermaid_mode(
            "```mermaid\nflowchart TD\nA-->B\n```",
            MdMermaidRender::Never,
        );
        assert!(
            rendered.content.contains("mermaid"),
            "expected plain Mermaid code rendering when mode is Never, got: {:?}",
            rendered.content
        );
        assert!(
            rendered.content.contains("flowchart TD"),
            "expected Mermaid source content when mode is Never, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn mermaid_invalid_input_marks_strict_failure_in_always_mode() {
        let rendered =
            render_with_mermaid_mode("```mermaid\nnot a diagram\n```", MdMermaidRender::Always);
        assert!(
            rendered.content.contains("Mermaid render failed"),
            "expected explicit Mermaid failure block, got: {:?}",
            rendered.content
        );
        assert!(
            rendered.strict_failure.is_some(),
            "expected strict failure marker in Always mode"
        );
    }

    #[test]
    fn non_mermaid_code_block_uses_standard_ansi_path() {
        let rendered =
            render_with_mermaid_mode("```rust\nfn main() {}\n```", MdMermaidRender::Auto);
        assert!(
            rendered.content.contains("rust"),
            "expected non-Mermaid fence header to use standard code-block path, got: {:?}",
            rendered.content
        );
        assert!(
            rendered.content.contains("fn main() {}"),
            "expected non-Mermaid fence content in ANSI output, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn html_renderer_emits_inline_svg_for_mermaid_in_auto_mode() {
        let rendered = md_to_html_with_mermaid_mode(
            "```mermaid\nflowchart TD\nA-->B\n```",
            MdMermaidRender::Auto,
            Some("github"),
        );
        assert!(
            rendered.content.contains("<svg"),
            "expected inline SVG for Mermaid in HTML auto mode, got: {:?}",
            rendered.content
        );
        assert!(
            !rendered.content.contains("language-mermaid"),
            "expected Mermaid code fence to be replaced, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn html_renderer_falls_back_to_code_block_on_invalid_mermaid_in_auto_mode() {
        let rendered = md_to_html_with_mermaid_mode(
            "```mermaid\nnot a diagram\n```",
            MdMermaidRender::Auto,
            Some("github"),
        );
        assert!(
            rendered.content.contains("language-mermaid"),
            "expected invalid Mermaid to stay as code block in auto mode, got: {:?}",
            rendered.content
        );
        assert!(
            rendered.strict_failure.is_none(),
            "auto mode should not mark strict Mermaid failure"
        );
    }

    #[test]
    fn html_renderer_marks_strict_failure_in_always_mode() {
        let rendered = md_to_html_with_mermaid_mode(
            "```mermaid\nnot a diagram\n```",
            MdMermaidRender::Always,
            Some("github"),
        );
        assert!(
            rendered.content.contains("Mermaid render failed"),
            "expected explicit Mermaid failure block, got: {:?}",
            rendered.content
        );
        assert!(
            rendered.strict_failure.is_some(),
            "expected strict Mermaid failure marker in Always mode"
        );
    }

    #[test]
    fn html_renderer_preserves_non_mermaid_code_block_path() {
        let rendered = md_to_html_with_mermaid_mode(
            "```rust\nfn main() {}\n```",
            MdMermaidRender::Auto,
            Some("github"),
        );
        assert!(
            rendered.content.contains("language-rust"),
            "expected non-Mermaid fence to remain in code-block HTML path, got: {:?}",
            rendered.content
        );
        assert!(
            !rendered.content.contains("class=\"mcat-mermaid\""),
            "expected non-Mermaid fence to skip Mermaid rewrite path, got: {:?}",
            rendered.content
        );
    }

    fn extract_iterm_payload_bytes(content: &str) -> Vec<u8> {
        let re = Regex::new(r"1337;File=inline=1;size=\d+:([A-Za-z0-9+/=]+)").unwrap();
        let payload = re
            .captures(content)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str())
            .expect("expected iTerm inline payload in output");

        general_purpose::STANDARD
            .decode(payload)
            .expect("expected valid base64 iTerm payload")
    }

    fn small_policy_height_px() -> u32 {
        term_misc::dim_to_px("1c", term_misc::SizeDirection::Height)
            .unwrap_or_default()
            .saturating_sub(1) as u32
    }

    #[test]
    fn mermaid_code_block_uses_inline_image_on_kitty_in_auto_mode() {
        let rendered = render_raw_with_config(
            "```mermaid\nflowchart TD\nA-->B\n```",
            config_with_inline_encoder(InlineEncoder::Kitty),
        );

        assert!(
            rendered.content.contains("\u{10eeee}"),
            "expected Kitty inline Mermaid image payload, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn mermaid_uses_ansi_when_md_image_is_none() {
        let mut config = config_with_inline_encoder(InlineEncoder::Kitty);
        config.md_image_render = MdImageRender::None;

        let rendered = render_with_config("```mermaid\nflowchart TD\nA-->B\n```", config);

        assert!(
            !rendered.content.contains("\u{10eeee}"),
            "expected no Kitty image payload when md-image is none, got: {:?}",
            rendered.content
        );
        assert!(
            !rendered.content.contains("```mermaid"),
            "expected Mermaid ANSI fallback rendering instead of raw code fence, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn mermaid_uses_ansi_when_fast_mode_disables_markdown_images() {
        let mut config = config_with_inline_encoder(InlineEncoder::Kitty);
        // --fast maps to md-image none in config parsing.
        config.md_image_render = MdImageRender::None;

        let rendered = render_with_config("```mermaid\nflowchart TD\nA-->B\n```", config);

        assert!(
            !rendered.content.contains("\u{10eeee}"),
            "expected no Kitty image payload when --fast disables markdown images, got: {:?}",
            rendered.content
        );
        assert!(
            !rendered.content.contains("```mermaid"),
            "expected Mermaid ANSI fallback rendering instead of raw code fence, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn mermaid_inline_image_uses_full_policy_when_md_image_is_all() {
        let mut config = config_with_inline_encoder(InlineEncoder::Iterm);
        config.md_image_render = MdImageRender::All;

        let rendered = render_raw_with_config("```mermaid\nflowchart TD\nA-->B\n```", config);
        let bytes = extract_iterm_payload_bytes(&rendered.content);
        let img = image::load_from_memory(&bytes).expect("expected decodable iTerm image payload");

        assert!(
            img.height() > small_policy_height_px(),
            "expected full-policy image height above small-policy cap, got {}",
            img.height()
        );
    }

    #[test]
    fn mermaid_inline_image_uses_small_policy_on_iterm() {
        let rendered = render_raw_with_config(
            "```mermaid\nflowchart TD\nA-->B\n```",
            config_with_inline_encoder(InlineEncoder::Iterm),
        );
        let bytes = extract_iterm_payload_bytes(&rendered.content);
        let img = image::load_from_memory(&bytes).expect("expected decodable iTerm image payload");

        assert!(
            img.height() <= small_policy_height_px(),
            "expected small-policy image height cap, got {}",
            img.height()
        );
    }

    #[test]
    fn mermaid_image_failure_falls_back_to_ansi_in_auto_mode() {
        let rendered = with_image_render_failure_for_tests(|| {
            render_with_config(
                "```mermaid\nflowchart TD\nA-->B\n```",
                config_with_inline_encoder(InlineEncoder::Kitty),
            )
        });

        assert!(
            rendered.strict_failure.is_none(),
            "expected no strict failure in auto mode, got: {:?}",
            rendered.strict_failure
        );
        assert!(
            !rendered.content.contains("Mermaid render failed"),
            "expected fallback rendering in auto mode, got: {:?}",
            rendered.content
        );
    }

    #[test]
    fn mermaid_image_failure_marks_strict_failure_in_always_mode() {
        let mut config = config_with_inline_encoder(InlineEncoder::Kitty);
        config.md_mermaid_render = MdMermaidRender::Always;
        config.md_image_render = MdImageRender::All;

        let rendered = with_image_render_failure_for_tests(|| {
            render_with_config("```mermaid\nflowchart TD\nA-->B\n```", config)
        });

        assert!(
            rendered.content.contains("Mermaid render failed"),
            "expected explicit Mermaid failure output in strict mode, got: {:?}",
            rendered.content
        );
        assert!(
            rendered.strict_failure.is_some(),
            "expected strict failure marker in Always mode"
        );
    }
}
