# Mermaid Inline Image Design

## Goal

Render Mermaid code fences as terminal inline images on image-capable markdown
paths without depending on the HTML-to-image browser backend, while preserving
the existing ANSI fallback and HTML inline-SVG behavior.

## Context

`mcat` currently supports Mermaid in two independent ways:

- ANSI markdown output renders `mermaid` fences through `console-mermaid`.
- HTML markdown output rewrites `mermaid` fences into inline SVG via
  `mermaid-rs-renderer`.

The ANSI markdown renderer currently treats Mermaid as text output only. In that
path, image-capable terminals still receive ANSI / Unicode Mermaid output rather
than a real inline image, even though `mermaid-rs-renderer` and `mcat`'s
existing SVG rasterization path can render Mermaid diagrams directly in Rust.

Separately, full-document markdown screenshot outputs such as `--output image`,
`--output inline`, and `--output interactive` already render the entire markdown
document to HTML and then rasterize that HTML through the Chromium-backed
`html_to_image` pipeline. That browser-backed path remains appropriate for
whole-page screenshots and is not the target of this design.

## Approaches Considered

### 1. Keep the existing browser-backed markdown image path

This requires no structural changes, but it preserves the current Chromium
dependency for Mermaid in image-capable markdown output. It also means Mermaid
support in those modes remains coupled to full-page HTML screenshot behavior
instead of using the native Mermaid renderers already in the repo.

### 2. Replace the full markdown image pipeline with a pure Rust renderer

This would remove Chromium more broadly, but it is out of scope. `mcat` does
not have a full markdown layout engine for image outputs. Replacing the browser
path for whole-document rendering would be a separate project with much larger
surface area than Mermaid support alone.

### 3. Add a Mermaid-only terminal image path inside the ANSI markdown renderer

This keeps the existing full-document browser path for `md -> image`,
`md -> inline`, and `md -> interactive` unchanged, but allows Mermaid code
fences to be rendered as inline terminal images directly inside the ANSI
markdown renderer when the terminal supports inline images. Non-image terminals
continue to use the existing ANSI/Unicode Mermaid renderer. This is the
recommended approach because it isolates the change to Mermaid blocks, reuses
existing image-encoding infrastructure, and removes the browser dependency from
the Mermaid-specific ANSI markdown path without redesigning the rest of markdown
rendering.

## Recommended Design

### Rendering behavior

For `mermaid` code fences in the ANSI markdown renderer:

- On image-capable terminals, render Mermaid to SVG with `mermaid-rs-renderer`,
  rasterize the SVG with `converter::svg_to_image`, and encode the raster image
  with the same terminal image machinery already used by markdown image nodes.
- On non-image-capable markdown paths, keep using `console-mermaid` for ANSI /
  Unicode output.
- On HTML output, keep rewriting Mermaid fences into inline SVG as today.

The existing `--md-mermaid auto|always|never` contract remains unchanged:

- `never`: preserve normal code-block rendering.
- `auto`: attempt Mermaid rendering, fall back to normal code rendering on
  failure.
- `always`: render explicit failure output and return a non-zero error if
  Mermaid rendering fails.

### Scope

This design only changes Mermaid block rendering inside the ANSI markdown
viewer. It does not replace the browser-backed full-document image pipeline
used for `--output image`, `--output inline`, or `--output interactive` when
those modes render an entire markdown document as a single rasterized page.

### Architecture

Introduce a small Mermaid image helper alongside the existing Mermaid adapters.
That helper should:

- Render Mermaid source to SVG via the existing `render_svg_mermaid` path.
- Convert the SVG into `DynamicImage` via `converter::svg_to_image`.
- Encode that image into the terminal's inline image protocol using the same
  `rasteroid::inline_an_image` path currently used by `ImagePreprocessor`.
- Return both the inline image payload and a placeholder marker so block layout
  remains compatible with the existing text post-processing strategy.

The ANSI markdown renderer should then treat Mermaid blocks similarly to
markdown image nodes when the configured inline encoder is image-capable.

### File-level changes

- `crates/core/src/markdown_viewer/mermaid.rs`
  - Add a Mermaid-to-`DynamicImage` helper that builds on the existing SVG path.
- `crates/core/src/markdown_viewer/render.rs`
  - Update Mermaid code-block rendering so image-capable terminals can emit
    inline images for Mermaid fences instead of ANSI box art.
  - Preserve existing fallback and strict-failure behavior.
