use crate::schema::PageDoc;
use std::collections::HashSet;

// =============================================================================
// Export Types
// =============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    PlainText,
    Markdown,
    Html,
    Epub,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportScope {
    SinglePage,
    WholeCrawlOneFile,
    WholeCrawlFolder,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportContent {
    ContentOnly,
    WithMetadata,
    WithAssets,
    Full,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportRequest {
    pub format: ExportFormat,
    pub scope: ExportScope,
    pub content: ExportContent,
    pub page_url: Option<String>,
    pub crawl_id: Option<String>,
    pub source: Option<crate::commands::StorageSource>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportResult {
    pub path: String,
    pub page_count: usize,
    pub format: String,
    pub scope: String,
}

impl ExportRequest {
    pub fn format_string(&self) -> String {
        match self.format {
            ExportFormat::PlainText => "plain_text".to_string(),
            ExportFormat::Markdown => "markdown".to_string(),
            ExportFormat::Html => "html".to_string(),
            ExportFormat::Epub => "epub".to_string(),
        }
    }

    pub fn scope_string(&self) -> String {
        match self.scope {
            ExportScope::SinglePage => "single_page".to_string(),
            ExportScope::WholeCrawlOneFile => "whole_crawl_one_file".to_string(),
            ExportScope::WholeCrawlFolder => "whole_crawl_folder".to_string(),
        }
    }

    pub fn is_valid(&self) -> Result<(), String> {
        match self.format {
            ExportFormat::Epub => {
                match self.scope {
                    ExportScope::SinglePage | ExportScope::WholeCrawlFolder => {
                        return Err("EPUB only supports Whole Crawl as a single file".to_string());
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        match self.scope {
            ExportScope::SinglePage => {
                if self.page_url.is_none() || self.page_url.as_ref().unwrap().is_empty() {
                    return Err("Single page export requires a page_url".to_string());
                }
            }
            _ => {
                if self.crawl_id.is_none() || self.crawl_id.as_ref().unwrap().is_empty() {
                    return Err("Whole crawl export requires a crawl_id".to_string());
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// Plain Text Renders
// =============================================================================

pub fn page_to_plain_text(page: &PageDoc, content: &ExportContent) -> String {
    match content {
        ExportContent::ContentOnly => page_content_only_plain(page),
        ExportContent::WithMetadata => page_with_metadata_plain(page),
        ExportContent::WithAssets => page_with_assets_plain(page),
        ExportContent::Full => page_full_plain(page),
    }
}

fn page_content_only_plain(page: &PageDoc) -> String {
    if is_extraction_failed(page) {
        return extraction_failed_plain(page);
    }
    get_body_text(page).to_string()
}

fn page_with_metadata_plain(page: &PageDoc) -> String {
    let mut out = String::new();
    let title = get_title(page);
    out.push_str(&format!("Title: {}\n", title));
    out.push_str(&format!("URL: {}\n", page.url));
    out.push_str(&format!("Archived: {}\n", page.timestamp));
    if let Some(ref author) = page.author {
        out.push_str(&format!("Author: {}\n", author));
    }
    if let Some(ref published) = page.published_date {
        out.push_str(&format!("Published: {}\n", published));
    }
    out.push_str("---\n");
    out.push_str(get_body_text(page));
    out
}

fn page_with_assets_plain(page: &PageDoc) -> String {
    let mut out = page_with_metadata_plain(page);
    if let Some(ref assets) = page.assets {
        if !assets.images.is_empty() {
            out.push_str("\n\nImages\n");
            for img in &assets.images {
                out.push_str(&format!("- {}\n", img.src));
            }
        }
        if !assets.documents.is_empty() {
            out.push_str("\n\nDocuments\n");
            for doc in &assets.documents {
                out.push_str(&format!("- {}\n", doc.src));
            }
        }
    }
    out
}

fn page_full_plain(page: &PageDoc) -> String {
    let mut out = page_with_assets_plain(page);
    if let Some(ref assets) = page.assets {
        if !assets.videos.is_empty() {
            out.push_str("\n\nVideos\n");
            for video in &assets.videos {
                out.push_str(&format!("- {}\n", video.src));
            }
        }
    }
    out.push_str("\n\nLinked pages\n");
    out.push_str(&format!("- {}\n", page.url));
    out
}

// =============================================================================
// Markdown Renders
// =============================================================================

pub fn page_to_markdown(page: &PageDoc, content: &ExportContent) -> String {
    match content {
        ExportContent::ContentOnly => {
            if is_extraction_failed(page) {
                extraction_failed_markdown(page)
            } else {
                get_body_text(page).to_string()
            }
        }
        ExportContent::WithMetadata => page_doc_to_markdown(page),
        ExportContent::WithAssets => page_to_markdown_with_assets(page),
        ExportContent::Full => page_to_markdown_full(page),
    }
}

fn page_to_markdown_with_assets(page: &PageDoc) -> String {
    let mut out = page_doc_to_markdown(page);
    if let Some(ref assets) = page.assets {
        if !assets.images.is_empty() {
            out.push_str(&format!("\n\n## Images ({})\n\n", assets.images.len()));
            for img in &assets.images {
                let alt = img.alt.as_deref().unwrap_or("image");
                out.push_str(&format!("- ![{}]({})\n", alt, img.src));
            }
        }
        if !assets.documents.is_empty() {
            out.push_str(&format!("\n\n## Documents ({})\n\n", assets.documents.len()));
            for doc in &assets.documents {
                let label = doc.link_text.as_deref().unwrap_or(&doc.src);
                out.push_str(&format!("- [{}]({})\n", label, doc.src));
            }
        }
    }
    out
}

fn page_to_markdown_full(page: &PageDoc) -> String {
    let mut out = page_to_markdown_with_assets(page);
    if let Some(ref assets) = page.assets {
        if !assets.videos.is_empty() {
            out.push_str(&format!("\n\n## Videos ({})\n\n", assets.videos.len()));
            for video in &assets.videos {
                out.push_str(&format!("- [{}]({})\n", video.kind, video.src));
            }
        }
    }
    out.push_str("\n\n## Asset Inventory\n\n");
    out.push_str("| Type | Source |\n");
    out.push_str("|------|--------|\n");
    if let Some(ref assets) = page.assets {
        for img in &assets.images {
            out.push_str(&format!("| Image | {} |\n", img.src));
        }
        for doc in &assets.documents {
            out.push_str(&format!("| Document | {} |\n", doc.src));
        }
    }
    out
}

// =============================================================================
// HTML Renders
// =============================================================================

pub fn page_to_html(page: &PageDoc, content: &ExportContent) -> String {
    match content {
        ExportContent::ContentOnly => page_html_content_only(page),
        ExportContent::WithMetadata => page_doc_to_html(page),
        ExportContent::WithAssets => page_html_with_assets(page),
        ExportContent::Full => page_html_full(page),
    }
}

fn page_html_content_only(page: &PageDoc) -> String {
    let body = get_body_html(page);
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <meta charset=\"UTF-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <style>{}</style>\n</head>\n<body>\n{}\n</body>\n</html>",
        minimal_reading_stylesheet(),
        body
    )
}

fn page_html_with_assets(page: &PageDoc) -> String {
    let mut out = page_doc_to_html(page);
    if let Some(ref assets) = page.assets {
        if !assets.images.is_empty() || !assets.documents.is_empty() {
            let mut aside = String::from("<aside class=\"assets\">\n");
            if !assets.images.is_empty() {
                aside.push_str("<h3>Images</h3>\n<ul>\n");
                for img in &assets.images {
                    let alt = img.alt.as_deref().unwrap_or("image");
                    aside.push_str(&format!("<li><img src=\"{}\" alt=\"{}\" style=\"max-width:200px;\"></li>\n", html_escape_attr(&img.src), html_escape_attr(alt)));
                }
                aside.push_str("</ul>\n");
            }
            if !assets.documents.is_empty() {
                aside.push_str("<h3>Documents</h3>\n<ul>\n");
                for doc in &assets.documents {
                    let label = doc.link_text.as_deref().unwrap_or(&doc.src);
                    aside.push_str(&format!("<li><a href=\"{}\">{}</a></li>\n", html_escape_attr(&doc.src), html_escape_text(label)));
                }
                aside.push_str("</ul>\n");
            }
            aside.push_str("</aside>\n");
            out = out.replace("</body>", &format!("{}</body>", aside));
        }
    }
    out
}

fn page_html_full(page: &PageDoc) -> String {
    let mut out = page_html_with_assets(page);
    let mut table = String::from("<h2>Asset Inventory</h2>\n<table border=\"1\">\n<tr><th>Type</th><th>Source</th></tr>\n");
    if let Some(ref assets) = page.assets {
        for img in &assets.images {
            table.push_str(&format!("<tr><td>Image</td><td>{}</td></tr>\n", html_escape_text(&img.src)));
        }
        for doc in &assets.documents {
            table.push_str(&format!("<tr><td>Document</td><td>{}</td></tr>\n", html_escape_text(&doc.src)));
        }
    }
    table.push_str("</table>\n");
    out = out.replace("</body>", &format!("{}</body>", table));
    out
}

fn minimal_reading_stylesheet() -> &'static str {
    "body { max-width: 720px; margin: 2rem auto; font-family: Georgia, serif; font-size: 1.1rem; line-height: 1.7; color: #1a1a1a; padding: 0 1rem; } img { max-width: 100%; height: auto; } a { color: #1a6ea8; } blockquote { border-left: 3px solid #ccc; margin: 0; padding-left: 1rem; color: #555; } pre { background: #f5f5f5; padding: 1rem; overflow-x: auto; } figure { margin: 1.5rem 0; } figcaption { font-size: 0.85rem; color: #666; text-align: center; margin-top: 0.5rem; } .extraction-failed { border: 2px dashed #e0a800; background: #fffbec; padding: 1.5rem; border-radius: 4px; color: #7a5c00; margin: 1rem 0; } .extraction-failed p { margin: 0.5rem 0; }"
}

// =============================================================================
// Multi-page renders
// =============================================================================

pub fn pages_to_plain_text_combined(pages: &[PageDoc], content: &ExportContent) -> String {
    let mut out = String::new();
    for (i, page) in pages.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n---\n\n");
        }
        out.push_str(&format!("{}", page.url));
        out.push_str(&format!("\n\n{}", page_to_plain_text(page, content)));
    }
    out
}

pub fn pages_to_markdown_combined(pages: &[PageDoc], content: &ExportContent) -> String {
    let mut out = String::new();
    for (i, page) in pages.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n---\n\n");
        }
        let title = get_title(page);
        out.push_str(&format!("## {}\n\n", title));
        out.push_str(&page_to_markdown(page, content));
    }
    out
}

pub fn pages_to_html_combined(pages: &[PageDoc], content: &ExportContent) -> String {
    let mut articles = String::new();
    let mut toc_items = String::new();

    for (i, page) in pages.iter().enumerate() {
        let title = get_title(page);
        toc_items.push_str(&format!("<li><a href=\"#page-{}\">{}</a></li>\n", i, html_escape_text(title)));

        let article_content = match content {
            ExportContent::ContentOnly => page_html_content_only(page),
            ExportContent::WithMetadata => page_doc_to_html(page),
            ExportContent::WithAssets => page_html_with_assets(page),
            ExportContent::Full => page_html_full(page),
        };

        let body_start = article_content.find("<body>").unwrap_or(0) + 6;
        let body_end = article_content.rfind("</body>").unwrap_or(article_content.len());
        let body_content = &article_content[body_start..body_end];

        articles.push_str(&format!("<article id=\"page-{}\">\n{}\n</article>\n", i, body_content));
    }

    let seed_url = pages.first().map(|p| p.url.as_str()).unwrap_or("Crawl Archive");

    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <meta charset=\"UTF-8\">\n  <title>Crawl Archive — {}</title>\n  <style>{}</style>\n</head>\n<body>\n<nav id=\"toc\">\n  <h2>Table of Contents</h2>\n  <ol>\n{}  </ol>\n</nav>\n{}</body>\n</html>",
        html_escape_text(seed_url),
        minimal_reading_stylesheet(),
        toc_items,
        articles
    )
}

// =============================================================================
// Folder renders
// =============================================================================

pub fn pages_to_plain_text_folder(pages: &[PageDoc], content: &ExportContent) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let index_content = build_text_index(pages);
    result.push(("index.txt".to_string(), index_content));

    let mut used_slugs: HashSet<String> = HashSet::new();
    for page in pages {
        let slug = unique_slug(&page.url, &mut used_slugs);
        let text = page_to_plain_text(page, content);
        result.push((format!("{}.txt", slug), text));
    }
    result
}

pub fn pages_to_markdown_folder(pages: &[PageDoc], content: &ExportContent) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let index_content = build_markdown_index(pages);
    result.push(("index.md".to_string(), index_content));

    let mut used_slugs: HashSet<String> = HashSet::new();
    for page in pages {
        let slug = unique_slug(&page.url, &mut used_slugs);
        let md = page_to_markdown(page, content);
        result.push((format!("{}.md", slug), md));
    }
    result
}

pub fn pages_to_html_folder(pages: &[PageDoc], content: &ExportContent) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let index_content = build_html_index(pages);
    result.push(("index.html".to_string(), index_content));

    let mut used_slugs: HashSet<String> = HashSet::new();
    for page in pages {
        let slug = unique_slug(&page.url, &mut used_slugs);
        let html = page_to_html(page, content);
        result.push((format!("{}.html", slug), html));
    }
    result
}

fn build_text_index(pages: &[PageDoc]) -> String {
    let mut out = String::new();
    for page in pages {
        let title = get_title(page);
        out.push_str(&format!("{} — {}\n", title, page.url));
    }
    out
}

fn build_markdown_index(pages: &[PageDoc]) -> String {
    let mut out = String::from("# Crawl Archive\n\n");
    let mut used_slugs: HashSet<String> = HashSet::new();
    for page in pages {
        let slug = unique_slug(&page.url, &mut used_slugs);
        let title = get_title(page);
        out.push_str(&format!("- [{}]({}.md) — {}\n", title, slug, page.url));
    }
    out
}

fn build_html_index(pages: &[PageDoc]) -> String {
    let mut links = String::new();
    let mut used_slugs: HashSet<String> = HashSet::new();
    for page in pages {
        let slug = unique_slug(&page.url, &mut used_slugs);
        let title = get_title(page);
        links.push_str(&format!(
            "<tr><td><a href=\"{}.html\">{}</a></td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            slug,
            html_escape_text(title),
            html_escape_text(&page.url),
            html_escape_text(&page.timestamp),
            page.reading_time_minutes.map(|r| r.to_string()).unwrap_or_default()
        ));
    }

    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <meta charset=\"UTF-8\">\n  <title>Crawl Archive Index</title>\n  <style>{}</style>\n</head>\n<body>\n<h1>Crawl Archive</h1>\n<table>\n<tr><th>Title</th><th>URL</th><th>Date</th><th>Reading time</th></tr>\n{}</table>\n</body>\n</html>",
        "table { width: 100%; border-collapse: collapse; } th, td { border: 1px solid #ddd; padding: 8px; text-align: left; } th { background: #f5f5f5; } a { color: #1a6ea8; } body { font-family: Georgia, serif; max-width: 960px; margin: 2rem auto; padding: 0 1rem; } h1 { font-size: 1.5rem; margin-bottom: 1rem; }",
        links
    )
}

// =============================================================================
// EPUB (from Brief A)
// =============================================================================

pub struct EpubChapter {
    pub title: String,
    pub content: String,
    pub url: String,
}

pub fn page_to_epub_chapter(page: &PageDoc) -> EpubChapter {
    let title = page
        .extracted_title
        .as_deref()
        .or_else(|| if page.title.is_empty() { None } else { Some(page.title.as_str()) })
        .unwrap_or("Untitled")
        .to_string();

    let raw_body = if is_extraction_failed(page) {
        format!(
            "<p><em>Content extraction failed for this page. \
             Confidence: {:.2}. \
             <a href=\"{}\">View original</a></em></p>",
            page.extraction_confidence.unwrap_or(0.0),
            html_escape_attr(&page.url)
        )
    } else {
        page.body_html
            .as_deref()
            .or_else(|| if page.content.is_empty() { None } else { Some(page.content.as_str()) })
            .unwrap_or("<p>No content extracted.</p>")
            .to_string()
    };

    let xhtml_body = html_to_xhtml(&raw_body);

    let content = format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<head><title>{}</title></head>\n<body>\n<p class=\"source\">Source: <a href=\"{}\">{}</a></p>\n{}\n</body>\n</html>",
        html_escape_text(&title),
        html_escape_attr(&page.url),
        html_escape_text(&page.url),
        xhtml_body
    );

    EpubChapter {
        title,
        content,
        url: page.url.clone(),
    }
}

