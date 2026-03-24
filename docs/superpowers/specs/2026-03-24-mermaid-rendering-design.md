# Mermaid Rendering Design

Date: 2026-03-24

## Goal

Add first-class Mermaid support to `mcat` without changing the core user
model for markdown rendering.

The feature should:

- Render Mermaid fenced code blocks in the default ANSI markdown viewer
- Render Mermaid fenced code blocks in HTML output
- Reuse the existing `md -> html -> image` pipeline for `image`, `inline`,
  and `interactive` outputs
- Preserve current behavior for users who do not opt in

## Scope

Version 1 only supports fenced code blocks whose info string is exactly
`mermaid`.

Out of scope for v1:

- Mermaid embedded in raw HTML
- User-facing backend selection
- Sidecar file generation for rendered diagrams
- Mermaid support when `mcat` is emitting raw markdown instead of rendering
  markdown

## User-Facing CLI

Add a markdown-specific flag:

`--md-mermaid <auto|always|never>`

Semantics:

- `never`: preserve current behavior
- `auto`: attempt Mermaid rendering where supported; fall back to normal code
  block rendering on Mermaid render errors
- `always`: require Mermaid rendering; surface a render failure explicitly and
  exit non-zero

This follows the existing `mcat` model where markdown rendering policy is
configured through markdown-specific flags such as `--md-image`.

Environment variable:

`MCAT_MD_MERMAID=<auto|always|never>`

Default:

`never`

Rationale:

- Avoid surprising users by changing the rendering of existing markdown files
- Keep backend choice as an implementation detail
- Preserve a simple CLI surface

## Architecture

Mermaid support should be implemented as a markdown-viewer subfeature with two
backend adapters.

### Graphic backend

Trait-like responsibility:

- Input: Mermaid source
- Output: SVG

Primary implementation:

- Embedded `mmdr` / `mermaid-rs-renderer`

Behavior:

- `html`: replace Mermaid fenced blocks with inline SVG-bearing HTML
- `image`, `inline`, `interactive`: reuse the transformed HTML and continue
  through the existing HTML-to-image pipeline

### ANSI backend

Trait-like responsibility:

- Input: Mermaid source
- Output: ANSI/Unicode text rendering

Primary implementation:

- Embedded `console-mermaid`

Behavior:

- Intercept Mermaid fenced blocks before normal code-block rendering
- Render supported diagrams as ANSI/Unicode diagrams
- On failure, follow `auto|always|never` policy

## Pipeline Integration

### ANSI rendering path

Current path:

- preprocess HTML fragments
- parse markdown with Comrak
- walk the AST and render nodes manually

Change:

- special-case fenced code blocks with info string `mermaid`
- delegate those blocks to the Mermaid ANSI backend
- continue using existing code fence rendering for all other languages

### HTML rendering path

Current path:

- convert markdown to HTML using Comrak
- optionally apply syntax highlighting

Change:

- transform Mermaid fences into inline SVG before final HTML output is emitted
- avoid leaving Mermaid as a plain `<pre><code class="language-mermaid">`
  block when rendering succeeds

The existing `html -> image` pipeline remains unchanged.

## Fallback And Errors

### `never`

- Do not attempt Mermaid rendering
- Render Mermaid fences exactly as ordinary code fences today

### `auto`

- ANSI mode: try the ANSI backend, then fall back to ordinary code rendering
- HTML/image-capable outputs: try the graphic backend, then fall back to
  ordinary code rendering

Fallback should be silent in normal operation.

### `always`

- If Mermaid rendering fails, emit an explicit render failure block
- Return a non-zero exit code
- Do not silently fall back to code rendering

## Why Embedded Backends

Use embedded Rust backends instead of requiring external CLIs.

Reasons:

- Better UX: one binary, no extra installation burden
- Better portability and fewer runtime failure modes
- Lower overhead than spawning a process for each Mermaid block
- Cleaner control over fallback and error behavior
- Better fit for `mcat`, which already owns markdown rendering behavior

External CLI execution should not be part of the normal path.

## Testing Strategy

### Unit tests

- Parse and store `--md-mermaid`
- Detect Mermaid fenced code blocks by info string
- Select the correct fallback policy for `auto|always|never`

### Adapter tests

- Graphic backend: supported Mermaid source produces non-empty SVG containing
  `<svg`
- ANSI backend: supported Mermaid source produces non-empty ANSI/Unicode text

### Pipeline tests

- ANSI render path: successful Mermaid rendering does not emit the ordinary
  `mermaid` code-block header
- HTML render path: successful Mermaid rendering emits inline SVG instead of
  `language-mermaid` code output
- Existing non-Mermaid code fences still render unchanged

### Failure tests

- Invalid Mermaid + `auto` falls back to ordinary code rendering
- Invalid Mermaid + `always` fails explicitly

## Implementation Notes

- Keep Mermaid backend selection internal for v1
- Keep v1 support limited to fenced code blocks with info string exactly
  `mermaid`
- Integrate configuration into `McatConfig` near existing markdown render
  options
- Prefer SVG as the canonical intermediate for graphic outputs

## Future Extensions

- Optional advanced backend-selection flag if there is a demonstrated need
- Mermaid theme mapping from `mcat` themes
- Support for Mermaid in raw HTML fragments
- Better diagnostics or debug logging for Mermaid render failures
