//! transmute — multi-format document conversion toolkit
//!
//! Currently supports: txt → epub (EPUB 3.3) with optional cover image.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use zip::read::ZipArchive;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

// ── EPUB document model ──────────────────────────────────────────────

/// EPUB book metadata.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub language: String,
    pub identifier: String,
    pub extra: HashMap<String, String>,
}

/// Image data for an EPUB cover.
#[derive(Debug, Clone)]
pub struct CoverImage {
    pub data: Vec<u8>,
    /// e.g. "image/jpeg", "image/png".
    pub media_type: String,
    /// File name inside `OEBPS/images/`, e.g. "cover.jpg".
    pub file_name: String,
}

/// A single chapter / section.
#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    pub body: String,
}

/// An EPUB book ready to serialize.
#[derive(Debug, Clone)]
pub struct Book {
    pub metadata: Metadata,
    pub chapters: Vec<Chapter>,
    pub cover: Option<CoverImage>,
}

// ── EPUB 3.3 constants ───────────────────────────────────────────────

const MIMETYPE: &str = "application/epub+zip";

const CONTAINER_XML: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">
  <rootfiles>
    <rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>
  </rootfiles>
</container>";

// ── EPUB writer ──────────────────────────────────────────────────────

pub fn write_epub<W: Write + io::Seek>(book: &Book, writer: W) -> Result<(), Error> {
    let metadata = &book.metadata;
    let chapters = &book.chapters;

    let uid = if metadata.identifier.is_empty() {
        uuid_v4()
    } else {
        metadata.identifier.clone()
    };

    let chapter_ids: Vec<String> = (0..chapters.len())
        .map(|i| format!("chapter{}", i + 1))
        .collect();

    // Always have a cover: use provided image, or auto-generate SVG
    let cover = book.cover.clone().or_else(|| {
        Some(CoverImage {
            data: generate_cover_svg(&metadata.title, &metadata.author).into_bytes(),
            media_type: "image/svg+xml".into(),
            file_name: "cover.svg".into(),
        })
    });

    let opf = make_opf(metadata, &uid, &chapter_ids, cover.as_ref());
    let nav = make_nav(metadata, chapters, &chapter_ids);

    let mut xhtmls: Vec<(String, String)> = Vec::new();
    for (i, ch) in chapters.iter().enumerate() {
        let id = &chapter_ids[i];
        let body_html = text_to_html(&ch.body);
        let xhtml = make_chapter_xhtml(&ch.title, &body_html);
        xhtmls.push((format!("OEBPS/{}.xhtml", id), xhtml));
    }

    let cover_xhtml = cover.as_ref().map(|c| {
        let cover_path = format!("images/{}", c.file_name);
        make_cover_xhtml(&metadata.title, &cover_path)
    });

    let mut zip = ZipWriter::new(writer);

    let store_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", store_opts)?;
    zip.write_all(MIMETYPE.as_bytes())?;

    let deflate_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("META-INF/container.xml", deflate_opts)?;
    zip.write_all(CONTAINER_XML.as_bytes())?;

    if let Some(ref c) = cover {
        let cover_path = format!("OEBPS/images/{}", c.file_name);
        zip.start_file(&cover_path, store_opts)?;
        zip.write_all(&c.data)?;
    }

    if let Some(ref cx) = cover_xhtml {
        zip.start_file("OEBPS/cover.xhtml", deflate_opts)?;
        zip.write_all(cx.as_bytes())?;
    }

    zip.start_file("OEBPS/content.opf", deflate_opts)?;
    zip.write_all(opf.as_bytes())?;

    zip.start_file("OEBPS/nav.xhtml", deflate_opts)?;
    zip.write_all(nav.as_bytes())?;

    for (path, content) in &xhtmls {
        zip.start_file(path.as_str(), deflate_opts)?;
        zip.write_all(content.as_bytes())?;
    }

    zip.finish()?;
    Ok(())
}

pub fn write_epub_file(book: &Book, path: impl AsRef<Path>) -> Result<(), Error> {
    let file = std::fs::File::create(path.as_ref())?;
    let buf = io::BufWriter::new(file);
    write_epub(book, buf)
}

// ── Text → HTML helpers ──────────────────────────────────────────────

pub fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 5);
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let last = paragraphs.len().saturating_sub(1);
    for (i, p) in paragraphs.iter().enumerate() {
        // strip trailing blank lines but keep leading indentation
        let body = p.trim_end_matches(|c: char| c == '\n' || c == '\r');
        if body.is_empty() {
            continue;
        }
        out.push_str("<p>");
        let lines: Vec<&str> = body.split('\n').collect();
        let ll = lines.len().saturating_sub(1);
        for (j, line) in lines.iter().enumerate() {
            out.push_str(&escape_xml(line));
            if j < ll {
                out.push_str("<br/>\n");
            }
        }
        out.push_str("</p>");
        if i < last {
            out.push('\n');
        }
    }
    out
}