pub fn generate_epub(
    chapters: &[EpubChapter],
    book_title: &str,
    cover_image_url: Option<&str>,
    output_path: &std::path::Path,
) -> Result<(), String> {
    let zip_library = epub_builder::ZipLibrary::new()
        .map_err(|e| format!("Failed to create zip library: {}", e))?;

    let mut builder = epub_builder::EpubBuilder::new(zip_library)
        .map_err(|e| format!("Failed to create EPUB builder: {}", e))?;

    builder
        .epub_version(epub_builder::EpubVersion::V30)
        .metadata("title", book_title)
        .map_err(|e| format!("Failed to set title: {}", e))?
        .metadata("author", "Crasp Archive")
        .map_err(|e| format!("Failed to set author: {}", e))?;

    if let Some(cover_url) = cover_image_url {
        let response = reqwest::blocking::Client::new()
            .get(cover_url)
            .timeout(std::time::Duration::from_secs(10))
            .send();
        if let Ok(resp) = response {
            if let Ok(bytes) = resp.bytes() {
                let _ = builder.add_cover_image(
                    "cover.png",
                    &bytes[..],
                    "image/png",
                );
            }
        }
    }

    let css = "body { font-family: Georgia, serif; margin: 1em; line-height: 1.7; }\n\
               h1 { font-size: 1.5em; }\n\
               p.source { font-size: 0.85em; color: #666; margin-bottom: 1.5em; }\n\
               img { max-width: 100%; }\n\
               .extraction-failed { border: 2px dashed #e0a800; background: #fffbec; padding: 1.5rem; border-radius: 4px; color: #7a5c00; margin: 1rem 0; }\n\
               .extraction-failed p { margin: 0.5rem 0; }";
    builder
        .stylesheet(css.as_bytes())
        .map_err(|e| format!("Failed to set stylesheet: {}", e))?;

    for (idx, chapter) in chapters.iter().enumerate() {
        let filename = format!("chapter_{:04}.xhtml", idx);
        let epub_content = epub_builder::EpubContent::new(filename, chapter.content.as_bytes())
            .title(&chapter.title);
        builder
            .add_content(epub_content)
            .map_err(|e| format!("Failed to add chapter {}: {}", idx, e))?;
    }

    let mut output_file = std::fs::File::create(output_path)
        .map_err(|e| format!("Failed to create EPUB file: {}", e))?;

    builder
        .generate(&mut output_file)
        .map_err(|e| format!("Failed to generate EPUB: {}", e))?;

    Ok(())
}

