| File Type                                                                   | Markdown Viewer | Image/Video | Interactive Viewer | LS  | Converting          |
| --------------------------------------------------------------------------- | --------------- | ----------- | ------------------ | --- | ------------------- |
| mp4, m4v, mkv, webm, mov, avi, wmv, mpg, flv                                | 🟥              | 🟩          | 🟥                 | 🟨  |                     |
| gif                                                                         | 🟨              | 🟩          | 🟨                 | 🟩  |                     |
| tex, typ                                                                    | 🟩              | 🟩          | 🟩                 | 🟨  | png                 |
| pdf                                                                         | 🟩              | 🟩          | 🟩                 | 🟩  | png                 |
| md                                                                          | 🟩              | 🟩          | 🟩                 | 🟨  | html, png           |
| html                                                                        | 🟨              | 🟩          | 🟩                 | 🟨  | png                 |
| svg, bmp, dds, exr, ff, hdr, ico, jpeg, png, pnm, qoi, tga, tiff, webp, jxl | 🟩              | 🟩          | 🟩                 | 🟩  | png                 |
| url, exe, lnk                                                               | 🟩              | 🟩          | 🟩                 | 🟩  | png                 |
| csv, docx, xls, xlsx, xlsm, xlsb, xla, xlam, odt, odp, pptx, ods, zip, tar  | 🟩              | 🟩          | 🟩                 | 🟨  | markdown, html, png |
| other text files                                                            | 🟩              | 🟩          | 🟩                 | 🟨  | markdown, html, png |

> [!NOTE]
>
> - files Compressed with `.xz` or `.gz` will also work
> - `pdf`, `typst`, and `tex` when passed alone into the interactive viewer, will show all their pages
## Mermaid rendering support for Markdown

Enable with `--md-mermaid <auto|always|never>` or `MCAT_MD_MERMAID=<auto|always|never>`.

- Default (`never`): Mermaid fences render as normal code blocks.
- ANSI markdown viewer: Mermaid fences render as ANSI/Unicode diagrams.
- HTML output: Mermaid fences are rewritten to inline `<svg`.
- Image-capable outputs (`image`, `inline`, `interactive`): use the HTML Mermaid rewrite path before image capture.