fn escape_xml(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

// ── EPUB 3 XML generators ───────────────────────────────────────────

fn make_opf(meta: &Metadata, uid: &str, chapter_ids: &[String], cover: Option<&CoverImage>) -> String {
    let lang = if meta.language.is_empty() { "en" } else { &meta.language };
    let title = escape_xml(&meta.title);
    let author = escape_xml(&meta.author);

    let mut manifest = String::new();
    let mut spine = String::new();

    if let Some(ci) = cover {
        let img_href = format!("images/{}", ci.file_name);
        let mime = escape_xml(&ci.media_type);
        manifest.push_str(&format!(
            "    <item id=\"cover-image\" href=\"{img_href}\" media-type=\"{mime}\" properties=\"cover-image\"/>\n"
        ));
        manifest.push_str(
            "    <item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
        );
    }

    manifest.push_str(
        "    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n",
    );

    for id in chapter_ids {
        manifest.push_str(&format!(
            "    <item id=\"{}\" href=\"{}.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            id, id
        ));
        spine.push_str(&format!("    <itemref idref=\"{}\" linear=\"yes\"/>\n", id));
    }

    if cover.is_some() {
        let mut new_spine = String::from("    <itemref idref=\"cover\" linear=\"no\"/>\n");
        new_spine.push_str(&spine);
        spine = new_spine;
    }

    let extra_meta: String = meta
        .extra
        .iter()
        .map(|(k, v)| format!("    <meta property=\"{k}\">{v}</meta>\n", k = k, v = escape_xml(v)))
        .collect();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" unique-identifier="BookId"
         xmlns="http://www.idpf.org/2007/opf"
         prefix="rendition: http://www.idpf.org/vocab/rendition/#">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">{uid}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:creator id="author">{author}</dc:creator>
    <dc:language>{lang}</dc:language>
    <meta property="dcterms:modified">{modified}</meta>
{extra_meta}  </metadata>
  <manifest>
{manifest}  </manifest>
  <spine>
{spine}  </spine>
</package>"#,
        modified = iso8601_now()
    )
}

fn make_nav(meta: &Metadata, chapters: &[Chapter], chapter_ids: &[String]) -> String {
    let title = escape_xml(&meta.title);

    let mut ol = String::from("    <ol>\n");
    for (i, ch) in chapters.iter().enumerate() {
        let id = &chapter_ids[i];
        let ch_title = escape_xml(&ch.title);
        ol.push_str(&format!("      <li><a href=\"{id}.xhtml\">{ch_title}</a></li>\n"));
    }
    ol.push_str("    </ol>");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"
      xmlns:epub="http://www.idpf.org/2007/ops">
<head>
  <title>{title}</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>{title}</h1>
{ol}
  </nav>
</body>
</html>"#
    )
}

fn make_cover_xhtml(title: &str, img_path: &str) -> String {
    let title = escape_xml(title);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"
      xmlns:epub="http://www.idpf.org/2007/ops">
<head>
  <title>Cover</title>
</head>
<body>
  <div style="text-align:center;">
    <img src="{img_path}" alt="{title}" style="max-width:100%;"/>
  </div>
</body>
</html>"#
    )
}

fn make_chapter_xhtml(title: &str, body: &str) -> String {
    let title = escape_xml(title);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"
      xmlns:epub="http://www.idpf.org/2007/ops">
<head>
  <title>{title}</title>
</head>
<body>
  <h1>{title}</h1>
{body}
</body>
</html>"#
    )
}

// ── Auto-generated SVG cover ─────────────────────────────────────────

/// Generate a minimal SVG cover with title and author.
fn generate_cover_svg(title: &str, author: &str) -> String {
    let t = escape_xml(title);
    let a = escape_xml(author);
    let bg = "#1a1a2e";
    let fg = "#e8e8e8";
    let sub = "#999999";
    let author_block = if author.is_empty() {
        String::new()
    } else {
        format!("  <text x=\"400\" y=\"530\" font-family=\"serif\" font-size=\"24\" fill=\"{sub}\" text-anchor=\"middle\">{a}</text>")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"800\" height=\"1200\"\n\
     viewBox=\"0 0 800 1200\">\n\
  <rect width=\"800\" height=\"1200\" fill=\"{bg}\"/>\n\
  <text x=\"400\" y=\"450\" font-family=\"serif\" font-size=\"36\"\n\
        fill=\"{fg}\" text-anchor=\"middle\">{t}</text>\n\
{author_block}\n\
</svg>"
    )
}