fn html_to_xhtml(html: &str) -> String {
    let no_scripts = strip_scripts_and_styles(html);
    let self_closed = self_close_void_elements(&no_scripts);
    escape_bare_ampersands(&self_closed)
}

fn strip_scripts_and_styles(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let lower = s.to_ascii_lowercase();
    let lower_bytes = lower.as_bytes();
    let src_bytes = s.as_bytes();
    let mut i = 0usize;
    while i < src_bytes.len() {
        if lower_bytes[i..].starts_with(b"<script") {
            if let Some(end) = lower_bytes[i..].windows(b"</script>".len()).position(|w| w == b"</script>") {
                i += end + b"</script>".len();
                continue;
            } else {
                break;
            }
        }
        if lower_bytes[i..].starts_with(b"<style") {
            if let Some(end) = lower_bytes[i..].windows(b"</style>".len()).position(|w| w == b"</style>") {
                i += end + b"</style>".len();
                continue;
            } else {
                break;
            }
        }
        let ch = s[i..].chars().next().unwrap_or('\0');
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

fn self_close_void_elements(s: &str) -> String {
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img",
        "input", "link", "meta", "param", "source", "track", "wbr"
    ];
    let mut result = s.to_string();
    for tag in VOID {
        result = fix_void_element(&result, tag);
    }
    result
}

fn fix_void_element(html: &str, tag: &str) -> String {
    let open_lower = format!("<{}", tag).to_ascii_lowercase();
    let lower: String = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0usize;
    let html_bytes = html.as_bytes();
    let lower_bytes = lower.as_bytes();
    let open_bytes = open_lower.as_bytes();
    while pos < html_bytes.len() {
        if lower_bytes[pos..].starts_with(open_bytes) {
            if let Some(end) = html_bytes[pos..].iter().position(|&b| b == b'>') {
                let tag_slice_end = pos + end + 1;
                let tag_slice = &html[pos..tag_slice_end];
                if !tag_slice.ends_with("/>") {
                    out.push_str(&html[pos..pos + end]);
                    out.push_str(" />");
                } else {
                    out.push_str(tag_slice);
                }
                pos = tag_slice_end;
                continue;
            }
        }
        let ch = html[pos..].chars().next().unwrap_or('\0');
        out.push(ch);
        pos += ch.len_utf8();
    }
    out
}

fn escape_bare_ampersands(s: &str) -> String {
    let s = s
        .replace("&",  "\x00AMP\x00")
        .replace("<",   "\x00LT\x00")
        .replace(">",   "\x00GT\x00")
        .replace("&#",  "\x00HASH\x00");

    let s = s.replace('&', "&");

    s.replace("\x00AMP\x00",  "&")
     .replace("\x00LT\x00",   "<")
     .replace("\x00GT\x00",   ">")
     .replace("\x00HASH\x00", "&#")
}

// =============================================================================
// Existing Brief A functions (kept, extended with content variants)
// =============================================================================

pub fn page_doc_to_markdown(page: &PageDoc) -> String {
    let mut out = String::with_capacity(8192);

    let title = get_title(page);

    out.push_str(&format!("# {}\n\n", title));

    out.push_str(&format!("**URL:** {}\n", page.url));
    out.push_str(&format!("**Archived:** {}\n", page.timestamp));
    if let Some(ref author) = page.author {
        out.push_str(&format!("**Author:** {}\n", author));
    }
    if let Some(ref published) = page.published_date {
        out.push_str(&format!("**Published:** {}\n", published));
    }
    if let Some(rt) = page.reading_time_minutes {
        if rt > 0 {
            out.push_str(&format!("**Reading time:** {} min\n", rt));
        }
    }

    out.push_str("\n---\n\n");

    if is_extraction_failed(page) {
        out.push_str(&extraction_failed_markdown(page));
    } else {
        let body = get_body_text(page);
        out.push_str(body);
    }
    out.push_str("\n\n---\n");

    out
}

pub fn page_doc_to_html(page: &PageDoc) -> String {
    let mut out = String::with_capacity(16384);

    let title = get_title(page);

    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    out.push_str("  <meta charset=\"UTF-8\">\n");
    out.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!("  <title>{}</title>\n", html_escape_text(title)));
    if let Some(ref author) = page.author {
        out.push_str(&format!(
            "  <meta name=\"author\" content=\"{}\">\n",
            html_escape_text(author)
        ));
    }
    if let Some(ref published) = page.published_date {
        out.push_str(&format!(
            "  <meta name=\"date\" content=\"{}\">\n",
            html_escape_text(published)
        ));
    }
    out.push_str(&format!(
        "  <meta name=\"source\" content=\"{}\">\n",
        html_escape_text(&page.url)
    ));
    out.push_str(&format!(
        "  <meta name=\"archived\" content=\"{}\">\n",
        html_escape_text(&page.timestamp)
    ));

    out.push_str("  <style>\n");
    out.push_str("    body { max-width: 720px; margin: 2rem auto; font-family: Georgia, serif;\n");
    out.push_str("           font-size: 1.1rem; line-height: 1.7; color: #1a1a1a; padding: 0 1rem; }\n");
    out.push_str("    h1 { font-size: 1.8rem; line-height: 1.3; margin-bottom: 0.5rem; }\n");
    out.push_str("    .meta { margin-bottom: 2rem; border-bottom: 1px solid #eee; padding-bottom: 1rem; }\n");
    out.push_str("    img { max-width: 100%; height: auto; }\n");
    out.push_str("    a { color: #1a6ea8; }\n");
    out.push_str("    blockquote { border-left: 3px solid #ccc; margin: 0; padding-left: 1rem; color: #555; }\n");
    out.push_str("    pre { background: #f5f5f5; padding: 1rem; overflow-x: auto; }\n");
    out.push_str("    figure { margin: 1.5rem 0; }\n");
    out.push_str("    figcaption { font-size: 0.85rem; color: #666; text-align: center; margin-top: 0.5rem; }\n");
    out.push_str("    .extraction-failed { border: 2px dashed #e0a800; background: #fffbec; padding: 1.5rem; border-radius: 4px; color: #7a5c00; margin: 1rem 0; }\n");
    out.push_str("    .extraction-failed p { margin: 0.5rem 0; }\n");
    out.push_str("  </style>\n");
    out.push_str("</head>\n<body>\n");

    out.push_str(&format!("  <h1>{}</h1>\n", html_escape_text(title)));

    out.push_str("  <div class=\"meta\">\n");
    out.push_str(&format!(
        "    <div>Source: <a href=\"{}\">{}</a></div>\n",
        html_escape_attr(&page.url),
        html_escape_text(&page.url)
    ));
    out.push_str(&format!(
        "    <div>Archived: {}</div>\n",
        html_escape_text(&page.timestamp)
    ));
    if let Some(ref author) = page.author {
        out.push_str(&format!(
            "    <div>Author: {}</div>\n",
            html_escape_text(author)
        ));
    }
    if let Some(ref published) = page.published_date {
        out.push_str(&format!(
            "    <div>Published: {}</div>\n",
            html_escape_text(published)
        ));
    }
    out.push_str("  </div>\n");

    let body_html = if is_extraction_failed(page) {
        extraction_failed_html(page)
    } else {
        get_body_html_raw(page).to_string()
    };
    out.push_str(&body_html);
    out.push_str("\n</body>\n</html>\n");

    out
}

