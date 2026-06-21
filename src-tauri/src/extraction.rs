use std::io::Cursor;

use scraper::{Html, Selector};
use url::Url;

use crate::schema::{AssetDocument, AssetImage, AssetVideo, PageAssets};

pub struct ExtractionResult {
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_date: Option<String>,
    pub excerpt: Option<String>,
    pub body_html: String,
    pub body_text: String,
    pub reading_time_minutes: u32,
    pub confidence: f32,
    pub thin_content: bool,
    pub extraction_failed: bool,
    method: String,
}

impl ExtractionResult {
    pub fn extraction_method(&self) -> &str {
        &self.method
    }

    pub fn raw_fallback() -> Self {
        ExtractionResult {
            title: None,
            author: None,
            published_date: None,
            excerpt: None,
            body_html: String::new(),
            body_text: String::new(),
            reading_time_minutes: 0,
            confidence: 0.0,
            thin_content: true,
            extraction_failed: true,
            method: "raw".to_string(),
        }
    }
}

pub fn extract_main_content(html: &str, base_url: &str) -> ExtractionResult {
    let full_text = extract_all_text(html);

    if let Some(product) = try_readability(html, base_url) {
        let confidence = calculate_confidence(&product.content, &product.text, &full_text);

        if confidence >= 0.40 {
            let non_ws_len = product.text.chars().filter(|c| !c.is_whitespace()).count();
            let thin_content = non_ws_len < 200;
            let word_count = product.text.split_whitespace().count();
            let reading_time_minutes = if word_count == 0 {
                0
            } else {
                ((word_count as f32 / 200.0).ceil() as u32).max(1)
            };
            let author = extract_author(html);
            let published_date = extract_published_date(html);
            let excerpt = extract_excerpt(html);

            return ExtractionResult {
                title: Some(product.title),
                author,
                published_date,
                excerpt,
                body_html: product.content,
                body_text: product.text,
                reading_time_minutes,
                confidence,
                thin_content,
                extraction_failed: false,
                method: "readability".to_string(),
            };
        }
    }

    let css = try_css_selector_extraction(html);
    let css_confidence = calculate_confidence(&css.body_html, &css.body_text, &full_text);

    if css_confidence >= 0.25 {
        let non_ws_len = css.body_text.chars().filter(|c| !c.is_whitespace()).count();
        let thin_content = non_ws_len < 200;
        let word_count = css.body_text.split_whitespace().count();
        let reading_time_minutes = if word_count == 0 {
            0
        } else {
            ((word_count as f32 / 200.0).ceil() as u32).max(1)
        };
        let author = extract_author(html);
        let published_date = extract_published_date(html);
        let excerpt = extract_excerpt(html);

        return ExtractionResult {
            title: css.title,
            author,
            published_date,
            excerpt,
            body_html: css.body_html,
            body_text: css.body_text,
            reading_time_minutes,
            confidence: css_confidence,
            thin_content,
            extraction_failed: false,
            method: "css_selector".to_string(),
        };
    }

    ExtractionResult {
        title: extract_title_tag(html),
        author: None,
        published_date: None,
        excerpt: None,
        body_html: String::new(),
        body_text: String::new(),
        reading_time_minutes: 0,
        confidence: 0.0,
        thin_content: true,
        extraction_failed: true,
        method: "failed".to_string(),
    }
}

fn text_to_tag_ratio(html: &str) -> f32 {
    let mut in_tag = false;
    let mut text_chars = 0usize;
    let mut tag_chars = 0usize;
    for c in html.chars() {
        match c {
            '<' => { in_tag = true; tag_chars += 1; }
            '>' => { in_tag = false; tag_chars += 1; }
            _ => if in_tag { tag_chars += 1; } else { text_chars += 1; }
        }
    }
    let total = text_chars + tag_chars;
    if total == 0 { return 0.0; }
    text_chars as f32 / total as f32
}

