# presswork

An HTML to PDF converter for documents such as invoices, reports and
certificates, written in Rust with no browser dependency. It aims to be a
replacement for [wkhtmltopdf](https://wkhtmltopdf.org/), which is no longer
maintained, and accepts the same command line.

> **Status: early development.** presswork renders text documents styled
> and images with a useful subset of CSS. Floats, positioning, flexbox and
> grid are not supported yet.

## What works today

- **HTML to PDF, end to end.** HTML5 parsing; paragraphs, headings, lists
  with hanging markers, preformatted text and tables.
- **CSS.** `<style>` elements, `<link rel=stylesheet>`, `@import`,
  `style=""` attributes and a built-in default stylesheet, with the full
  cascade (specificity, source order,
  `!important`, inheritance) and Selectors Level 4 matching (`:nth-child`,
  `:not`, `:is`, attribute selectors, combinators, `::before` and
  `::after`). `@media print` rules apply; `@media screen` rules do not.
  Supported properties:
  - text: `color`, `font-family`, `font-size`, `font-weight`, `font-style`,
    `font`, `line-height`, `text-align`, `white-space`, `text-indent`,
    `text-transform`, `letter-spacing`, `word-spacing`, `vertical-align`
    (`<sup>`, `<sub>` and the keyword values)
  - text decoration: `text-decoration` (underline, overline and
    line-through, drawn solid), `text-decoration-line`,
    `text-decoration-color`
  - boxes: `display`, `margin`, `padding`, `border` (solid, dashed, dotted
    and double; other styles are drawn solid), `border-radius`, `width`,
    `max-width`, `height`, `min-height`, `max-height`, `box-sizing`,
    `background-color`
  - inline elements: `background-color`, `border`, `padding` and
    horizontal `margin` on `<span>`, `<a>`, `<strong>` and the like, drawn
    on each line the element covers; `display: inline-block` boxes, sized
    to their content or their `width` and aligned by their last line
  - images: `width`, `height`, `max-width`, `max-height`, `vertical-align`
  - tables: `border-collapse`, `border-spacing`, `table-layout`,
    `vertical-align`, `height` on cells and rows
  - lists: `list-style-type`, including `devanagari`
  - generated content: `content` (strings, `counter()`, `counters()`,
    `attr()`, quotes) on `::before` and `::after`, `counter-reset`,
    `counter-increment`, `counter-set`
  - pages: `break-before`, `break-after`, `break-inside` (`page` and
    `avoid`) and the `page-break-*` aliases, `orphans`, `widows`
  - fonts: `@font-face` with TrueType, OpenType and WOFF files, loaded only
    when text uses them
- **Legacy HTML attributes** that wkhtmltopdf-era templates rely on:
  `align`, `valign`, `width`, `height`, `bgcolor`, `border`, `cellpadding`,
  `cellspacing`, `nowrap` and `<font>`.
- **Images.** PNG, JPEG, GIF and WebP in `<img>`, inline or as blocks,
  sized by their attributes and CSS and scaled to fit with `max-width`. An
  image that fails to load is reported and replaced by its `alt` text.
- **Linked files.** Images, stylesheets and fonts given as `data:` URLs
  always load. Local files load only with `--enable-local-file-access` or
  from a directory given with `--allow`, as in wkhtmltopdf 0.12.6.
  Relative URLs resolve against the input file (or the current directory
  for standard input), and inside a stylesheet against the stylesheet.
- **Tables.** Columns are sized to their content and to the widths given
  to cells and `<col>`s (columns with a width keep it; the others share
  the rest), with `colspan` and `rowspan`, cell borders and backgrounds,
  and collapsed borders, where the borders of cells, rows, row groups,
  columns and the table meet and the strongest wins, as in browsers. Rows are never split across pages, nor are rows
  joined by a `rowspan` cell; `<thead>` rows repeat at the top of every
  page, and a header is never left alone at the bottom of a page.
- **Pagination.** Pages break between lines and table rows; backgrounds and
  borders continue across the break. `@page { size; margin }` is honored.
  A box with `break-inside: avoid` moves to the next page whole when it
  fits there, headings stay with the content that follows them, and
  paragraphs leave at least two lines on each page.
- **Text in any script the system has fonts for.** Text is shaped with
  HarfBuzz-compatible shaping, so Devanagari conjuncts and vowel signs render
  correctly, and fonts fall back by script. Symbols (✔, ★, →) and emoji
  that the text's fonts lack come from installed symbol and emoji fonts,
  such as Noto Sans Symbols 2 and Noto Color Emoji. Fonts are embedded and
  subsetted, and the PDF text can be searched and copied.
- **Document metadata.** The `<title>` becomes the PDF title unless `--title`
  is given.
- **Bookmarks and links.** Headings (`<h1>` to `<h6>`) become the PDF
  outline, nested by level. `<a href>` links are clickable: links to other
  documents open them, and links to an `id` or `<a name>` in the document
  jump to it.
- **wkhtmltopdf's command line.** Every documented wkhtmltopdf 0.12.6 option
  is recognised, so invocations from wrapper libraries such as `pdfkit`,
  `wicked_pdf` and `snappy` parse:
  - `-` reads HTML from standard input or writes the PDF to standard output.
  - `file://` input URLs are accepted.
  - `-V` reports `wkhtmltopdf 0.12.6 (presswork <version>)`, so wrappers that
    check the version (such as `wicked_pdf`) use the current argument style.
  - Options that are not implemented yet are ignored with a warning.
    Options unknown to wkhtmltopdf are rejected, as wkhtmltopdf does.
  - `-q` / `--quiet` and `--log-level none` silence warnings, except for
    ignored options that change the output, such as HTML headers and
    zoom. Those are always reported, because `pdfkit` passes `--quiet` by
    default.
  - `--strict`, or `PRESSWORK_STRICT=1` in the environment, makes those
    options an error instead of a warning. Use the environment variable when
    a wrapper library builds the command line for you.
- **Page options.** `-s` / `--page-size` accepts every size wkhtmltopdf does
  (A0–A9, B0–B10, C5E, Comm10E, DLE, Executive, Folio, Ledger, Legal, Letter,
  Tabloid). `-O` / `--orientation`, `--page-width`, `--page-height`,
  `-T` / `-R` / `-B` / `-L` (`--margin-*`) and `--title` are supported.
  Options given on the command line win over the document's `@page` rule.
- **Resource options.** `--enable-local-file-access`,
  `--disable-local-file-access`, `--allow`, `--images`, `--no-images`,
  `--load-media-error-handling` (`abort` stops without writing the PDF) and
  `--user-style-sheet`, which the document's own styles override unless it
  uses `!important`. The user stylesheet is read whatever the file access
  options say, since whoever runs presswork named it.
- **Text headers and footers.** `--header-left`, `--header-center`,
  `--header-right`, the `--footer-*` equivalents, `--header-font-name`,
  `--header-font-size`, `--header-line`, `--header-spacing` (and their
  footer versions), `--default-header`, `--page-offset` and `--replace`.
  Placement matches wkhtmltopdf: the text sits in the page margin just
  outside the content, so give it room with `--margin-top` or
  `--margin-bottom`. These variables are replaced: `[page]`, `[topage]`,
  `[frompage]`, `[sitepage]`, `[sitepages]`, `[title]`, `[doctitle]`,
  `[webpage]`, `[date]`, `[isodate]`, `[time]`, and `[section]`,
  `[subsection]` and `[subsubsection]` (the headings in effect on the
  page), plus any set with `--replace`. The text can be in any script, for example
  `--footer-center "पृष्ठ [page] / [topage]"`.
- **Outline and link options.** `--outline`, `--no-outline`,
  `--outline-depth` (default 4), `--enable-external-links`,
  `--disable-external-links`, `--enable-internal-links` and
  `--disable-internal-links`.
- **HTML headers and footers.** `--header-html` and `--footer-html` take a
  path or `file:` URL and draw that document in the top or bottom margin
  of every page. Without `--margin-top` (or `--margin-bottom`), the margin
  grows to fit it; with one, the margin grows by the spacing, as in
  wkhtmltopdf. Images and stylesheets it links to follow the file access
  options. Page numbers work through the usual wkhtmltopdf script without
  running it; see below.

Not yet: SVG and CSS background images, WOFF2 fonts and `unicode-range`,
floats and positioning, flexbox and grid, and the table of contents.

## Usage

```sh
presswork [OPTIONS] <INPUT> <OUTPUT>
presswork --page-size Letter --margin-top 20mm invoice.html invoice.pdf
cat invoice.html | presswork --quiet - - > invoice.pdf
```

## Differences from wkhtmltopdf

- **No JavaScript.** `--javascript-delay`, `--window-status` and
  `--run-script` have no effect. Render dynamic content on the server first.
  HTML headers and footers usually carry the script from wkhtmltopdf's
  documentation, which fills elements with class `page`, `topage`,
  `title` and so on from the URL. When a header or footer has a script,
  presswork fills those elements itself, with every variable a text
  header can use, so such templates work unchanged. Other scripts do not
  run; presswork warns when a header's script has nothing it can fill.
- **HTML header placement.** The header's `<body>` box ends right above
  the content (less `--header-spacing`), and a footer's starts right below
  it, so a body margin stays outside the page margin instead of pushing the
  header into the content or the footer off the page.
- **No network access.** Documents and images are never fetched from
  `http(s)://` URLs. An image that would need the network is reported even
  under `--quiet`, since wkhtmltopdf would have loaded it, and `--strict`
  makes it an error. Embed such images as `data:` URLs or local files.
- **Standards-based layout.** Output follows the CSS specifications, not
  QtWebKit, so templates tuned for wkhtmltopdf's rendering (for example its
  smart shrinking) may lay out differently.