- `crates/core/src/markdown_viewer/image_preprocessor.rs`
  - Extract any reusable inline-image encoding helpers needed by Mermaid blocks,
    or move shared logic into a small utility module if that keeps boundaries
    cleaner.
- `crates/core/src/markdown_viewer/mod.rs`
  - Ensure any post-processing needed for Mermaid inline placeholders runs in
    the same phase as markdown image replacement.
- Tests in `crates/core/src/markdown_viewer` and `crates/core/src/catter.rs`
  - Add regression coverage for image-capable Mermaid rendering while preserving
    existing ANSI/plain and HTML behavior.

### Placeholder strategy

The safest implementation is to reuse the existing placeholder-and-replacement
model already used for markdown images:

- During Markdown AST rendering, Mermaid blocks emit a deterministic placeholder
  token into the text stream.
- After text rendering completes, Mermaid inline images replace those
  placeholders with the encoded terminal image payload.

That placeholder contract must reuse the current markdown-image placeholder
shaping rules rather than inventing a simpler token. The replacement mechanism
must preserve:

- uniqueness per rendered Mermaid block
- placeholder width and height compatibility with the encoded payload
- multiline Kitty payload behavior
- compatibility with the existing placeholder replacement pass in the markdown
  post-processing stage

This avoids mixing multiline terminal image payload generation directly into the
node renderer's layout logic and keeps Mermaid image handling consistent with
current markdown image processing.

### Interaction with `--md-image` and `--fast`

Mermaid inline images should follow the same gating policy as markdown inline
images in the ANSI markdown renderer:

- If `--md-image` disables inline markdown images, Mermaid should not emit
  terminal inline images either and should fall back to the normal ANSI Mermaid
  path.
- If `--fast` disables markdown images, Mermaid should also fall back to the
  normal ANSI Mermaid path.
- No new Mermaid-specific CLI flag is introduced for this behavior in this
  iteration.

This keeps terminal-image policy centralized and avoids surprising users with
Mermaid blocks bypassing markdown image controls.

### Error handling

- `auto` mode:
  - If Mermaid SVG/image rendering fails, fall back to the existing code-block
    path.
- `always` mode:
  - If Mermaid SVG/image rendering fails, record the strict failure and emit the
    existing explicit Mermaid failure output.
- If inline image encoding fails in an image-capable terminal, treat that as a
  Mermaid image rendering failure and apply the same `auto`/`always` fallback
  rules.

### Testing

Add regression coverage for:

- Mermaid code fences render as terminal inline images when the inline encoder
  is image-capable.
- Mermaid code fences in blockquotes and list items still render as valid
  terminal inline images when the inline encoder is image-capable.
- Mermaid code fences still render as ANSI/Unicode diagrams on ASCII-only
  terminals.
- `auto` mode falls back to normal code rendering when Mermaid image generation
  fails.
- `always` mode still produces explicit failure output and non-zero exit status
  when Mermaid image generation fails.
- Nested Mermaid fences in blockquotes and list items continue to preserve
  surrounding layout behavior on the ANSI/plain fallback paths.
- HTML output remains inline SVG and does not regress.
- `md -> image`, `md -> inline`, and `md -> interactive` retain the existing
  browser-backed full-document screenshot path and are not silently rerouted by
  this feature.

### Risks

- The current markdown image replacement logic was built around `NodeValue::Image`
  URLs, not code fences. The implementation should avoid forcing Mermaid blocks
  through URL-shaped abstractions if that creates unnecessary coupling.
- Terminal image payloads are multiline and layout-sensitive. Placeholder
  replacement must remain compatible with blockquote/list/code-block formatting.
- `mcat` currently carries two `resvg/usvg` versions via its own renderer and
  `mermaid-rs-renderer`; this feature should reuse existing helpers where
  possible instead of adding a third rasterization path.

## Success Criteria

- Mermaid fences display as real inline images in image-capable ANSI markdown
  output without requiring Chromium.
- Existing ANSI/Unicode Mermaid rendering continues to work on non-image
  terminals.
- Existing HTML inline-SVG Mermaid output continues to work unchanged.
- Browser-backed markdown screenshot modes remain functional and are not
  expanded in scope by this change.
