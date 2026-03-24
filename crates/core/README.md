# Core crate

This crate contains the `mcat` CLI entrypoint and rendering/conversion pipeline.
`main.rs` wires CLI/config parsing and dispatches to modules such as:

- `catter` for output pipeline routing
- `markdown_viewer` for ANSI/HTML markdown rendering
- `converter` for html/image/media conversions

## Mermaid rendering

Markdown Mermaid rendering is controlled by:

- `--md-mermaid <auto|always|never>`
- `MCAT_MD_MERMAID=<auto|always|never>`

Behavior:

- `never` (default): Mermaid fenced blocks stay normal code fences.
- `auto`: render Mermaid when supported, otherwise keep normal code fences.
- `always`: render Mermaid strictly, emit an explicit failure block, and return a non-zero exit.

Output-specific behavior:

- ANSI markdown viewer: Mermaid fences render as ANSI/Unicode diagrams.
- HTML output: Mermaid fences are rewritten to inline `<svg`.
- Image-capable outputs (`image`, `inline`, `interactive`): flow through the HTML path, so Mermaid is rendered as inline SVG before image capture.
