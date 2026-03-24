use std::fmt;

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
    let config = console_mermaid::diagram::Config::default_config();
    console_mermaid::render_diagram(source, &config).map_err(|err| MermaidError::new("ANSI", err))
}

pub fn render_svg_mermaid(source: &str) -> Result<String, MermaidError> {
    mermaid_rs_renderer::render(source).map_err(|err| MermaidError::new("SVG", err.to_string()))
}

pub fn render_mermaid_html(source: &str) -> Result<String, MermaidError> {
    let svg = render_svg_mermaid(source)?;
    Ok(format!("<div class=\"mcat-mermaid\">{svg}</div>"))
}

#[cfg(test)]
mod tests {
    use super::{is_mermaid_info, render_ansi_mermaid, render_mermaid_html, render_svg_mermaid};

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
}