fn calculate_confidence(body_html: &str, body_text: &str, full_page_text: &str) -> f32 {
    let ratio = text_to_tag_ratio(body_html);
    let para_count = body_html.matches("<p").count();
    let word_ratio = if full_page_text.is_empty() { 0.0 } else {
        let extracted_words = body_text.split_whitespace().count();
        let total_words = full_page_text.split_whitespace().count();
        if total_words == 0 { 0.0 } else { extracted_words as f32 / total_words as f32 }
    };

    if ratio < 0.35 || para_count < 3 || word_ratio < 0.10 {
        let failed = [ratio < 0.35, para_count < 3, word_ratio < 0.10]
            .iter().filter(|&&x| x).count();
        0.5 - (failed as f32 * 0.15)
    } else {
        (ratio * 0.4 + word_ratio * 0.4 + (para_count.min(10) as f32 / 10.0) * 0.2)
            .min(1.0)
    }
}

struct CssExtraction {
    title: Option<String>,
    body_html: String,
    body_text: String,
}

fn try_css_selector_extraction(html: &str) -> CssExtraction {
    let document = Html::parse_document(html);
    let title = extract_title_tag(html);

    let selectors_order: &[&str] = &[
        "article",
        "main",
        "[role=\"main\"]",
        ".content",
        ".post",
    ];

    for selector_str in selectors_order {
        if let Ok(sel) = Selector::parse(selector_str) {
            let elements: Vec<_> = document.select(&sel).collect();
            if elements.is_empty() {
                continue;
            }
            use crate::crawler::sanitize_to_html;
            let html_content = sanitize_to_html(&elements);
            if !html_content.trim().is_empty() {
                let text_content: String = elements.iter().flat_map(|e| e.text()).collect();
                if !text_content.trim().is_empty() {
                    return CssExtraction {
                        title,
                        body_html: html_content,
                        body_text: text_content,
                    };
                }
            }
        }
    }

    CssExtraction {
        title,
        body_html: String::new(),
        body_text: String::new(),
    }
}

fn extract_all_text(html: &str) -> String {
    let document = Html::parse_document(html);
    use crate::crawler::{sanitize_to_text, select_content};
    let matched = select_content(&document, &["body".to_string()]);
    if matched.is_empty() {
        return String::new();
    }
    sanitize_to_text(&matched)
}

fn extract_title_tag(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    if let Ok(sel) = Selector::parse("title") {
        if let Some(el) = document.select(&sel).next() {
            let text: String = el.text().collect();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn try_readability(html: &str, base_url: &str) -> Option<readability::extractor::Product> {
    let parsed_url = Url::parse(base_url).ok()?;
    let mut input = Cursor::new(html.as_bytes());
    readability::extractor::extract(&mut input, &parsed_url).ok()
}

pub fn extract_published_date(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    if let Ok(sel) = Selector::parse("meta[property=\"article:published_time\"]") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                if let Some(iso) = normalize_date(content.trim()) {
                    return Some(iso);
                }
            }
        }
    }

    if let Ok(sel) = Selector::parse("meta[name=\"pubdate\"]") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                if let Some(iso) = normalize_date(content.trim()) {
                    return Some(iso);
                }
            }
        }
    }

    if let Ok(sel) = Selector::parse("time[datetime]") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(dt) = el.value().attr("datetime") {
                if let Some(iso) = normalize_date(dt.trim()) {
                    return Some(iso);
                }
            }
        }
    }

    if let Some(iso) = extract_ld_json_date(html) {
        return Some(iso);
    }

    None
}

fn extract_author(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    if let Ok(sel) = Selector::parse("meta[name=\"author\"]") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    if let Ok(sel) = Selector::parse("meta[property=\"article:author\"]") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}

fn extract_excerpt(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    if let Ok(sel) = Selector::parse("meta[property=\"og:description\"]") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    if let Ok(sel) = Selector::parse("meta[name=\"description\"]") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}

