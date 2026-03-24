use comrak::{
    Arena,
    nodes::{AstNode, NodeValue},
};

use crate::config::MdMermaidRender;

use super::mermaid;

struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

pub(crate) fn rewrite_markdown(markdown: &str, mode: MdMermaidRender) -> String {
    if !mode.should_try_rendering() {
        return markdown.to_owned();
    }

    let arena = Arena::new();
    let options = super::comrak_options();
    let root = comrak::parse_document(&arena, markdown, &options);
    let line_starts = line_start_offsets(markdown);

    let mut replacements = Vec::new();
    collect_replacements(root, markdown, &line_starts, &mut replacements);

    if replacements.is_empty() {
        return markdown.to_owned();
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

    rewritten
}

fn collect_replacements<'a>(
    node: &'a AstNode<'a>,
    markdown: &str,
    line_starts: &[usize],
    replacements: &mut Vec<Replacement>,
) {
    if let Some(replacement) = replacement_for_node(node, markdown, line_starts) {
        replacements.push(replacement);
    }

    for child in node.children() {
        collect_replacements(child, markdown, line_starts, replacements);
    }
}

fn replacement_for_node<'a>(
    node: &'a AstNode<'a>,
    markdown: &str,
    line_starts: &[usize],
) -> Option<Replacement> {
    let data = node.data.borrow();
    let NodeValue::CodeBlock(code_block) = &data.value else {
        return None;
    };
    if !mermaid::is_mermaid_info(&code_block.info) {
        return None;
    }

    let (start, end) = sourcepos_to_range(markdown, line_starts, data.sourcepos)?;
    let actual_start = fence_start_byte(markdown, line_starts, data.sourcepos.start.line, start)?;
    let prefix = line_prefix(markdown, line_starts, data.sourcepos.start.line, actual_start)?;
    let rendered = mermaid::render_ansi_mermaid(&code_block.literal).ok()?;

    Some(Replacement {
        start: actual_start,
        end,
        text: prefix_following_lines(
            &strip_ansi_escapes::strip_str(&rendered).to_string(),
            prefix,
        ),
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
    if start > end {
        return None;
    }

    let line_slice = markdown.get(start..end)?;
    let mut byte_offset = 0usize;
    let mut current_column = 1usize;
    for ch in line_slice.chars() {
        if current_column == column {
            return Some(start + byte_offset);
        }
        byte_offset += ch.len_utf8();
        current_column += 1;
    }

    if current_column == column {
        Some(end)
    } else {
        None
    }
}

fn fence_start_byte(
    markdown: &str,
    line_starts: &[usize],
    line: usize,
    fallback_start: usize,
) -> Option<usize> {
    let line_start = *line_starts.get(line.saturating_sub(1))?;
    let line_end = match line_starts.get(line) {
        Some(next_line_start) => next_line_start.saturating_sub(1),
        None => markdown.len(),
    };
    let line_slice = markdown.get(line_start..line_end)?;
    let fence_offset = ["```", "~~~"]
        .into_iter()
        .filter_map(|fence| line_slice.find(fence))
        .min()
        .unwrap_or(fallback_start.saturating_sub(line_start));

    Some(line_start + fence_offset)
}

fn line_prefix<'a>(
    markdown: &'a str,
    line_starts: &[usize],
    line: usize,
    start: usize,
) -> Option<&'a str> {
    let line_start = *line_starts.get(line.saturating_sub(1))?;
    markdown.get(line_start..start)
}

fn prefix_following_lines(rendered: &str, prefix: &str) -> String {
    let mut lines = rendered.lines();
    let Some(first_line) = lines.next() else {
        return String::new();
    };

    let mut output = String::from(first_line);
    for line in lines {
        output.push('\n');
        output.push_str(prefix);
        output.push_str(line);
    }

    output
}
