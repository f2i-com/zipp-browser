//! What a navigation can load besides HTML.
//!
//! Every top-level load used to be decoded as text and handed to the HTML
//! parser, so a PDF, an image or a zip rendered as a screen of mojibake. Here a
//! load is classified by its `Content-Type` (falling back to the file
//! extension and the leading bytes), and anything that is not HTML becomes an
//! ordinary HTML page -- a PDF viewer, an image viewer, a text viewer, or a
//! note that the file was saved to Downloads -- so the toolbar, layout, paint
//! and scrolling treat it like any other page.
//!
//! PDFs are rasterised with hayro (pure Rust), one PNG per page.

use std::path::{Path, PathBuf};

use base64::Engine as _;

/// Pages rendered for one PDF. Past this the viewer says how many it left out
/// rather than building a document of unbounded size.
const MAX_PDF_PAGES: usize = 200;

/// PDF points are 1/72 inch and CSS pixels 1/96: a page is rendered at exactly
/// the size it is shown, so it is sharp at the default zoom.
const PDF_SCALE: f32 = 96.0 / 72.0;

/// Load `url` as a top-level document: the page's own HTML, or a viewer page
/// for anything else.
///
/// With `allow_download` false (prerendering), content that would be saved to
/// disk is an error instead -- a background prefetch must never write files.
pub(crate) fn fetch_document(url: &str, allow_download: bool) -> Result<String, String> {
    if let Some(path) = url.strip_prefix("file://") {
        let path = local_path(path);
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mime = mime_from_extension(&path);
        return classify(url, bytes, mime.map(str::to_string), None, allow_download);
    }

    let response = crate::HTTP_CLIENT.get(url).send().map_err(|e| e.to_string())?;
    let header = |name: reqwest::header::HeaderName| {
        response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
    };
    let mime = header(reqwest::header::CONTENT_TYPE)
        .map(|ct| ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
        .filter(|m| !m.is_empty());
    let disposition_name = header(reqwest::header::CONTENT_DISPOSITION).and_then(|d| disposition_filename(&d));

    // HTML and text go through `text()`, which honours the declared charset.
    if mime.as_deref().is_some_and(|m| is_html_mime(m) || is_text_mime(m)) {
        let text = response.text().map_err(|e| e.to_string())?;
        return Ok(if mime.as_deref().is_some_and(is_html_mime) {
            text
        } else {
            text_viewer_html(url, &text)
        });
    }
    let bytes = response.bytes().map_err(|e| e.to_string())?.to_vec();
    classify(url, bytes, mime, disposition_name, allow_download)
}

/// Decide what non-HTML-by-header content is, from its type and bytes.
fn classify(
    url: &str,
    bytes: Vec<u8>,
    mime: Option<String>,
    filename: Option<String>,
    allow_download: bool,
) -> Result<String, String> {
    let mime = mime.as_deref();
    if mime == Some("application/pdf") || bytes.starts_with(b"%PDF-") {
        return pdf_viewer_html(url, &bytes, None);
    }
    if let Some(image_mime) = sniff_image(&bytes).or(mime.filter(|m| m.starts_with("image/"))) {
        return Ok(image_viewer_html(url, &bytes, image_mime));
    }
    if mime.is_some_and(is_html_mime) || (mime.is_none() && looks_like_html(&bytes)) {
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    if mime.is_some_and(is_text_mime) || (mime.is_none() && looks_like_text(&bytes)) {
        return Ok(text_viewer_html(url, &String::from_utf8_lossy(&bytes)));
    }
    if !allow_download {
        return Err(format!("not a page ({})", mime.unwrap_or("unknown type")));
    }
    let name = filename.unwrap_or_else(|| filename_from_url(url));
    let saved = save_to_downloads(&name, &bytes)?;
    eprintln!("[DOWNLOAD] {url} -> {}", saved.display());
    Ok(download_page_html(url, &saved, bytes.len()))
}

/// `zipp-save:<url>` -- the PDF viewer's Save button. Fetches the file again,
/// writes it to Downloads, and returns the viewer with a note saying where.
pub(crate) fn save_pdf(url: &str) -> Result<String, String> {
    let bytes = if let Some(path) = url.strip_prefix("file://") {
        std::fs::read(local_path(path)).map_err(|e| e.to_string())?
    } else {
        let response = crate::HTTP_CLIENT.get(url).send().map_err(|e| e.to_string())?;
        response.bytes().map_err(|e| e.to_string())?.to_vec()
    };
    let saved = save_to_downloads(&filename_from_url(url), &bytes)?;
    eprintln!("[DOWNLOAD] {url} -> {}", saved.display());
    pdf_viewer_html(url, &bytes, Some(&format!("Saved to {}", saved.display())))
}

// ---------------------------------------------------------------------------
// Viewers
// ---------------------------------------------------------------------------

fn pdf_viewer_html(url: &str, bytes: &[u8], notice: Option<&str>) -> Result<String, String> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("not a readable PDF: {e:?}"))?;
    let pages = pdf.pages();
    let total = pages.len();
    let cache = RenderCache::new();
    let interpreter = InterpreterSettings::default();
    let pixmap_settings = PixmapSettings { x_scale: PDF_SCALE, y_scale: PDF_SCALE, bg_color: WHITE };

    let mut body = String::new();
    for (index, page) in pages.iter().take(MAX_PDF_PAGES).enumerate() {
        let pixmap = render(page, &cache, &interpreter, &RenderSettings::default(), &pixmap_settings);
        let (width, height) = (pixmap.width(), pixmap.height());
        let png = pixmap.into_png().map_err(|e| format!("page {}: {e:?}", index + 1))?;
        body.push_str(&format!(
            r#"<img class="pdf-page" alt="Page {n}" width="{width}" height="{height}" src="data:image/png;base64,{data}">"#,
            n = index + 1,
            data = base64::engine::general_purpose::STANDARD.encode(png),
        ));
    }
    if total > MAX_PDF_PAGES {
        body.push_str(&format!(
            r#"<p class="pdf-more">Showing the first {MAX_PDF_PAGES} of {total} pages.</p>"#
        ));
    }

    let name = escape(&filename_from_url(url));
    let count = if total == 1 { "1 page".to_string() } else { format!("{total} pages") };
    let notice = notice
        .map(|n| format!(r#"<div class="pdf-notice">{}</div>"#, escape(n)))
        .unwrap_or_default();
    Ok(format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><title>{name}</title><style>
html,body{{margin:0;background:#525659}}
.pdf-bar{{display:flex;align-items:center;gap:16px;height:44px;padding:0 16px;background:#323639;color:#f1f3f4;font:14px Arial,sans-serif}}
.pdf-name{{flex:1;min-width:0;overflow:hidden;white-space:nowrap}}
.pdf-count{{color:#bdc1c6;white-space:nowrap}}
.pdf-save{{color:#f1f3f4;text-decoration:none;border:1px solid #6f7378;border-radius:4px;padding:5px 14px}}
.pdf-notice{{background:#264d37;color:#d6f5df;font:13px Arial,sans-serif;padding:9px 16px}}
.pdf-pages{{padding:16px 0 8px}}
.pdf-page{{display:block;margin:0 auto 16px;max-width:calc(100% - 32px);height:auto;background:#fff;box-shadow:0 2px 6px rgba(0,0,0,.45)}}
.pdf-more{{color:#e8eaed;font:14px Arial,sans-serif;text-align:center;margin:8px 0 24px}}
</style></head><body><div class="pdf-bar"><span class="pdf-name">{name}</span><span class="pdf-count">{count}</span><a class="pdf-save" href="zipp-save:{href}">Save</a></div>{notice}<div class="pdf-pages">{body}</div></body></html>"#,
        href = escape(url),
    ))
}

fn image_viewer_html(url: &str, bytes: &[u8], mime: &str) -> String {
    let name = escape(&filename_from_url(url));
    // A data URI rather than the URL: the bytes are already here, and a URL that
    // needed cookies or a one-time token would not load a second time.
    let src = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>{name}</title><style>
html,body{{margin:0;background:#0e0e0e}}
img{{display:block;margin:0 auto;max-width:100%;height:auto}}
</style></head><body><img alt="{name}" src="{src}"></body></html>"#
    )
}

fn text_viewer_html(url: &str, text: &str) -> String {
    let name = escape(&filename_from_url(url));
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>{name}</title><style>
html,body{{margin:0;background:#fff}}
pre{{margin:0;padding:12px 16px;white-space:pre-wrap;overflow-wrap:break-word;font:13px/1.5 Consolas,"Courier New",monospace;color:#202124}}
</style></head><body><pre>{}</pre></body></html>"#,
        escape(text)
    )
}

fn download_page_html(url: &str, saved: &Path, size: usize) -> String {
    let name = escape(&saved.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Downloaded {name}</title><style>
html,body{{margin:0;background:#f8f9fa;font:15px Arial,sans-serif;color:#202124}}
.dl{{max-width:640px;margin:64px auto;padding:28px 32px;background:#fff;border:1px solid #dadce0;border-radius:12px}}
h1{{font-size:22px;font-weight:600;margin:0 0 6px}}
p{{margin:10px 0;line-height:1.5;overflow-wrap:anywhere}}
.k{{color:#5f6368;font-size:13px}}
</style></head><body><div class="dl"><h1>Downloaded {name}</h1>
<p class="k">{size} from</p><p>{src}</p><p class="k">Saved to</p><p>{path}</p></div></body></html>"#,
        size = human_size(size),
        src = escape(url),
        path = escape(&saved.display().to_string()),
    )
}

// ---------------------------------------------------------------------------
// Classification helpers
// ---------------------------------------------------------------------------

fn is_html_mime(mime: &str) -> bool {
    matches!(mime, "text/html" | "application/xhtml+xml")
}

fn is_text_mime(mime: &str) -> bool {
    // `image/svg+xml` is XML, but it is an image: show it, not its source.
    !is_html_mime(mime)
        && !mime.starts_with("image/")
        && (mime.starts_with("text/")
            || matches!(
                mime,
                "application/json" | "application/xml" | "application/javascript" | "application/x-javascript"
            )
            || mime.ends_with("+json")
            || mime.ends_with("+xml"))
}

fn mime_from_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" | "xhtml" => "text/html",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "txt" | "md" | "log" | "csv" | "rs" | "toml" | "ini" | "cfg" => "text/plain",
        "json" => "application/json",
        "xml" => "application/xml",
        "js" | "mjs" => "application/javascript",
        "css" => "text/css",
        _ => return None,
    })
}

fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") && bytes.len() > 26 {
        Some("image/bmp")
    } else {
        None
    }
}

fn looks_like_text(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    !head.contains(&0) && std::str::from_utf8(head).is_ok_or_truncated()
}

fn looks_like_html(bytes: &[u8]) -> bool {
    if !looks_like_text(bytes) {
        return false;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<!doctype html") || head.starts_with("<html") || head.contains("<head") || head.contains("<body")
}

/// A UTF-8 check that tolerates a multi-byte character cut off by the 4KB head.
trait Utf8Head {
    fn is_ok_or_truncated(&self) -> bool;
}
impl Utf8Head for Result<&str, std::str::Utf8Error> {
    fn is_ok_or_truncated(&self) -> bool {
        match self {
            Ok(_) => true,
            Err(e) => e.error_len().is_none(),
        }
    }
}

/// `filename="a.pdf"` or `filename*=UTF-8''a%20b.pdf` from a Content-Disposition.
fn disposition_filename(disposition: &str) -> Option<String> {
    for part in disposition.split(';').map(str::trim) {
        if let Some(value) = part.strip_prefix("filename*=") {
            let encoded = value.rsplit("''").next().unwrap_or(value);
            if let Ok(decoded) = urlencoding::decode(encoded.trim_matches('"')) {
                return Some(decoded.into_owned()).filter(|n| !n.is_empty());
            }
        }
    }
    disposition
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("filename="))
        .map(|v| v.trim_matches('"').to_string())
        .filter(|n| !n.is_empty())
}

fn filename_from_url(url: &str) -> String {
    let path = url::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_else(|_| url.to_string());
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let decoded = urlencoding::decode(last).map(|c| c.into_owned()).unwrap_or_else(|_| last.to_string());
    if decoded.is_empty() { "download".to_string() } else { decoded }
}

/// `file:///C:/x`, `file://C:/x` and `file:///c%3A/x` alike.
fn local_path(path: &str) -> PathBuf {
    let path = path.trim_start_matches('/');
    PathBuf::from(urlencoding::decode(path).map(|c| c.into_owned()).unwrap_or_else(|_| path.to_string()))
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

fn downloads_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// Write `bytes` to Downloads under a safe form of `name`, never overwriting:
/// `a.pdf`, then `a (1).pdf`, `a (2).pdf`, ...
fn save_to_downloads(name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = downloads_dir();
    let safe = sanitize_filename(name);
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (safe.clone(), String::new()),
    };
    let mut path = dir.join(&safe);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("cannot save {}: {e}", path.display()))?;
    Ok(path)
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() { "download".to_string() } else { cleaned }
}

fn human_size(bytes: usize) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} bytes")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_type_and_content() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_image(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(sniff_image(b"%PDF-1.4"), None);
        assert!(is_text_mime("application/json") && is_text_mime("text/plain"));
        assert!(!is_text_mime("text/html") && is_html_mime("text/html"));
        assert!(is_text_mime("application/ld+json") && !is_text_mime("image/svg+xml"));
        assert!(looks_like_html(b"\xef\xbb\xbf<!DOCTYPE html><html>"));
        assert!(!looks_like_html(b"just some notes"));
        assert!(looks_like_text(b"plain text") && !looks_like_text(b"PK\x03\x04\x00\x00"));
    }

    #[test]
    fn filenames() {
        assert_eq!(filename_from_url("https://x.com/a/Lance%20CV.pdf?v=2"), "Lance CV.pdf");
        assert_eq!(filename_from_url("https://x.com/"), "download");
        assert_eq!(disposition_filename(r#"attachment; filename="r.pdf""#).as_deref(), Some("r.pdf"));
        assert_eq!(disposition_filename("attachment; filename*=UTF-8''a%20b.zip").as_deref(), Some("a b.zip"));
        assert_eq!(sanitize_filename(r#"..\evil/name?.zip"#), "_evil_name_.zip");
        assert_eq!(sanitize_filename("   "), "download");
    }
}
