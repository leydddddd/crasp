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
        ExportContent::ContentOnly => get_body_text(page).to_string(),
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
    "body { max-width: 720px; margin: 2rem auto; font-family: Georgia, serif; font-size: 1.1rem; line-height: 1.7; color: #1a1a1a; padding: 0 1rem; } img { max-width: 100%; height: auto; } a { color: #1a6ea8; } blockquote { border-left: 3px solid #ccc; margin: 0; padding-left: 1rem; color: #555; } pre { background: #f5f5f5; padding: 1rem; overflow-x: auto; } figure { margin: 1.5rem 0; } figcaption { font-size: 0.85rem; color: #666; text-align: center; margin-top: 0.5rem; }"
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

    let body = page
        .body_html
        .as_deref()
        .or_else(|| if page.content.is_empty() { None } else { Some(page.content.as_str()) })
        .unwrap_or("<p>No content extracted.</p>");

    let xhtml_body = html_to_xhtml(body);

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
               img { max-width: 100%; }";
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
    let mut result = html.replace("<br>", "<br />");
    result = result.replace("<hr>", "<hr />");

    let mut img_fixed = String::with_capacity(result.len());
    let mut i = 0;
    let bytes = result.as_bytes();
    while i < bytes.len() {
        if i + 4 <= bytes.len() && &bytes[i..i+4] == b"<img" {
            let tag_start = i;
            let mut j = i + 4;
            let mut found_close = false;
            while j < bytes.len() {
                if bytes[j] == b'>' {
                    if j > 0 && bytes[j-1] == b'/' {
                        found_close = true;
                    } else {
                        img_fixed.push_str(&result[tag_start..j]);
                        img_fixed.push_str(" /");
                        found_close = true;
                    }
                    if found_close {
                        img_fixed.push('>');
                        i = j + 1;
                    }
                    break;
                }
                j += 1;
            }
            if !found_close {
                img_fixed.push_str(&result[tag_start..]);
                break;
            }
        } else {
            img_fixed.push(bytes[i] as char);
            i += 1;
        }
    }

    encode_bare_ampersands(&img_fixed)
}

fn encode_bare_ampersands(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' {
            let is_entity = if i + 1 < chars.len() && chars[i + 1] == '#' {
                true
            } else if i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic() {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-') {
                    if chars[j] == ';' {
                        break;
                    }
                    j += 1;
                }
                j < chars.len() && chars[j] == ';'
            } else {
                false
            };
            if is_entity {
                result.push('&');
            } else {
                result.push_str("&");
            }
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    result
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

    let body = get_body_text(page);
    out.push_str(body);
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
    out.push_str("    ..ensurewithinull margin-bottom: 2rem; border-bottom: 1px solid #eee; padding-bottom: 1rem; }\n");
    out.push_str("    img { max-width: 100%; height: auto; }\n");
    out.push_str("    a { color: #1a6ea8; }\n");
    out.push_str("    blockquote { border-left: 3px solid #ccc; margin: 0; padding-left: 1rem; color: #555; }\n");
    out.push_str("    pre { background: #f5f5f5; padding: 1rem; overflow-x: auto; }\n");
    out.push_str("    figure { margin: 1.5rem 0; }\n");
    out.push_str("    figcaption { font-size: 0.85rem; color: #666; text-align: center; margin-top: 0.5rem; }\n");
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

    let body_html = page
        .body_html
        .as_deref()
        .unwrap_or_else(|| if page.content.is_empty() { "" } else { &page.content });
    out.push_str(body_html);
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

fn get_body_text(page: &PageDoc) -> &str {
    page.body_text
        .as_deref()
        .unwrap_or_else(|| if page.content.is_empty() { "No content extracted." } else { &page.content })
}

fn get_body_html(page: &PageDoc) -> &str {
    page.body_html
        .as_deref()
        .unwrap_or_else(|| if page.content.is_empty() { "" } else { &page.content })
}

fn html_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&"),
            '<' => out.push_str("<"),
            '>' => out.push_str(">"),
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