// =============================================================================
// Helpers
// =============================================================================

fn get_title(page: &PageDoc) -> &str {
    page
        .extracted_title
        .as_deref()
        .or_else(|| if page.title.is_empty() { None } else { Some(page.title.as_str()) })
        .unwrap_or("Untitled")
}

fn is_extraction_failed(page: &PageDoc) -> bool {
    if page.extraction_method.as_deref() == Some("failed") {
        return true;
    }
    let body_html_empty = page.body_html.as_deref().unwrap_or("").trim().is_empty();
    let body_text_empty = page.body_text.as_deref().unwrap_or("").trim().is_empty();
    body_html_empty && body_text_empty
}

fn extraction_failed_html(page: &PageDoc) -> String {
    let confidence = page.extraction_confidence.unwrap_or(0.0);
    format!(
        "<div class=\"extraction-failed\">\n\
         <p><strong>&#9888; Content extraction failed for this page.</strong></p>\n\
         <p>The page may use JavaScript rendering, or its structure could not be \
         parsed by the readability or CSS-selector extraction methods.</p>\n\
         <p>Confidence score: {:.2}</p>\n\
         <p>Original URL: <a href=\"{}\">{}</a></p>\n\
         </div>",
        confidence,
        html_escape_attr(&page.url),
        html_escape_text(&page.url)
    )
}