fn extract_ld_json_date(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let sel = Selector::parse("script[type=\"application/ld+json\"]").ok()?;

    for el in document.select(&sel) {
        let text: String = el.text().collect();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(date_str) = val.get("datePublished").and_then(|v| v.as_str()) {
                if let Some(iso) = normalize_date(date_str.trim()) {
                    return Some(iso);
                }
            }
            if let Some(obj) = val.as_object() {
                for (_key, nested) in obj {
                    if let Some(nested_obj) = nested.as_object() {
                        if let Some(date_str) =
                            nested_obj.get("datePublished").and_then(|v| v.as_str())
                        {
                            if let Some(iso) = normalize_date(date_str.trim()) {
                                return Some(iso);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn normalize_date(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.to_rfc3339());
    }

    let date_only = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Some(format!("{}T00:00:00Z", date_only))
}

pub fn extract_assets(html: &str, base_url: &str, main_content_html: &str) -> PageAssets {
    let base = Url::parse(base_url).ok();
    let full_doc = Html::parse_document(html);
    let main_doc = Html::parse_document(main_content_html);

    let og_image = extract_og(&full_doc, "og:image");
    let og_description = extract_og(&full_doc, "og:description");
    let og_published_time = extract_og(&full_doc, "article:published_time");

    let images = extract_images(&full_doc, &main_doc, &base);
    let videos = extract_videos(&full_doc, &main_doc, &base);
    let documents = extract_documents(&full_doc, &main_doc, &base);

    PageAssets {
        images,
        videos,
        documents,
        og_image,
        og_description,
        og_published_time,
    }
}

fn extract_og(document: &Html, property: &str) -> Option<String> {
    let sel = Selector::parse(&format!("meta[property=\"{}\"]", property)).ok()?;
    let el = document.select(&sel).next()?;
    let content = el.value().attr("content")?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_images(
    full_doc: &Html,
    main_doc: &Html,
    base: &Option<Url>,
) -> Vec<AssetImage> {
    let mut images = Vec::new();
    let Ok(sel) = Selector::parse("img[src]") else {
        return images;
    };

    let main_srcs: std::collections::HashSet<String> = main_doc
        .select(&sel)
        .filter_map(|el| el.value().attr("src").map(String::from))
        .collect();

    for el in full_doc.select(&sel) {
        let Some(src_raw) = el.value().attr("src") else {
            continue;
        };

        if src_raw.starts_with("data:") {
            continue;
        }

        let width_attr = el.value().attr("width").and_then(|w| w.parse::<u32>().ok());
        let height_attr = el.value().attr("height").and_then(|h| h.parse::<u32>().ok());
        if width_attr == Some(1) && height_attr == Some(1) {
            continue;
        }

        let src = resolve_url(base, src_raw);
        let alt = el.value().attr("alt").map(|s| s.to_string());
        let in_main_content = main_srcs.contains(src_raw);

        let caption = find_adjacent_figcaption(el);

        images.push(AssetImage {
            src,
            alt,
            caption,
            in_main_content,
            width: width_attr,
            height: height_attr,
        });
    }

    images
}

fn find_adjacent_figcaption(img_el: scraper::ElementRef) -> Option<String> {
    for ancestor_ref in img_el.ancestors() {
        match ancestor_ref.value() {
            scraper::Node::Element(el) => {
                if el.name.local.as_ref() == "figure" {
                    if let Some(wrapped) = scraper::ElementRef::wrap(ancestor_ref) {
                        let Ok(sel) = Selector::parse("figcaption") else {
                            return None;
                        };
                        if let Some(caption_el) = wrapped.select(&sel).next() {
                            let text: String = caption_el.text().collect::<Vec<_>>().join("");
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                return Some(trimmed.to_string());
                            }
                        }
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_videos(
    full_doc: &Html,
    main_doc: &Html,
    base: &Option<Url>,
) -> Vec<AssetVideo> {
    let mut videos = Vec::new();
    let mut main_video_srcs = std::collections::HashSet::new();

    if let Ok(sel) = Selector::parse("video[src], video source[src], iframe[src]") {
        for el in main_doc.select(&sel) {
            if let Some(src) = el.value().attr("src") {
                main_video_srcs.insert(src.to_string());
            }
        }
    }

    if let Ok(sel) = Selector::parse("video[src]") {
        for el in full_doc.select(&sel) {
            if let Some(src_raw) = el.value().attr("src") {
                let src = resolve_url(base, src_raw);
                let in_main_content = main_video_srcs.contains(src_raw);
                videos.push(AssetVideo {
                    src,
                    kind: "direct".to_string(),
                    video_id: None,
                    in_main_content,
                });
            }
        }
    }

    if let Ok(sel) = Selector::parse("video source[src]") {
        for el in full_doc.select(&sel) {
            if let Some(src_raw) = el.value().attr("src") {
                let src = resolve_url(base, src_raw);
                let in_main_content = main_video_srcs.contains(src_raw);
                videos.push(AssetVideo {
                    src,
                    kind: "direct".to_string(),
                    video_id: None,
                    in_main_content,
                });
            }
        }
    }

    if let Ok(sel) = Selector::parse("iframe[src]") {
        for el in full_doc.select(&sel) {
            if let Some(src_raw) = el.value().attr("src") {
                if src_raw.contains("youtube") || src_raw.contains("vimeo") {
                    let (kind, video_id) = classify_embed(src_raw);
                    let src = resolve_url(base, src_raw);
                    let in_main_content = main_video_srcs.contains(src_raw);
                    videos.push(AssetVideo {
                        src,
                        kind,
                        video_id,
                        in_main_content,
                    });
                }
            }
        }
    }

    videos
}

fn classify_embed(src: &str) -> (String, Option<String>) {
    if src.contains("youtube.com/embed/") || src.contains("youtube-nocookie.com/embed/") {
        let id = extract_youtube_id(src);
        return ("youtube".to_string(), id);
    }
    if src.contains("player.vimeo.com/") {
        let id = extract_vimeo_id(src);
        return ("vimeo".to_string(), id);
    }
    ("embed".to_string(), None)
}

fn extract_youtube_id(src: &str) -> Option<String> {
    let parsed = Url::parse(src).ok()?;
    let path = parsed.path();
    let segment = path.rsplit('/').next()?;
    if segment.is_empty() {
        None
    } else {
        Some(segment.to_string())
    }
}

fn extract_vimeo_id(src: &str) -> Option<String> {
    let parsed = Url::parse(src).ok()?;
    let path = parsed.path();
    let segment = path.rsplit('/').find(|s| !s.is_empty())?;
    Some(segment.to_string())
}

fn extract_documents(
    full_doc: &Html,
    main_doc: &Html,
    base: &Option<Url>,
) -> Vec<AssetDocument> {
    let mut documents = Vec::new();
    let Ok(sel) = Selector::parse("a[href]") else {
        return documents;
    };

    let doc_exts = [
        ".pdf", ".epub", ".docx", ".xlsx", ".pptx", ".mp3", ".mp4", ".zip",
    ];

    let main_href_set: std::collections::HashSet<String> = main_doc
        .select(&sel)
        .filter_map(|el| el.value().attr("href").map(String::from))
        .collect();

    for el in full_doc.select(&sel) {
        let Some(href_raw) = el.value().attr("href") else {
            continue;
        };

        let href_lower = href_raw.to_lowercase();
        let ext_matches = doc_exts.iter().any(|ext| href_lower.ends_with(ext));
        if !ext_matches {
            continue;
        }

        let src = resolve_url(base, href_raw);
        let link_text: String = el.text().collect::<Vec<_>>().join("");
        let link_text = if link_text.trim().is_empty() {
            None
        } else {
            Some(link_text.trim().to_string())
        };

        let mime_type = guess_mime_from_url(&src);
        let in_main_content = main_href_set.contains(href_raw);

        documents.push(AssetDocument {
            src,
            link_text,
            mime_type,
            in_main_content,
        });
    }

    documents
}

fn guess_mime_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let path = parsed.path();
    let ext = path.rsplit('.').next()?;
    mime_guess::from_ext(ext).first().map(|m| m.to_string())
}

fn resolve_url(base: &Option<Url>, href: &str) -> String {
    if let Some(b) = base {
        b.join(href).map(|u| u.to_string()).unwrap_or_else(|_| href.to_string())
    } else {
        href.to_string()
    }
}
