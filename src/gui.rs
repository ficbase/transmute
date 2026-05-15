// transmute-gui — desktop GUI for txt↔epub conversion
use eframe::egui;
use std::path::PathBuf;
use transmute::{Book, Chapter, CoverImage, Metadata, parse_epub_file, write_epub_file};

fn main() -> Result<(), eframe::Error> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([520.0, 620.0])
            .with_title("Transmute · txt ↔ epub"),
        ..Default::default()
    };
    eframe::run_native(
        "Transmute",
        opts,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

// ── App state ────────────────────────────────────────────────────────

#[derive(Default)]
struct App {
    // File paths
    input_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    cover_path: Option<PathBuf>,

    // Metadata (txt→epub)
    title: String,
    author: String,
    language: String,
    description: String,
    publisher: String,
    identifier: String,

    // Status
    status: String,
    status_ok: bool,
}

impl App {
    fn mode(&self) -> Option<&str> {
        self.input_path.as_ref().and_then(|p| {
            match p.extension()?.to_str()?.to_lowercase().as_str() {
                "txt" => Some("txt2epub"),
                "epub" => Some("epub2txt"),
                _ => None,
            }
        })
    }

    fn auto_output(&self) -> Option<PathBuf> {
        let input = self.input_path.as_ref()?;
        let mut out = input.clone();
        let new_ext = match self.mode()? {
            "txt2epub" => "epub",
            "epub2txt" => "txt",
            _ => return None,
        };
        out.set_extension(new_ext);
        Some(out)
    }

    fn convert(&mut self) {
        let input = match &self.input_path {
            Some(p) => p.clone(),
            None => { self.status = "请先选择输入文件".into(); return; }
        };
        let output = self.output_path.clone()
            .or_else(|| self.auto_output())
            .unwrap_or_else(|| {
                let mut p = input.clone();
                p.set_extension("out");
                p
            });

        let result = match self.mode() {
            Some("txt2epub") => self.txt_to_epub(&input, &output),
            Some("epub2txt") => self.epub_to_txt(&input, &output),
            _ => { self.status = "不支持的文件格式".into(); return; }
        };

        match result {
            Ok(()) => {
                self.status = format!("✅ 已保存至 {}", output.display());
                self.status_ok = true;
            }
            Err(e) => {
                self.status = format!("转换失败: {e}");
                self.status_ok = false;
            }
        }
    }

    fn txt_to_epub(&self, input: &PathBuf, output: &PathBuf) -> Result<(), String> {
        let raw = std::fs::read(input).map_err(|e| format!("读取文件失败: {e}"))?;
        let txt = auto_decode(&raw)?;

        let chapters = split_into_chapters(&txt);
        let cover = self.cover_path.as_ref().and_then(|p| {
            let data = std::fs::read(p).ok()?;
            let ext = p.extension()?.to_str()?.to_lowercase();
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                _ => "image/jpeg",
            };
            let name = p.file_name()?.to_str()?.to_string();
            Some(CoverImage { data, media_type: mime.into(), file_name: name })
        });

        let book = Book {
            metadata: Metadata {
                title: if self.title.is_empty() { detect_title(&txt).unwrap_or_default() } else { self.title.clone() },
                author: if self.author.is_empty() { detect_author(&txt) } else { self.author.clone() },
                language: if self.language.is_empty() { "zh".into() } else { self.language.clone() },
                identifier: self.identifier.clone(),
                extra: {
                    let mut m = std::collections::HashMap::new();
                    if !self.description.is_empty() { m.insert("dcterms:description".into(), self.description.clone()); }
                    if !self.publisher.is_empty() { m.insert("dcterms:publisher".into(), self.publisher.clone()); }
                    m
                },
            },
            chapters,
            cover,
        };

        write_epub_file(&book, output).map_err(|e| format!("写入 EPUB 失败: {e}"))
    }

    fn epub_to_txt(&self, input: &PathBuf, output: &PathBuf) -> Result<(), String> {
        let book = parse_epub_file(input).map_err(|e| format!("解析 EPUB 失败: {e}"))?;
        let mut txt = String::new();
        if !book.metadata.title.is_empty() { txt.push_str(&format!("《{}》\n", book.metadata.title)); }
        if !book.metadata.author.is_empty() { txt.push_str(&format!("作者：{}\n", book.metadata.author)); }
        txt.push('\n');
        for ch in &book.chapters {
            txt.push_str(&format!("{}\n", ch.title));
            txt.push_str(ch.body.trim_end());
            txt.push_str("\n\n");
        }
        std::fs::write(output, &txt).map_err(|e| format!("写入 TXT 失败: {e}"))
    }
}