fn extraction_failed_markdown(page: &PageDoc) -> String {
    let confidence = page.extraction_confidence.unwrap_or(0.0);
    format!(
        "> \u{26a0} Content extraction failed for this page.\n\
         > The page may use JavaScript rendering.\n\
         > Confidence: {:.2}\n\
         > Original URL: {}",
        confidence,
        page.url
    )
}

fn extraction_failed_plain(page: &PageDoc) -> String {
    let confidence = page.extraction_confidence.unwrap_or(0.0);
    format!(
        "[EXTRACTION FAILED]\n\
         This page could not be parsed by the readability or CSS-selector methods.\n\
         Confidence: {:.2}\n\
         Original URL: {}",
        confidence,
        page.url
    )
}

fn get_body_text(page: &PageDoc) -> &str {
    page.body_text
        .as_deref()
        .unwrap_or_else(|| if page.content.is_empty() { "No content extracted." } else { &page.content })
}

fn get_body_html(page: &PageDoc) -> String {
    if is_extraction_failed(page) {
        return extraction_failed_html(page);
    }
    page.body_html
        .as_deref()
        .unwrap_or_else(|| if page.content.is_empty() { "" } else { &page.content })
        .to_string()
}

fn get_body_html_raw(page: &PageDoc) -> &str {
    page.body_html
        .as_deref()
        .unwrap_or_else(|| if page.content.is_empty() { "" } else { &page.content })
}

fn html_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            _ => out.push(c),
        }
    }
    out
}

fn html_escape_attr(s: &str) -> String {
    html_escape_text(s)
}

fn unique_slug(url_str: &str, used_slugs: &mut HashSet<String>) -> String {
    let parsed = url::Url::parse(url_str).ok();
    let path = parsed.as_ref().map(|u| u.path()).unwrap_or(url_str);
    let mut slug = path
        .replace('/', "_")
        .replace('?', "")
        .replace('#', "");

    slug = slug
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();

    if slug.is_empty() {
        slug = "index".to_string();
    }

    if slug.len() > 60 {
        slug = slug.chars().take(60).collect();
    }

    let mut unique = slug.clone();
    let mut counter = 2;
    while used_slugs.contains(&unique) {
        unique = format!("{}_{}", slug, counter);
        counter += 1;
    }
    used_slugs.insert(unique.clone());
    unique
}
