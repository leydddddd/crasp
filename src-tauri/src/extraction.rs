use std::io::Cursor;

use scraper::{Html, Selector};
use url::Url;

use crate::schema::{AssetDocument, AssetImage, AssetVideo, PageAssets};

#[derive(Debug, Clone, PartialEq)]
pub enum PageType {
    Article,
    SpaApplication,
    EcommerceProduct,
    NavigationIndex,
    MediaGallery,
    Unknown,
}

impl PageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PageType::Article => "article",
            PageType::SpaApplication => "spa_application",
            PageType::EcommerceProduct => "ecommerce_product",
            PageType::NavigationIndex => "navigation_index",
            PageType::MediaGallery => "media_gallery",
            PageType::Unknown => "unknown",
        }
    }
}

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
    pub method: String,
    pub page_type: Option<String>,
}

pub struct ZyteExtractionResult {
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_date: Option<String>,
    pub excerpt: Option<String>,
    pub body_html: String,
    pub body_text: String,
    pub reading_time_minutes: u32,
    pub confidence: f32,
    pub method: String,
    pub thin_content: bool,
    pub page_type: Option<String>,
}

pub struct StructuredData {
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub date_published: Option<String>,
    pub body_text: Option<String>,
    pub image_url: Option<String>,
    pub page_type: Option<String>,
}

impl StructuredData {
    fn empty() -> Self {
        Self {
            title: None,
            description: None,
            author: None,
            date_published: None,
            body_text: None,
            image_url: None,
            page_type: None,
        }
    }
}

struct PageSignals {
    og_type: Option<String>,
    jsonld_types: Vec<String>,
    has_article_tag: bool,
    has_main_tag: bool,
    p_count: usize,
    p_to_div_ratio: f32,
    text_ratio: f32,
    svg_ratio: f32,
    interactive_density: f32,
    img_to_p_ratio: f32,
    link_density: f32,
}

fn compute_signals(html: &str) -> PageSignals {
    let og_type = extract_meta_property(html, "og:type");
    let jsonld_types = extract_jsonld_types(html);
    let p_count = html.matches("<p").count();
    let div_count = html.matches("<div").count();
    let p_to_div_ratio = p_count as f32 / (div_count + 1) as f32;
    let has_article_tag = html.contains("<article");
    let has_main_tag = html.contains("<main")
        || html.contains("role=\"main\"")
        || html.contains("role='main'");

    let total_html_chars = html.len();
    let text_ratio = if total_html_chars == 0 {
        0.0
    } else {
        count_text_chars(html) as f32 / total_html_chars as f32
    };

    let svg_ratio = if total_html_chars == 0 {
        0.0
    } else {
        count_svg_chars(html) as f32 / total_html_chars as f32
    };

    let interactive_density =
        (html.matches("<button").count() + html.matches("<input").count()) as f32
            / (p_count + 1) as f32;

    let img_to_p_ratio = html.matches("<img").count() as f32 / (p_count + 1) as f32;

    let href_count = html.matches("href=").count();
    let link_density = href_count as f32 / (href_count + p_count + 1) as f32;

    PageSignals {
        og_type,
        jsonld_types,
        has_article_tag,
        has_main_tag,
        p_count,
        p_to_div_ratio,
        text_ratio,
        svg_ratio,
        interactive_density,
        img_to_p_ratio,
        link_density,
    }
}