// ── GUI ──────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Transmute · txt ↔ epub");
            ui.separator();

            // ── Input file ──────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label("输入文件:");
                let label = self.input_path.as_ref()
                    .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                    .unwrap_or_else(|| "未选择".into());
                ui.add_enabled(false, egui::TextEdit::singleline(&mut label.as_str()));
                if ui.button("选择").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("文档", &["txt", "epub"])
                        .pick_file()
                    {
                        // auto-detect metadata from txt
                        if let Some("txt") = p.extension().and_then(|e| e.to_str()) {
                            if let Ok(raw) = std::fs::read(&p) {
                                if let Ok(txt) = auto_decode(&raw) {
                                    if self.title.is_empty() {
                                        if let Some(t) = detect_title(&txt) { self.title = t; }
                                    }
                                    if self.author.is_empty() {
                                        self.author = detect_author(&txt);
                                    }
                                }
                            }
                        }
                        self.input_path = Some(p);
                        self.output_path = self.auto_output();
                        self.status.clear();
                    }
                }
            });

            // ── Mode indicator ─────────────────────────────────────
            if let Some(mode) = self.mode() {
                ui.horizontal(|ui| {
                    ui.label("方向:");
                    ui.label(match mode {
                        "txt2epub" => "📄 TXT → 📗 EPUB",
                        "epub2txt" => "📗 EPUB → 📄 TXT",
                        _ => "未知",
                    });
                });
            }

            // ── Metadata (txt→epub only) ──────────────────────────
            if self.mode() == Some("txt2epub") {
                ui.separator();
                ui.label("📋 元数据:");
                egui::Grid::new("meta_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("书名"); ui.text_edit_singleline(&mut self.title); ui.end_row();
                    ui.label("作者"); ui.text_edit_singleline(&mut self.author); ui.end_row();
                    ui.label("语言"); ui.text_edit_singleline(&mut self.language); ui.end_row();
                    ui.label("标识符"); ui.text_edit_singleline(&mut self.identifier); ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.label("简介:");
                    ui.text_edit_multiline(&mut self.description);
                });
                ui.horizontal(|ui| {
                    ui.label("出版社:");
                    ui.text_edit_singleline(&mut self.publisher);
                });
                // Cover
                ui.horizontal(|ui| {
                    ui.label("封面:");
                    let cover_label = self.cover_path.as_ref()
                        .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                        .unwrap_or_else(|| "未选择".into());
                    ui.add_enabled(false, egui::TextEdit::singleline(&mut cover_label.as_str()));
                    if ui.button("选择封面").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("图片", &["jpg", "jpeg", "png"])
                            .pick_file()
                        {
                            self.cover_path = Some(p);
                        }
                    }
                    if self.cover_path.is_some() && ui.button("清除").clicked() {
                        self.cover_path = None;
                    }
                });
            }

            // ── Output ─────────────────────────────────────────────
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("输出文件:");
                let mut out_str = self.output_path.as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                if ui.text_edit_singleline(&mut out_str).changed() {
                    self.output_path = if out_str.is_empty() { None } else { Some(PathBuf::from(&out_str)) };
                }
                if ui.button("另存为...").clicked() {
                    let ext = match self.mode() {
                        Some("txt2epub") => "epub",
                        Some("epub2txt") => "txt",
                        _ => "out",
                    };
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("输出", &[ext])
                        .save_file()
                    {
                        self.output_path = Some(p);
                    }
                }
            });

            // ── Convert ────────────────────────────────────────────
            ui.separator();
            let can_convert = self.input_path.is_some();
            if ui.add_enabled(can_convert, egui::Button::new("🔄 开始转换")).clicked() {
                self.convert();
            }

            // ── Status ─────────────────────────────────────────────
            if !self.status.is_empty() {
                ui.add_space(4.0);
                if self.status_ok {
                    ui.colored_label(egui::Color32::from_rgb(0x22, 0xC5, 0x5E), &self.status);
                } else {
                    ui.colored_label(egui::Color32::from_rgb(0xFF, 0x6B, 0x6B), &self.status);
                }
            }
        });
    }
}