// ── ISO 8601 timestamp ───────────────────────────────────────────────

fn iso8601_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;
    let (y, mo, d) = days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u8, u8) {
    days += 719_162;
    let era = days / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u8, d as u8)
}

// ── UUID v4 ──────────────────────────────────────────────────────────

fn uuid_v4() -> String {
    let mut rng = Lcg::new();
    let bytes: [u8; 16] = std::array::from_fn(|_| rng.next());
    let mut s = String::with_capacity(36);
    for (idx, &b) in bytes.iter().enumerate() {
        if idx == 4 || idx == 6 || idx == 8 || idx == 10 {
            s.push('-');
        }
        if idx == 6 {
            s.push(HEX[((0x40 | (b & 0x0f)) >> 4) as usize] as char);
            s.push(HEX[((0x40 | (b & 0x0f)) & 0x0f) as usize] as char);
        } else if idx == 8 {
            s.push(HEX[((0x80 | (b & 0x3f)) >> 4) as usize] as char);
            s.push(HEX[((0x80 | (b & 0x3f)) & 0x0f) as usize] as char);
        } else {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    s
}

const HEX: &[u8; 16] = b"0123456789abcdef";

struct Lcg { state: u64 }

impl Lcg {
    fn new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42);
        Self { state: seed ^ 0xDEADBEEFCAFE0000 }
    }
    fn next(&mut self) -> u8 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u8
    }
}

// ── Error ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Zip(zip::result::ZipError),
}

impl From<io::Error> for Error { fn from(e: io::Error) -> Self { Error::Io(e) } }
impl From<zip::result::ZipError> for Error { fn from(e: zip::result::ZipError) -> Self { Error::Zip(e) } }

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {}", e),
            Error::Zip(e) => write!(f, "ZIP error: {}", e),
        }
    }
}

impl std::error::Error for Error {}

// ── EPUB → Book parser ───────────────────────────────────────────────

/// Parse an EPUB file and return a `Book`.
pub fn parse_epub_file(path: impl AsRef<Path>) -> Result<Book, Error> {
    let file = std::fs::File::open(path.as_ref())?;
    let buf = io::BufReader::new(file);
    parse_epub(buf)
}

/// Parse EPUB from a reader.
pub fn parse_epub<R: Read + io::Seek>(reader: R) -> Result<Book, Error> {
    let mut zip = ZipArchive::new(reader)?;

    // 1. read container.xml to find OPF path
    let opf_path = {
        let container = read_zip_entry(&mut zip, "META-INF/container.xml")?;
        find_opf_path(&container)
    };

    // 2. read OPF for metadata and spine
    let opf_xml = read_zip_entry(&mut zip, &opf_path)?;
    let (metadata, spine, manifest) = parse_opf(&opf_xml);

    // 3. read each spine item, convert to Chapter
    let mut chapters = Vec::new();
    for idref in &spine {
        let href = manifest.get(idref).cloned().unwrap_or_default();
        let xhtml = read_zip_entry(&mut zip, &href).ok().unwrap_or_default();
        let (title, body) = parse_xhtml(&xhtml);
        chapters.push(Chapter { title, body });
    }

    Ok(Book {
        metadata,
        chapters,
        cover: None,
    })
}

fn read_zip_entry(zip: &mut ZipArchive<impl Read + io::Seek>, name: &str) -> Result<String, Error> {
    // try exact and OEBPS-prefixed
    let candidates = [
        name.to_string(),
        format!("OEBPS/{}", name.trim_start_matches("OEBPS/")),
    ];
    for c in &candidates {
        if let Ok(mut f) = zip.by_name(c) {
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            return Ok(buf);
        }
    }
    Err(Error::Io(io::Error::new(
        io::ErrorKind::NotFound,
        format!("entry not found: {}", name),
    )))
}

fn find_opf_path(container_xml: &str) -> String {
    // <rootfile full-path="OEBPS/content.opf"
    for line in container_xml.lines() {
        if let Some(start) = line.find("full-path=\"") {
            let rest = &line[start + 11..];
            if let Some(end) = rest.find('"') {
                return rest[..end].to_string();
            }
        }
    }
    "OEBPS/content.opf".to_string()
}

