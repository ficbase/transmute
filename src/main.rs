use std::path::PathBuf;

use transmute::{Book, Chapter, CoverImage, Metadata, parse_epub_file, write_epub_file};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: transmute <input> <output> [-c <cover.jpg>]");
        eprintln!("  .txt → .epub  or  .epub → .txt  (auto-detect)");
        std::process::exit(1);
    }

    let mut input = None;
    let mut output = None;
    let mut cover_path: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--cover" => {
                i += 1;
                if i < args.len() {
                    cover_path = Some(PathBuf::from(&args[i]));
                } else {
                    eprintln!("Missing cover image path after -c");
                    std::process::exit(1);
                }
            }
            arg if !arg.starts_with('-') => {
                if input.is_none() {
                    input = Some(PathBuf::from(arg));
                } else {
                    output = Some(PathBuf::from(arg));
                }
            }
            _ => {}
        }
        i += 1;
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("Missing input file");
        std::process::exit(1);
    });
    let output = output.unwrap_or_else(|| {
        eprintln!("Missing output file");
        std::process::exit(1);
    });

    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "txt" => txt_to_epub(&input, &output, cover_path),
        "epub" => epub_to_txt(&input, &output),
        other => {
            eprintln!("Unknown input format: .{other}");
            std::process::exit(1);
        }
    }
}

/// Read a text file with automatic encoding detection.
/// Tries BOM → UTF-8 validation → GBK fallback.
fn auto_decode(path: &std::path::Path) -> Result<String, String> {
    use encoding_rs::Encoding;
    let raw = std::fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // 1. Check BOM
    if raw.len() >= 3 && raw[0] == 0xEF && raw[1] == 0xBB && raw[2] == 0xBF {
        return Ok(Encoding::for_label(b"utf-8").unwrap()
            .decode_without_bom_handling(&raw[3..]).0.into_owned());
    }
    if raw.len() >= 2 && raw[0] == 0xFF && raw[1] == 0xFE {
        return Ok(Encoding::for_label(b"utf-16le").unwrap()
            .decode_without_bom_handling(&raw[2..]).0.into_owned());
    }
    if raw.len() >= 2 && raw[0] == 0xFE && raw[1] == 0xFF {
        return Ok(Encoding::for_label(b"utf-16be").unwrap()
            .decode_without_bom_handling(&raw[2..]).0.into_owned());
    }

    // 2. Try UTF-8 — validate by decoding
    let utf8 = Encoding::for_label(b"utf-8").unwrap();
    let (cow, _, had_errors) = utf8.decode(&raw);
    if !had_errors {
        return Ok(cow.into_owned());
    }

    // 3. Fall back to GBK (most common for Chinese txt files)
    let gbk = Encoding::for_label(b"gbk").unwrap();
    let (cow, _, _) = gbk.decode(&raw);
    Ok(cow.into_owned())
}

fn txt_to_epub(input: &PathBuf, output: &PathBuf, cover_path: Option<PathBuf>) {

    let text = match auto_decode(input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let cover = cover_path
        .and_then(|p| {
            let ext = p.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg")
                .to_lowercase();
            let media_type = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                other => {
                    eprintln!("Warning: unknown image extension .{other}, using image/jpeg");
                    "image/jpeg"
                }
            };
            let file_name = p.file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| "cover.jpg".into());
            std::fs::read(&p).ok().map(|data| CoverImage {
                data,
                media_type: media_type.into(),
                file_name,
            })
        });

    let author = extract_author(&text);
    let title = extract_title(&text).unwrap_or_else(|| {
        input
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".into())
    });

    let chapters = split_into_chapters(&text);

    let book = Book {
        metadata: Metadata {
            title,
            author,
            language: "zh".into(),
            ..Default::default()
        },
        chapters,
        cover,
    };

    match write_epub_file(&book, &output) {
        Ok(()) => println!("✅ EPUB written to {}", output.display()),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn split_into_chapters(text: &str) -> Vec<Chapter> {
    let mut chapters = Vec::new();
    let mut current_title = String::from("Chapter 1");
    let mut current_body = String::new();
    let mut first = true;

    for line in text.lines() {
        let trimmed = line.trim();
        if is_chapter_heading(trimmed) {
            if !first {
                chapters.push(Chapter {
                    title: current_title.clone(),
                    body: std::mem::take(&mut current_body),
                });
            }
            current_title = trimmed.to_string();
            first = false;
        } else {
            if !current_body.is_empty() {
                current_body.push('\n');
            }
            current_body.push_str(line);
        }
    }

    if !first || !current_body.is_empty() {
        if current_title.is_empty() {
            current_title = "Chapter 1".into();
        }
        chapters.push(Chapter {
            title: current_title,
            body: current_body,
        });
    }

    chapters
}

fn is_chapter_heading(line: &str) -> bool {
    if line.starts_with("# ") {
        return true;
    }
    if line.starts_with("Chapter ") || line.starts_with("CHAPTER ") {
        return true;
    }
    // "第..." patterns: 第一章, 第1章, 第一部, 第X卷, etc.
    let mut chars = line.chars();
    if chars.next() != Some('第') {
        return false;
    }
    let mut skipped = false;
    for c in chars.by_ref() {
        if c.is_ascii_digit() {
            skipped = true;
        } else if is_cn_digit_char(c) {
            skipped = true;
        } else if skipped
            && (c == '章' || c == '节' || c == '部' || c == '卷' || c == '篇' || c == '集')
        {
            // reject false positives like 第X部分
            let next = chars.next();
            return next != Some('分');
        } else {
            return false;
        }
    }
    false
}

fn is_cn_digit_char(c: char) -> bool {
    matches!(
        c,
        '零' | '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九'
            | '十' | '百' | '千' | '万' | '两' | '〇'
    )
}

/// Extract author from text — looks for "作者：XXX" anywhere in first 20 lines.
fn extract_author(text: &str) -> String {
    for line in text.lines().take(20) {
        for separator in ["作者：", "作者:"] {
            if let Some(pos) = line.find(separator) {
                let author = line[pos + separator.len()..].trim();
                if !author.is_empty() {
                    return author.to_string();
                }
            }
        }
    }
    String::new()
}

/// Extract title from text metadata lines like "《高天之上》".
fn extract_title(text: &str) -> Option<String> {
    for line in text.lines().take(20) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('《') {
            if let Some(title) = rest.split('》').next() {
                if !title.is_empty() {
                    return Some(title.to_string());
                }
            }
        }
    }
    None
}

fn epub_to_txt(input: &PathBuf, output: &PathBuf) {
    match parse_epub_file(input) {
        Ok(book) => {
            let mut txt = String::new();
            // metadata header
            if !book.metadata.title.is_empty() {
                txt.push_str(&format!("《{}》\n", book.metadata.title));
            }
            if !book.metadata.author.is_empty() {
                txt.push_str(&format!("作者：{}\n", book.metadata.author));
            }
            txt.push('\n');

            for ch in &book.chapters {
                txt.push_str(&format!("{}\n", ch.title));
                txt.push_str(ch.body.trim_end());
                txt.push_str("\n\n");
            }

            match std::fs::write(output, &txt) {
                Ok(()) => println!("✅ TXT written to {}", output.display()),
                Err(e) => {
                    eprintln!("Error writing {}: {}", output.display(), e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to parse EPUB: {}", e);
            std::process::exit(1);
        }
    }
}