// ── Helpers (mirror main.rs but work on bytes) ───────────────────────

fn auto_decode(raw: &[u8]) -> Result<String, String> {
    use encoding_rs::Encoding;
    // BOM
    if raw.len() >= 3 && raw[0] == 0xEF && raw[1] == 0xBB && raw[2] == 0xBF {
        return Ok(Encoding::for_label(b"utf-8").unwrap().decode_without_bom_handling(&raw[3..]).0.into_owned());
    }
    if raw.len() >= 2 && raw[0] == 0xFF && raw[1] == 0xFE {
        return Ok(Encoding::for_label(b"utf-16le").unwrap().decode_without_bom_handling(&raw[2..]).0.into_owned());
    }
    if raw.len() >= 2 && raw[0] == 0xFE && raw[1] == 0xFF {
        return Ok(Encoding::for_label(b"utf-16be").unwrap().decode_without_bom_handling(&raw[2..]).0.into_owned());
    }
    let utf8 = Encoding::for_label(b"utf-8").unwrap();
    let (cow, _, had_errors) = utf8.decode(raw);
    if !had_errors { return Ok(cow.into_owned()); }
    Ok(Encoding::for_label(b"gbk").unwrap().decode(raw).0.into_owned())
}

fn detect_title(txt: &str) -> Option<String> {
    for line in txt.lines().take(20) {
        if let Some(rest) = line.trim().strip_prefix('《') {
            if let Some(title) = rest.split('》').next() {
                if !title.is_empty() { return Some(title.to_string()); }
            }
        }
    }
    None
}

fn detect_author(txt: &str) -> String {
    for line in txt.lines().take(20) {
        for sep in ["作者：", "作者:"] {
            if let Some(pos) = line.find(sep) {
                let a = line[pos + sep.len()..].trim();
                if !a.is_empty() { return a.to_string(); }
            }
        }
    }
    String::new()
}

fn split_into_chapters(text: &str) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut current_title = String::from("第1章");
    let mut current_body = String::new();
    let mut first = true;

    for line in text.lines() {
        let trimmed = line.trim();
        if is_chapter_heading(trimmed) {
            if !first {
                chapters.push(Chapter { title: current_title.clone(), body: std::mem::take(&mut current_body) });
            }
            current_title = trimmed.to_string();
            first = false;
        } else {
            if !current_body.is_empty() { current_body.push('\n'); }
            current_body.push_str(line);
        }
    }
    if !first || !current_body.is_empty() {
        if current_title.is_empty() { current_title = "第1章".into(); }
        chapters.push(Chapter { title: current_title, body: current_body });
    }
    chapters
}

fn is_chapter_heading(line: &str) -> bool {
    if line.starts_with("# ") { return true; }
    if line.starts_with("Chapter ") || line.starts_with("CHAPTER ") { return true; }
    let mut chars = line.chars();
    if chars.next() != Some('第') { return false; }
    let mut skipped = false;
    for c in chars.by_ref() {
        if c.is_ascii_digit() { skipped = true; }
        else if is_cn_digit(c) { skipped = true; }
        else if skipped && (c == '章' || c == '节' || c == '部' || c == '卷' || c == '篇' || c == '集') {
            return chars.next() != Some('分');
        } else { return false; }
    }
    false
}

fn is_cn_digit(c: char) -> bool {
    matches!(c, '零'|'一'|'二'|'三'|'四'|'五'|'六'|'七'|'八'|'九'|'十'|'百'|'千'|'万'|'两'|'〇')
}
