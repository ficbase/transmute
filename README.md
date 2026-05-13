# transmute

Multi-format document conversion toolkit.  
Currently: **txt ↔ epub** (EPUB 3.3).

## Install

```bash
cargo install --git https://github.com/ficbase/transmute
```

Or build from source:

```bash
git clone git@github.com:ficbase/transmute.git
cd transmute
cargo build --release
```

## Usage

Format is auto-detected by file extension.

```bash
# txt → epub (with auto-generated SVG cover)
transmute novel.txt novel.epub

# txt → epub (with custom cover image)
transmute novel.txt novel.epub -c cover.jpg

# epub → txt
transmute novel.epub novel.txt
```

## Features

- **EPUB 3.3** output — nav.xhtml navigation, XHTML5 content pages
- **Auto-generated SVG cover** — dark theme with title & author
- **Smart chapter detection** — Arabic (`第1章`), Chinese (`第一章`, `第八十四章`), parts/volumes (`第一部`, `第X卷`), Markdown (`# Title`)
- **Metadata extraction** — auto-detects title (`《书名》`) and author (`作者：XXX`)
- **Preserves indentation** — full-width spaces kept intact

## As a library

```rust
use transmute::{Book, Chapter, Metadata, write_epub_file, parse_epub_file};

// txt → epub
let book = Book {
    metadata: Metadata {
        title: "My Novel".into(),
        author: "Author".into(),
        language: "zh".into(),
        ..Default::default()
    },
    chapters: vec![Chapter {
        title: "Chapter 1".into(),
        body: "Once upon a time...".into(),
    }],
    cover: None, // auto-generates SVG
};
write_epub_file(&book, "output.epub")?;

// epub → txt
let book = parse_epub_file("input.epub")?;
println!("{} by {}", book.metadata.title, book.metadata.author);
```

## License

MIT
