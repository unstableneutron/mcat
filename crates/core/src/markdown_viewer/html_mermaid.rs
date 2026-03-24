use comrak::{
    Arena,
    nodes::{AstNode, NodeValue},
    options::Options,
};

use crate::config::MdMermaidRender;

use super::mermaid;

pub(crate) struct HtmlMermaidRewrite {
    pub markdown: String,
    pub strict_failure: Option<String>,
}

struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

pub(crate) fn rewrite_markdown(
    markdown: &str,
    mode: MdMermaidRender,
    options: &Options<'_>,
) -> HtmlMermaidRewrite {
    if !mode.should_try_rendering() {
        return HtmlMermaidRewrite {
            markdown: markdown.to_owned(),
            strict_failure: None,
        };
    }

    let arena = Arena::new();
    let root = comrak::parse_document(&arena, markdown, options);
    let line_starts = line_start_offsets(markdown);

    let mut replacements = Vec::new();
    let mut strict_failure = None;
    collect_replacements(
        root,
        markdown,
        &line_starts,
        mode,
        &mut replacements,
        &mut strict_failure,
    );

    if replacements.is_empty() {
        return HtmlMermaidRewrite {
            markdown: markdown.to_owned(),
            strict_failure,
        };
    }

    replacements.sort_by_key(|replacement| replacement.start);

    let mut rewritten = String::with_capacity(markdown.len());
    let mut cursor = 0usize;
    for replacement in replacements {
        if replacement.start < cursor
            || replacement.end < replacement.start
            || replacement.end > markdown.len()
        {
            continue;
        }
        rewritten.push_str(&markdown[cursor..replacement.start]);
        rewritten.push_str(&replacement.text);
        cursor = replacement.end;
    }
    rewritten.push_str(&markdown[cursor..]);

    HtmlMermaidRewrite {
        markdown: rewritten,
        strict_failure,
    }
}

fn collect_replacements<'a>(
    node: &'a AstNode<'a>,
    markdown: &str,
    line_starts: &[usize],
    mode: MdMermaidRender,
    replacements: &mut Vec<Replacement>,
    strict_failure: &mut Option<String>,
) {
    if let Some(replacement) =
        replacement_for_node(node, markdown, line_starts, mode, strict_failure)
    {
        replacements.push(replacement);
    }

    for child in node.children() {
        collect_replacements(
            child,
            markdown,
            line_starts,
            mode,
            replacements,
            strict_failure,
        );
    }
}

fn replacement_for_node<'a>(
    node: &'a AstNode<'a>,
    markdown: &str,
    line_starts: &[usize],
    mode: MdMermaidRender,
    strict_failure: &mut Option<String>,
) -> Option<Replacement> {
    let data = node.data.borrow();
    let NodeValue::CodeBlock(code_block) = &data.value else {
        return None;
    };
    if !mermaid::is_mermaid_info(&code_block.info) {
        return None;
    }

    let (start, end) = sourcepos_to_range(markdown, line_starts, data.sourcepos)?;
    let replacement_text = match mermaid::render_mermaid_html(&code_block.literal) {
        Ok(rendered) => rendered,
        Err(err) if mode.is_strict() => {
            let failure = err.to_string();
            if strict_failure.is_none() {
                *strict_failure = Some(failure.clone());
            }
            render_mermaid_failure_html(&failure)
        }
        Err(_) => return None,
    };

    Some(Replacement {
        start,
        end,
        text: replacement_text,
    })
}

fn sourcepos_to_range(
    markdown: &str,
    line_starts: &[usize],
    sourcepos: comrak::nodes::Sourcepos,
) -> Option<(usize, usize)> {
    let start = line_col_to_byte(
        markdown,
        line_starts,
        sourcepos.start.line,
        sourcepos.start.column,
    )?;
    let end_column = sourcepos.end.column.saturating_add(1);
    let end = line_col_to_byte(markdown, line_starts, sourcepos.end.line, end_column)?;
    if end < start {
        return None;
    }
    Some((start, end))
}

fn line_start_offsets(markdown: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, b) in markdown.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn line_col_to_byte(
    markdown: &str,
    line_starts: &[usize],
    line: usize,
    column: usize,
) -> Option<usize> {
    if line == 0 || column == 0 {
        return None;
    }
    let start = *line_starts.get(line.saturating_sub(1))?;
    let end = match line_starts.get(line) {
        Some(next_line_start) => next_line_start.saturating_sub(1),
        None => markdown.len(),
    };

    let mut col = 1usize;
    let mut offset = start;
    while col < column {
        if offset >= end {
            return Some(end);
        }
        let ch = markdown[offset..end].chars().next()?;
        offset += ch.len_utf8();
        col += 1;
    }

    Some(offset)
}

fn render_mermaid_failure_html(message: &str) -> String {
    format!(
        "<div class=\"mcat-mermaid-failure\"><p><strong>Mermaid render failed</strong></p><pre><code>{}</code></pre></div>",
        escape_html(message)
    )
}

fn escape_html(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&#39;".to_owned(),
            _ => ch.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::rewrite_markdown;
    use crate::{config::MdMermaidRender, markdown_viewer::comrak_options};

    #[test]
    fn rewrites_valid_mermaid_fence_into_html() {
        let options = comrak_options();
        let rewritten = rewrite_markdown(
            "```mermaid\nflowchart TD\nA-->B\n```",
            MdMermaidRender::Auto,
            &options,
        );
        assert!(rewritten.markdown.contains("<svg"));
        assert!(rewritten.strict_failure.is_none());
    }

    #[test]
    fn strict_mode_rewrites_invalid_mermaid_with_failure_block() {
        let options = comrak_options();
        let rewritten = rewrite_markdown(
            "```mermaid\nnot a diagram\n```",
            MdMermaidRender::Always,
            &options,
        );
        assert!(rewritten.markdown.contains("Mermaid render failed"));
        assert!(rewritten.strict_failure.is_some());
    }
}