- **Boxes grow to fit.** Content taller than a box's `height` makes the box
  taller instead of overflowing it, so text never runs into what follows.
  `overflow` is not supported.
- **Page breaks.** wkhtmltopdf ignores `orphans` and `widows` and can cut
  a line of text in half at a page break. presswork never splits a line,
  keeps at least two lines of a paragraph on each page (the CSS default),
  and keeps headings with what follows them, as print formatters do, so
  pages may break a little earlier than in wkhtmltopdf.
- **Print media.** Stylesheets are evaluated for `print` media, so
  `@media screen` rules do not apply. wkhtmltopdf uses `screen` unless
  `--print-media-type` is given.
- **System fonts.** Fonts come from the system's font configuration, so
  output can differ between machines with different fonts installed. Use
  `@font-face` to make a document look the same everywhere. On
  Linux, fontconfig is loaded at runtime: building needs no fontconfig
  development package, but rendering text needs fontconfig and fonts
  installed (presswork warns if it finds none).
- **One input document.** Multiple inputs, cover pages and `toc` objects are
  not supported yet.
- **Header dates.** `[date]` and `[time]` always use wkhtmltopdf's format
  for the C locale (`17 Nov 2026`, `20:06:55`), the usual one on servers.
  wkhtmltopdf follows `LANG`, so it may write `11/17/26` instead.

## Building

Requires Rust 1.92 or newer.

```sh
cargo build --release
cargo test --workspace
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