fn extract_meta_property(html: &str, property: &str) -> Option<String> {
    let patterns = [
        format!("property=\"{}\"", property),
        format!("property='{}'", property),
    ];

    for pat in &patterns {
        if let Some(idx) = html.find(pat) {
            let slice = &html[idx..];
            if let Some(content_idx) = slice.find("content=") {
                let content_slice = &slice[content_idx + "content=".len()..];
                if let Some(value) = extract_quoted_value(content_slice) {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }
    None
}

fn extract_jsonld_types(html: &str) -> Vec<String> {
    let mut types = Vec::new();
    for block in extract_jsonld_blocks(html) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&block) {
            collect_jsonld_types(&val, &mut types);
        }
    }
    types
}

fn collect_jsonld_types(val: &serde_json::Value, types: &mut Vec<String>) {
    match val {
        serde_json::Value::Object(map) => {
            if let Some(t) = map.get("@type") {
                match t {
                    serde_json::Value::String(s) => types.push(s.to_string()),
                    serde_json::Value::Array(arr) => {
                        for entry in arr {
                            if let Some(s) = entry.as_str() {
                                types.push(s.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for (_key, value) in map {
                collect_jsonld_types(value, types);
            }
        }
        serde_json::Value::Array(arr) => {
            for entry in arr {
                collect_jsonld_types(entry, types);
            }
        }
        _ => {}
    }
}

fn extract_quoted_value(input: &str) -> Option<String> {
    let mut chars = input.chars();
    let quote = match chars.next() {
        Some('"') => '"',
        Some('\'') => '\'',
        _ => return None,
    };
    let mut value = String::new();
    for c in chars {
        if c == quote {
            break;
        }
        value.push(c);
    }
    Some(value)
}

fn count_text_chars(html: &str) -> usize {
    let mut in_tag = false;
    let mut count = 0usize;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ => if !in_tag { count += 1; },
        }
    }
    count
}

fn count_svg_chars(html: &str) -> usize {
    let mut total = 0usize;
    let mut start_idx = 0usize;

    while let Some(open_idx) = html[start_idx..].find("<svg") {
        let abs_open = start_idx + open_idx;
        if let Some(tag_end_idx) = html[abs_open..].find('>') {
            let content_start = abs_open + tag_end_idx + 1;
            if let Some(close_idx) = html[content_start..].find("</svg>") {
                let content_end = content_start + close_idx;
                total += html[content_start..content_end].len();
                start_idx = content_end + "</svg>".len();
                continue;
            }
        }
        break;
    }

    total
}

fn classify_page(html: &str) -> PageType {
    let signals = compute_signals(html);

    const LINK_DENSITY_NAV_MIN: f32 = 0.97;
    const P_COUNT_NAV_MAX: usize = 12;
    let is_strong_nav = signals.link_density >= LINK_DENSITY_NAV_MIN
        && signals.p_count < P_COUNT_NAV_MAX
        && !signals.has_article_tag;

    for jsonld_type in &signals.jsonld_types {
        match jsonld_type.to_lowercase().as_str() {
            "article" | "newsarticle" | "blogposting" | "technicalarticle" => {
                if is_strong_nav {
                    return PageType::NavigationIndex;
                }
                return PageType::Article;
            }
            "product" => return PageType::EcommerceProduct,
            "website" | "webpage" => {}
            _ => {}
        }
    }

    if let Some(ref og) = signals.og_type {
        match og.as_str() {
            "article" => {
                if is_strong_nav {
                    return PageType::NavigationIndex;
                }
                return PageType::Article;
            }
            "product" => return PageType::EcommerceProduct,
            _ => {}
        }
    }

    const P_COUNT_ARTICLE_MIN: usize = 24;
    const P_TO_DIV_ARTICLE_MIN: f32 = 0.08;
    const INTERACTIVE_DENSITY_MAX: f32 = 1.0;
    const LINK_DENSITY_MAX: f32 = 0.95;
    const TEXT_RATIO_ARTICLE_MAX: f32 = 0.60;

    if signals.p_count >= P_COUNT_ARTICLE_MIN
        && signals.p_to_div_ratio >= P_TO_DIV_ARTICLE_MIN
        && signals.interactive_density < INTERACTIVE_DENSITY_MAX
        && signals.link_density < LINK_DENSITY_MAX
        && signals.text_ratio <= TEXT_RATIO_ARTICLE_MAX
        && (signals.has_article_tag || signals.has_main_tag)
    {
        return PageType::Article;
    }

    if signals.link_density >= LINK_DENSITY_NAV_MIN
        && signals.p_count < P_COUNT_NAV_MAX
        && !signals.has_article_tag
    {
        return PageType::NavigationIndex;
    }

    const TEXT_RATIO_SPA_MAX: f32 = 0.20;
    const INTERACTIVE_DENSITY_SPA_MIN: f32 = 0.20;
    const SVG_RATIO_SPA_MIN: f32 = 0.007;

    if signals.text_ratio < TEXT_RATIO_SPA_MAX
        || signals.interactive_density >= INTERACTIVE_DENSITY_SPA_MIN
        || signals.svg_ratio >= SVG_RATIO_SPA_MIN
    {
        return PageType::SpaApplication;
    }

    const IMG_TO_P_GALLERY_MIN: f32 = 2.5;
    const P_COUNT_GALLERY_MAX: usize = 12;

    if signals.img_to_p_ratio >= IMG_TO_P_GALLERY_MIN
        && signals.p_count < P_COUNT_GALLERY_MAX
    {
        return PageType::MediaGallery;
    }

    let has_price_pattern = html.contains("itemprop=\"price\"")
        || html.contains("class=\"price\"")
        || html.contains("data-price=");
    if has_price_pattern {
        return PageType::EcommerceProduct;
    }

    PageType::Unknown
}

fn extract_structured_data(html: &str) -> StructuredData {
    let mut data = StructuredData::empty();

    for block in extract_jsonld_blocks(html) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&block) {
            merge_structured_data_from_jsonld(&val, &mut data);
        }
    }

    let og_title = extract_meta_property(html, "og:title");
    let og_description = extract_meta_property(html, "og:description");
    let og_image = extract_meta_property(html, "og:image");
    let og_type = extract_meta_property(html, "og:type");
    let og_author = extract_meta_property(html, "article:author");
    let og_published = extract_meta_property(html, "article:published_time");

    let meta_description = extract_meta_name(html, "description");
    let meta_author = extract_meta_name(html, "author");

    if data.title.is_none() {
        data.title = og_title;
    }
    if data.description.is_none() {
        data.description = og_description.or(meta_description);
    }
    if data.author.is_none() {
        data.author = og_author.or(meta_author);
    }
    if data.date_published.is_none() {
        data.date_published = og_published;
    }
    if data.image_url.is_none() {
        data.image_url = og_image;
    }
    if data.page_type.is_none() {
        data.page_type = og_type;
    }

    data
}

fn extract_jsonld_blocks(html: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut start_idx = 0usize;

    while let Some(script_idx) = html[start_idx..].find("<script") {
        let abs_script = start_idx + script_idx;
        let Some(tag_end_idx) = html[abs_script..].find('>') else {
            break;
        };
        let tag_end = abs_script + tag_end_idx + 1;
        let tag_head = &html[abs_script..tag_end];
        if tag_head.contains("application/ld+json") {
            if let Some(close_idx) = html[tag_end..].find("</script>") {
                let content_end = tag_end + close_idx;
                let content = html[tag_end..content_end].trim();
                if !content.is_empty() {
                    blocks.push(content.to_string());
                }
                start_idx = content_end + "</script>".len();
                continue;
            }
        }
        start_idx = tag_end;
    }

    blocks
}

fn merge_structured_data_from_jsonld(
    val: &serde_json::Value,
    data: &mut StructuredData,
) {
    match val {
        serde_json::Value::Object(map) => {
            if data.title.is_none() {
                if let Some(v) = map.get("headline").or_else(|| map.get("name")) {
                    data.title = extract_string_value(v, &["name"]);
                }
            }
            if data.description.is_none() {
                if let Some(v) = map.get("description") {
                    data.description = extract_string_value(v, &[]);
                }
            }
            if data.author.is_none() {
                if let Some(v) = map.get("author") {
                    data.author = extract_string_value(v, &["name"]);
                }
            }
            if data.date_published.is_none() {
                if let Some(v) = map.get("datePublished") {
                    data.date_published = extract_string_value(v, &[]);
                }
            }
            if data.body_text.is_none() {
                if let Some(v) = map.get("articleBody") {
                    data.body_text = extract_string_value(v, &[]);
                }
            }
            if data.image_url.is_none() {
                if let Some(v) = map.get("image") {
                    data.image_url = extract_string_value(v, &["url", "contentUrl"]);
                }
            }
            if data.page_type.is_none() {
                if let Some(v) = map.get("@type") {
                    data.page_type = extract_string_value(v, &[]);
                }
            }

            for (_key, value) in map {
                merge_structured_data_from_jsonld(value, data);
            }
        }
        serde_json::Value::Array(arr) => {
            for entry in arr {
                merge_structured_data_from_jsonld(entry, data);
            }
        }
        _ => {}
    }
}

fn extract_string_value(
    val: &serde_json::Value,
    preferred_keys: &[&str],
) -> Option<String> {
    match val {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        }
        serde_json::Value::Array(arr) => {
            for entry in arr {
                if let Some(found) = extract_string_value(entry, preferred_keys) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Object(map) => {
            for key in preferred_keys {
                if let Some(v) = map.get(*key) {
                    if let Some(found) = extract_string_value(v, preferred_keys) {
                        return Some(found);
                    }
                }
            }
            if let Some(v) = map.get("@value") {
                return extract_string_value(v, preferred_keys);
            }
            None
        }
        _ => None,
    }
}

fn extract_meta_name(html: &str, name: &str) -> Option<String> {
    let patterns = [
        format!("name=\"{}\"", name),
        format!("name='{}'", name),
    ];

    for pat in &patterns {
        if let Some(idx) = html.find(pat) {
            let slice = &html[idx..];
            if let Some(content_idx) = slice.find("content=") {
                let content_slice = &slice[content_idx + "content=".len()..];
                if let Some(value) = extract_quoted_value(content_slice) {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }
    None
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
            page_type: None,
        }
    }
}

pub fn extract_main_content(html: &str, base_url: &str) -> ExtractionResult {
    let page_type = classify_page(html);
    let structured = extract_structured_data(html);

    let mut result = match page_type {
        PageType::Article => extract_article(html, base_url, &structured),
        PageType::SpaApplication => extract_spa(html, base_url, &structured),
        PageType::EcommerceProduct => extract_product(html, base_url, &structured),
        PageType::NavigationIndex => extract_navigation(html, base_url, &structured),
        PageType::MediaGallery => extract_gallery(html, base_url, &structured),
        PageType::Unknown => extract_unknown(html, base_url, &structured),
    };

    if result.title.is_none() {
        result.title = structured.title.clone();
    }
    if result.author.is_none() {
        result.author = structured.author.clone();
    }
    if result.published_date.is_none() {
        result.published_date = structured.date_published.clone();
    }
    if result.excerpt.is_none() {
        result.excerpt = structured.description.clone();
    }

    result.page_type = Some(page_type.as_str().to_string());
    result
}

fn extract_article(html: &str, base_url: &str, _structured: &StructuredData) -> ExtractionResult {
    let full_text = extract_all_text(html);

    if let Some(product) = try_readability(html, base_url) {
        let confidence = calculate_confidence(&product.content, &product.text, &full_text);

        if confidence >= 0.40 {
            let thin_content = is_thin(&product.text);
            let reading_time_minutes = reading_time_minutes(&product.text);

            return ExtractionResult {
                title: Some(product.title),
                author: extract_author(html),
                published_date: extract_published_date(html),
                excerpt: extract_excerpt(html),
                body_html: product.content,
                body_text: product.text,
                reading_time_minutes,
                confidence,
                thin_content,
                extraction_failed: false,
                method: "readability".to_string(),
                page_type: None,
            };
        }
    }

    let css = try_css_selector_extraction(html);
    let css_confidence = calculate_confidence(&css.body_html, &css.body_text, &full_text);

    if css_confidence >= 0.25 {
        let thin_content = is_thin(&css.body_text);
        let reading_time_minutes = reading_time_minutes(&css.body_text);

        return ExtractionResult {
            title: css.title,
            author: extract_author(html),
            published_date: extract_published_date(html),
            excerpt: extract_excerpt(html),
            body_html: css.body_html,
            body_text: css.body_text,
            reading_time_minutes,
            confidence: css_confidence,
            thin_content,
            extraction_failed: false,
            method: "css_selector".to_string(),
            page_type: None,
        };
    }

    let fallback = extract_largest_text_block(html);
    let thin_content = is_thin(&fallback.body_text);
    let reading_time_minutes = reading_time_minutes(&fallback.body_text);
    let fallback_empty = fallback.body_text.trim().is_empty();

    ExtractionResult {
        title: fallback.title.or_else(|| extract_title_tag(html)),
        author: extract_author(html),
        published_date: extract_published_date(html),
        excerpt: extract_excerpt(html),
        body_html: fallback.body_html,
        body_text: fallback.body_text,
        reading_time_minutes,
        confidence: if fallback_empty { 0.0 } else { 0.1 },
        thin_content,
        extraction_failed: fallback_empty,
        method: "article_largest_block".to_string(),
        page_type: None,
    }
}

fn extract_spa(html: &str, _base_url: &str, structured: &StructuredData) -> ExtractionResult {
    let full_text = extract_all_text(html);
    let selectors = ["main", "[role=\"main\"]", "[role='main']"];
    let mut css = try_css_selector_extraction_with_selectors(html, &selectors);

    if css.body_text.trim().is_empty() {
        css = extract_largest_text_block(html);
    }

    if css.body_text.trim().is_empty() && structured.description.is_some() {
        css.body_text = structured.description.clone().unwrap_or_default();
    }

    let confidence = if css.body_text.trim().is_empty() {
        0.0
    } else {
        calculate_confidence(&css.body_html, &css.body_text, &full_text)
    };

    let thin_content = is_thin(&css.body_text);
    let reading_time_minutes = reading_time_minutes(&css.body_text);

    let css_empty = css.body_text.trim().is_empty();

    ExtractionResult {
        title: css.title.or_else(|| extract_title_tag(html)),
        author: None,
        published_date: None,
        excerpt: structured.description.clone(),
        body_html: css.body_html,
        body_text: css.body_text,
        reading_time_minutes,
        confidence,
        thin_content,
        extraction_failed: css_empty,
        method: "spa_content".to_string(),
        page_type: None,
    }
}

fn extract_product(html: &str, _base_url: &str, structured: &StructuredData) -> ExtractionResult {
    let is_product = structured
        .page_type
        .as_deref()
        .map(|t| t.eq_ignore_ascii_case("product"))
        .unwrap_or(false)
        || extract_jsonld_types(html)
            .iter()
            .any(|t| t.eq_ignore_ascii_case("product"));

    if is_product {
        let body_text = structured
            .body_text
            .clone()
            .or_else(|| structured.description.clone())
            .unwrap_or_default();
        let thin_content = is_thin(&body_text);
        let reading_time_minutes = reading_time_minutes(&body_text);

        return ExtractionResult {
            title: structured.title.clone().or_else(|| extract_title_tag(html)),
            author: structured.author.clone(),
            published_date: structured.date_published.clone(),
            excerpt: structured.description.clone(),
            body_html: String::new(),
            body_text,
            reading_time_minutes,
            confidence: 0.85,
            thin_content,
            extraction_failed: false,
            method: "jsonld_product".to_string(),
            page_type: None,
        };
    }

    let document = Html::parse_document(html);
    let title = select_text_from_selectors(
        &document,
        &["[itemprop=\"name\"]", ".product-title"],
    )
    .or_else(|| extract_title_tag(html));
    let body_text = select_text_from_selectors(
        &document,
        &["[itemprop=\"description\"]", ".product-description"],
    )
    .unwrap_or_default();
    let body_html = select_html_from_selectors(
        &document,
        &["[itemprop=\"description\"]", ".product-description"],
    )
    .unwrap_or_default();

    let thin_content = is_thin(&body_text);
    let reading_time_minutes = reading_time_minutes(&body_text);
    let body_text_empty = body_text.trim().is_empty();

    ExtractionResult {
        title,
        author: structured.author.clone(),
        published_date: structured.date_published.clone(),
        excerpt: structured.description.clone(),
        body_html,
        body_text,
        reading_time_minutes,
        confidence: 0.25,
        thin_content,
        extraction_failed: body_text_empty,
        method: "css_product".to_string(),
        page_type: None,
    }
}

fn extract_navigation(html: &str, _base_url: &str, structured: &StructuredData) -> ExtractionResult {
    ExtractionResult {
        title: structured.title.clone().or_else(|| extract_title_tag(html)),
        author: None,
        published_date: None,
        excerpt: structured.description.clone(),
        body_html: String::new(),
        body_text: String::new(),
        reading_time_minutes: 0,
        confidence: 0.0,
        thin_content: true,
        extraction_failed: false,
        method: "navigation_index".to_string(),
        page_type: None,
    }
}

fn extract_gallery(html: &str, _base_url: &str, structured: &StructuredData) -> ExtractionResult {
    let document = Html::parse_document(html);
    let mut parts = Vec::new();

    if let Ok(sel) = Selector::parse("figcaption") {
        for el in document.select(&sel) {
            let text: String = el.text().collect::<Vec<_>>().join("");
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }

    if let Ok(sel) = Selector::parse("img[alt]") {
        for el in document.select(&sel) {
            if let Some(alt) = el.value().attr("alt") {
                let trimmed = alt.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }

    let body_text = parts.join("\n");
    let thin_content = is_thin(&body_text);
    let reading_time_minutes = reading_time_minutes(&body_text);

    ExtractionResult {
        title: structured.title.clone().or_else(|| extract_title_tag(html)),
        author: None,
        published_date: None,
        excerpt: structured.description.clone(),
        body_html: String::new(),
        body_text,
        reading_time_minutes,
        confidence: 0.2,
        thin_content,
        extraction_failed: parts.is_empty(),
        method: "gallery_captions".to_string(),
        page_type: None,
    }
}

fn extract_unknown(html: &str, base_url: &str, _structured: &StructuredData) -> ExtractionResult {
    let full_text = extract_all_text(html);

    if let Some(product) = try_readability(html, base_url) {
        let confidence = calculate_confidence(&product.content, &product.text, &full_text);
        if confidence >= 0.40 {
            let thin_content = is_thin(&product.text);
            let reading_time_minutes = reading_time_minutes(&product.text);

            return ExtractionResult {
                title: Some(product.title),
                author: extract_author(html),
                published_date: extract_published_date(html),
                excerpt: extract_excerpt(html),
                body_html: product.content,
                body_text: product.text,
                reading_time_minutes,
                confidence,
                thin_content,
                extraction_failed: false,
                method: "unknown_readability".to_string(),
                page_type: None,
            };
        }
    }

    let fallback = extract_largest_text_block(html);
    let thin_content = is_thin(&fallback.body_text);
    let reading_time_minutes = reading_time_minutes(&fallback.body_text);
    let fallback_empty = fallback.body_text.trim().is_empty();

    ExtractionResult {
        title: fallback.title.or_else(|| extract_title_tag(html)),
        author: extract_author(html),
        published_date: extract_published_date(html),
        excerpt: extract_excerpt(html),
        body_html: fallback.body_html,
        body_text: fallback.body_text,
        reading_time_minutes,
        confidence: if fallback_empty { 0.0 } else { 0.1 },
        thin_content,
        extraction_failed: fallback_empty,
        method: "unknown_largest_block".to_string(),
        page_type: None,
    }
}

fn reading_time_minutes(text: &str) -> u32 {
    let word_count = text.split_whitespace().count();
    if word_count == 0 {
        0
    } else {
        ((word_count as f32 / 200.0).ceil() as u32).max(1)
    }
}

fn is_thin(text: &str) -> bool {
    let non_ws_len = text.chars().filter(|c| !c.is_whitespace()).count();
    non_ws_len < 200
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

fn link_density_ratio(html: &str) -> f32 {
    let href_count = html.matches("href=").count();
    let para_count = html.matches("</p>").count();

    if para_count == 0 {
        return if href_count > 0 { 1.0 } else { 0.0 };
    }

    href_count as f32 / (href_count + para_count) as f32
}

fn calculate_confidence(body_html: &str, body_text: &str, full_page_text: &str) -> f32 {
    let ratio = text_to_tag_ratio(body_html);
    let para_count = body_html.matches("<p").count();
    let close_para_count = body_html.matches("</p>").count();
    let word_ratio = if full_page_text.is_empty() { 0.0 } else {
        let extracted_words = body_text.split_whitespace().count();
        let total_words = full_page_text.split_whitespace().count();
        if total_words == 0 { 0.0 } else { extracted_words as f32 / total_words as f32 }
    };
    let link_density = link_density_ratio(body_html);

    let low_confidence = ratio < 0.35
        || close_para_count < 5
        || word_ratio < 0.10
        || link_density > 0.95;

    if low_confidence {
        let failed = [
            ratio < 0.35,
            close_para_count < 5,
            word_ratio < 0.10,
            link_density > 0.95,
        ].iter().filter(|&&x| x).count();
        0.5 - (failed as f32 * 0.15)
    } else {
        (ratio * 0.35 + word_ratio * 0.35
         + (para_count.min(10) as f32 / 10.0) * 0.15
         + (1.0 - link_density) * 0.15)
            .min(1.0)
    }
}

struct CssExtraction {
    title: Option<String>,
    body_html: String,
    body_text: String,
}

fn try_css_selector_extraction(html: &str) -> CssExtraction {
    let selectors_order: &[&str] = &[
        "article",
        "main",
        "[role=\"main\"]",
        ".content",
        ".post",
    ];

    try_css_selector_extraction_with_selectors(html, selectors_order)
}

fn try_css_selector_extraction_with_selectors(
    html: &str,
    selectors_order: &[&str],
) -> CssExtraction {
    let document = Html::parse_document(html);
    let title = extract_title_tag(html);

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

fn extract_largest_text_block(html: &str) -> CssExtraction {
    let document = Html::parse_document(html);
    let title = extract_title_tag(html);

    let selectors_order: &[&str] = &["main", "[role=\"main\"]", "article", "section", "div"]; 
    let mut best_text = String::new();
    let mut best_html = String::new();

    for selector_str in selectors_order {
        let Ok(sel) = Selector::parse(selector_str) else {
            continue;
        };
        for el in document.select(&sel) {
            let tag = el.value().name();
            if tag == "nav" || tag == "header" || tag == "footer" || tag == "aside" {
                continue;
            }
            let text_content: String = el.text().collect::<Vec<_>>().join("");
            if text_content.trim().is_empty() {
                continue;
            }
            if text_content.len() > best_text.len() {
                use crate::crawler::sanitize_to_html;
                best_text = text_content;
                best_html = sanitize_to_html(&vec![el]);
            }
        }
    }

    CssExtraction {
        title,
        body_html: best_html,
        body_text: best_text,
    }
}

fn select_text_from_selectors(
    document: &Html,
    selectors: &[&str],
) -> Option<String> {
    for selector_str in selectors {
        if let Ok(sel) = Selector::parse(selector_str) {
            if let Some(el) = document.select(&sel).next() {
                let text: String = el.text().collect::<Vec<_>>().join("");
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn select_html_from_selectors(
    document: &Html,
    selectors: &[&str],
) -> Option<String> {
    for selector_str in selectors {
        if let Ok(sel) = Selector::parse(selector_str) {
            let elements: Vec<_> = document.select(&sel).collect();
            if elements.is_empty() {
                continue;
            }
            use crate::crawler::sanitize_to_html;
            let html_content = sanitize_to_html(&elements);
            if !html_content.trim().is_empty() {
                return Some(html_content);
            }
        }
    }
    None
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