fn parse_opf(xml: &str) -> (Metadata, Vec<String>, HashMap<String, String>) {
    let mut meta = Metadata::default();
    let mut spine = Vec::new();
    let mut manifest: HashMap<String, String> = HashMap::new();

    for line in xml.lines() {
        let t = line.trim();
        if let Some(val) = extract_xml_content(t, "dc:title") {
            meta.title = val;
        } else if let Some(val) = extract_xml_content(t, "dc:creator") {
            meta.author = val;
        } else if let Some(val) = extract_xml_content(t, "dc:language") {
            meta.language = val;
        } else if let Some(val) = extract_xml_content(t, "dc:identifier") {
            meta.identifier = val;
        } else if let Some(idref) = extract_attr(t, "itemref", "idref") {
            // skip cover and nav pages
            if idref != "cover" && idref != "nav" {
                spine.push(idref);
            }
        } else if let Some(id) = extract_attr(t, "item", "id") {
            if let Some(href) = extract_attr(t, "item", "href") {
                manifest.insert(id, format!("OEBPS/{}", href));
            }
        }
    }

    (meta, spine, manifest)
}

/// Extract content between tags, e.g. `<dc:title>Foo</dc:title>` → `Foo`.
fn extract_xml_content(line: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let line = line.trim();
    if let Some(open_pos) = line.find(&open) {
        // find > after the open tag
        let after_open = &line[open_pos..];
        let tag_end = after_open.find('>')?;
        let content_start = open_pos + tag_end + 1;
        // find the close tag after open
        if let Some(close_pos) = line[content_start..].find(&close) {
            let content = line[content_start..content_start + close_pos].trim();
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }
    }
    None
}

/// Extract an attribute, e.g. `<item id="foo"` → `Some("foo")`.
fn extract_attr(line: &str, tag: &str, attr: &str) -> Option<String> {
    let line = line.trim();
    let tag_start = format!("<{} ", tag);
    if !line.starts_with(&tag_start) && !line.starts_with(&format!("<{tag}>")) {
        // allow <itemref ...>
        if !line.starts_with(&format!("<{tag} ")) {
            return None;
        }
    }
    let search = format!("{}=\"", attr);
    if let Some(pos) = line.find(&search) {
        let rest = &line[pos + search.len()..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Parse XHTML: extract `<title>` and strip HTML tags for body.
fn parse_xhtml(xhtml: &str) -> (String, String) {
    let title = if let Some(t) = extract_xml_content(xhtml, "title") {
        t
    } else {
        // fallback: extract h1
        xhtml
            .find("<h1>")
            .and_then(|s| {
                let rest = &xhtml[s + 4..];
                rest.find("</h1>").map(|e| rest[..e].to_string())
            })
            .unwrap_or_default()
    };

    // Remove <h1>...</h1> and <title>...</title> from body to avoid title duplication
    let mut body_src = xhtml.to_string();
    // strip <title>...</title>
    if let Some(start) = body_src.find("<title>") {
        if let Some(end) = body_src.find("</title>") {
            body_src.replace_range(start..end + 8, "");
        }
    }
    // strip <h1>...</h1>
    if let Some(start) = body_src.find("<h1>") {
        if let Some(end) = body_src.find("</h1>") {
            body_src.replace_range(start..end + 5, "");
        }
    }

    let body = html_to_text(&body_src);

    (title, body)
}

/// Strip HTML tags for plain text output.
pub fn html_to_text(html: &str) -> String {
    let squashed = html.replace('\n', "");
    let mut out = String::with_capacity(squashed.len());
    let mut skip = 0u32;
    // Keep a small ring buffer of the last 5 chars to detect tag endings
    let mut ring = ['\0'; 5];
    let mut ri = 0;

    for c in squashed.chars() {
        ring[ri % 5] = c;
        ri = ri.wrapping_add(1);

        if c == '<' {
            skip += 1;
        }
        if skip == 0 {
            out.push(c);
        }
        if c == '>' && skip > 0 {
            skip -= 1;
            // Check what tag just closed using the ring buffer
            let i = ri.wrapping_sub(1) % 5;
            let prev4 = |n: usize| ring[(i.wrapping_sub(n)) % 5];
            let is_br =
                // <br>
                (prev4(1) == 'r' && prev4(2) == 'b' && prev4(3) == '<')
                // <br/>
                || (prev4(1) == '/' && prev4(2) == 'r' && prev4(3) == 'b' && prev4(4) == '<')
                // <br />
                || (prev4(1) == '/' && prev4(2) == ' ' && prev4(3) == 'r' && prev4(4) == 'b');
            if is_br {
                out.push('\n');
            }
            // </p> → paragraph break
            if prev4(1) == 'p' && prev4(2) == '/' && prev4(3) == '<' {
                out.push('\n');
                out.push('\n');
            }
        }
    }

    let mut result = String::with_capacity(out.len());
    let mut prev = '\0';
    for c in out.chars() {
        if c == ' ' && prev == ' ' { continue; }
        result.push(c);
        prev = c;
    }
    let result = result.trim().to_string();
    collapse_newlines(&result)
}

fn collapse_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0;
    for c in s.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(c);
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    out.trim().to_string()
}