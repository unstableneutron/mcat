use image::DynamicImage;
#[cfg(test)]
use std::cell::Cell;
use std::fmt;

use crate::converter;

#[derive(Debug)]
pub struct MermaidError {
    backend: &'static str,
    message: String,
}

impl MermaidError {
    fn new(backend: &'static str, message: impl Into<String>) -> Self {
        Self {
            backend,
            message: message.into(),
        }
    }
}

impl fmt::Display for MermaidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Mermaid render failed: {}",
            self.backend, self.message
        )
    }
}

impl std::error::Error for MermaidError {}

pub fn is_mermaid_info(info: &str) -> bool {
    info == "mermaid"
}

pub fn render_ansi_mermaid(source: &str) -> Result<String, MermaidError> {
    ensure_supported_diagram_kind(source, "ANSI")?;
    let config = console_mermaid::diagram::Config::default_config();
    console_mermaid::render_diagram(source, &config).map_err(|err| MermaidError::new("ANSI", err))
}

pub fn render_svg_mermaid(source: &str) -> Result<String, MermaidError> {
    ensure_supported_diagram_kind(source, "SVG")?;
    mermaid_rs_renderer::render(source).map_err(|err| MermaidError::new("SVG", err.to_string()))
}

pub fn render_image_mermaid(source: &str) -> Result<DynamicImage, MermaidError> {
    let svg = render_svg_mermaid(source)?;
    render_svg_as_image(&svg)
}

pub fn render_mermaid_html(source: &str) -> Result<String, MermaidError> {
    let svg = render_svg_mermaid(source)?;
    Ok(format!("<div class=\"mcat-mermaid\">{svg}</div>"))
}

fn render_svg_as_image(svg: &str) -> Result<DynamicImage, MermaidError> {
    if should_inject_image_render_failure() {
        return Err(MermaidError::new(
            "Image",
            "injected image conversion failure for tests",
        ));
    }

    converter::svg_to_image(svg.as_bytes(), None, None)
        .map_err(|err| MermaidError::new("Image", err.to_string()))
}

fn ensure_supported_diagram_kind(source: &str, backend: &'static str) -> Result<(), MermaidError> {
    if first_meaningful_line(source).is_some_and(is_supported_mermaid_header) {
        return Ok(());
    }
    Err(MermaidError::new(
        backend,
        "unsupported or missing Mermaid diagram header",
    ))
}

fn first_meaningful_line(source: &str) -> Option<&str> {
    source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
}

fn is_supported_mermaid_header(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "flowchart",
        "graph",
        "sequencediagram",
        "classdiagram",
        "statediagram",
        "statediagram-v2",
        "erdiagram",
        "pie",
        "xychart",
        "quadrantchart",
        "gantt",
        "timeline",
        "journey",
        "mindmap",
        "gitgraph",
        "requirementdiagram",
        "c4",
        "sankey",
        "zenuml",
        "block",
        "packet",
        "kanban",
        "architecture",
        "radar",
        "treemap",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

#[cfg(test)]
thread_local! {
    static IMAGE_RENDER_FAILURE_DEPTH_FOR_TESTS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn with_image_render_failure_for_tests<R>(operation: impl FnOnce() -> R) -> R {
    struct ScopedImageRenderFailureGuard;

    impl Drop for ScopedImageRenderFailureGuard {
        fn drop(&mut self) {
            IMAGE_RENDER_FAILURE_DEPTH_FOR_TESTS.with(|depth| {
                depth.set(depth.get().saturating_sub(1));
            });
        }
    }

    IMAGE_RENDER_FAILURE_DEPTH_FOR_TESTS.with(|depth| {
        depth.set(depth.get() + 1);
    });
    let _guard = ScopedImageRenderFailureGuard;
    operation()
}

fn should_inject_image_render_failure() -> bool {
    #[cfg(test)]
    {
        IMAGE_RENDER_FAILURE_DEPTH_FOR_TESTS.with(|depth| depth.get() > 0)
    }
    #[cfg(not(test))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_mermaid_info, render_ansi_mermaid, render_image_mermaid, render_mermaid_html,
        render_svg_mermaid, with_image_render_failure_for_tests,
    };

    const VALID_MERMAID: &str = "flowchart TD\nA-->B";

    #[test]
    fn graphic_renderer_returns_svg_for_valid_mermaid() {
        let svg = render_svg_mermaid(VALID_MERMAID).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn ansi_renderer_returns_non_empty_text_for_valid_mermaid() {
        let rendered = render_ansi_mermaid(VALID_MERMAID).unwrap();
        assert!(!rendered.trim().is_empty());
    }

    #[test]
    fn mermaid_info_string_detection_works() {
        assert!(is_mermaid_info("mermaid"));
        assert!(!is_mermaid_info("rust"));
        assert!(!is_mermaid_info(" mermaid"));
        assert!(!is_mermaid_info("mermaid "));
    }

    #[test]
    fn html_wrapper_returns_trusted_svg_container() {
        let html = render_mermaid_html(VALID_MERMAID).unwrap();
        assert!(html.contains("class=\"mcat-mermaid\""));
        assert!(html.contains("<svg"));
    }

    #[test]
    fn unsupported_header_is_rejected() {
        let err = render_svg_mermaid("not a diagram").unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported or missing Mermaid diagram header")
        );
    }

    #[test]
    fn image_renderer_returns_dynamic_image_for_valid_mermaid() {
        let img = render_image_mermaid(VALID_MERMAID).unwrap();
        assert!(img.width() > 0);
        assert!(img.height() > 0);
    }

    #[test]
    fn image_renderer_rejects_unsupported_header() {
        let err = render_image_mermaid("not a diagram").unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported or missing Mermaid diagram header")
        );
    }

    #[test]
    fn image_renderer_exposes_failure_injection_after_svg_render() {
        let err = with_image_render_failure_for_tests(|| render_image_mermaid(VALID_MERMAID))
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("injected image conversion failure for tests")
        );
    }

    #[test]
    fn image_renderer_failure_injection_does_not_leak_after_scope() {
        with_image_render_failure_for_tests(|| {
            let err = render_image_mermaid(VALID_MERMAID).unwrap_err();
            assert!(
                err.to_string()
                    .contains("injected image conversion failure for tests")
            );
        });

        let img = render_image_mermaid(VALID_MERMAID).unwrap();
        assert!(img.width() > 0);
        assert!(img.height() > 0);
    }
}
