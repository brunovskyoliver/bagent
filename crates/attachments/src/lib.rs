use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Attachment kind ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    Pdf,
    Text,
    Other,
}

impl AttachmentKind {
    pub fn as_str(&self) -> &str {
        match self {
            AttachmentKind::Image => "image",
            AttachmentKind::Pdf   => "pdf",
            AttachmentKind::Text  => "text",
            AttachmentKind::Other => "other",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "image" => AttachmentKind::Image,
            "pdf"   => AttachmentKind::Pdf,
            "text"  => AttachmentKind::Text,
            _       => AttachmentKind::Other,
        }
    }
}

// ── Extraction result ─────────────────────────────────────────────────────────

pub struct ExtractResult {
    pub kind: AttachmentKind,
    /// Extracted text preview (UTF-8, max ~8 000 chars). None for images.
    pub extracted_text: Option<String>,
    /// True for images — caller should route to a vision model.
    pub requires_vision: bool,
}

const MAX_TEXT_CHARS: usize = 8_000;

/// Classify and extract text from an attachment at `path`.
/// `mime` is the MIME type reported by the HTTP multipart header.
pub fn extract(path: &Path, mime: &str) -> Result<ExtractResult> {
    // Images — pass through for vision model, no text extraction.
    if mime.starts_with("image/") {
        return Ok(ExtractResult {
            kind: AttachmentKind::Image,
            extracted_text: None,
            requires_vision: true,
        });
    }

    // PDFs — try macOS-available CLI tools.
    if mime == "application/pdf" {
        let text = extract_pdf_text(path);
        return Ok(ExtractResult {
            kind: AttachmentKind::Pdf,
            extracted_text: text,
            requires_vision: false,
        });
    }

    // Office documents — docx via textutil, xlsx via python zipfile.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext == "docx" || mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document" {
        if let Some(text) = extract_docx_text(path) {
            return Ok(ExtractResult {
                kind: AttachmentKind::Text,
                extracted_text: Some(text),
                requires_vision: false,
            });
        }
    }
    if ext == "xlsx" || ext == "xls"
        || mime == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        || mime == "application/vnd.ms-excel"
    {
        if let Some(text) = extract_xlsx_text(path) {
            return Ok(ExtractResult {
                kind: AttachmentKind::Text,
                extracted_text: Some(text),
                requires_vision: false,
            });
        }
    }

    // Plain text / source / markdown / data files.
    if mime.starts_with("text/") || is_text_extension(path) {
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|_| "[nedá sa čítať]".to_string());
        let preview = truncate_to_chars(&raw, MAX_TEXT_CHARS);
        return Ok(ExtractResult {
            kind: AttachmentKind::Text,
            extracted_text: Some(preview),
            requires_vision: false,
        });
    }

    Ok(ExtractResult {
        kind: AttachmentKind::Other,
        extracted_text: None,
        requires_vision: false,
    })
}

/// SHA-256 hex digest of the file at `path` — used for content-addressed
/// deduplication in the `attachments` table.
pub fn file_sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn is_text_extension(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "txt" | "md" | "markdown" | "csv" | "json" | "yaml" | "yml"
            | "rs"  | "swift" | "py"  | "js"   | "ts"  | "tsx" | "jsx"
            | "html"| "css"   | "sql" | "toml" | "sh"  | "xml" | "log"
    )
}

fn extract_pdf_text(path: &Path) -> Option<String> {
    let path_str = path.to_string_lossy();

    // Preferred: pdftotext (Homebrew / poppler).
    if let Ok(out) = std::process::Command::new("pdftotext")
        .args([path_str.as_ref(), "-"])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if !text.trim().is_empty() {
                return Some(truncate_to_chars(&text, MAX_TEXT_CHARS));
            }
        }
    }

    // Fallback: macOS textutil (handles some PDFs).
    if let Ok(out) = std::process::Command::new("textutil")
        .args(["-convert", "txt", "-stdout", path_str.as_ref()])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if !text.trim().is_empty() {
                return Some(truncate_to_chars(&text, MAX_TEXT_CHARS));
            }
        }
    }

    None
}

/// Extract text from a .docx file using macOS textutil.
fn extract_docx_text(path: &Path) -> Option<String> {
    let out = std::process::Command::new("textutil")
        .args(["-convert", "txt", "-stdout", &path.to_string_lossy()])
        .output().ok()?;
    if !out.status.success() { return None; }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() { return None; }
    Some(truncate_to_chars(&text, MAX_TEXT_CHARS))
}

/// Extract text from a .xlsx file by reading the shared strings XML inside the zip.
fn extract_xlsx_text(path: &Path) -> Option<String> {
    // Use python3 -c to avoid a dependency — available on macOS by default.
    let script = r#"
import sys, zipfile, xml.etree.ElementTree as ET
path = sys.argv[1]
texts = []
with zipfile.ZipFile(path) as z:
    # Shared strings (most cell text is here)
    if 'xl/sharedStrings.xml' in z.namelist():
        with z.open('xl/sharedStrings.xml') as f:
            tree = ET.parse(f)
            for si in tree.getroot():
                parts = [e.text or '' for e in si.iter() if e.text]
                val = ''.join(parts).strip()
                if val:
                    texts.append(val)
    # Inline strings in sheet1
    try:
        with z.open('xl/worksheets/sheet1.xml') as f:
            tree = ET.parse(f)
            ns = {'x': 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}
            for c in tree.findall('.//x:c[@t="inlineStr"]', ns):
                t = c.find('.//x:t', ns)
                if t is not None and t.text:
                    texts.append(t.text.strip())
    except: pass
print('\n'.join(texts))
"#;
    let out = std::process::Command::new("python3")
        .args(["-c", script, &path.to_string_lossy()])
        .output().ok()?;
    if !out.status.success() { return None; }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() { return None; }
    Some(truncate_to_chars(&text, MAX_TEXT_CHARS))
}

fn truncate_to_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "\n…[skrátené]"
    }
}
