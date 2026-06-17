import scrapy
import hashlib
import json
from urllib.parse import urljoin, urlparse, urldefrag
from datetime import datetime, timezone


STRIP_TAGS = {
    "script", "style", "noscript", "iframe", "object", "embed",
    "applet", "form", "input", "button", "select", "textarea",
    "svg", "canvas",
}

TRACKING_ATTRS = {
    "onclick", "onload", "onerror", "onmouseover", "onmouseout",
    "onmousedown", "onmouseup", "onkeydown", "onkeyup", "onfocus",
    "onblur", "onsubmit", "onchange", "data-tracking", "data-ad",
    "data-analytics", "data-pixel",
}


def compute_hash(content, algorithm):
    if algorithm == "md5":
        return hashlib.md5(content.encode("utf-8")).hexdigest()
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def extract_and_sanitize(response, css_selectors, preserve_html):
    from parsel import Selector

    for sel_str in css_selectors:
        try:
            els = response.css(sel_str)
            if els:
                matched = els
                break
        except Exception:
            continue
    else:
        for tag in ["article", "main", "body"]:
            els = response.css(tag)
            if els:
                matched = els
                break
        else:
            return ""

    if not matched:
        return ""

    if preserve_html:
        return sanitize_to_html(matched, response)
    return sanitize_to_text(matched, response)


def _strip_element(el):
    tag = el.root.tag if hasattr(el, "root") and callable(getattr(el.root, "get", None)) else ""
    if isinstance(tag, str) and tag.lower() in STRIP_TAGS:
        return True
    return False


def sanitize_to_html(elements, response):
    parts = []
    for el in elements:
        raw_html = el.get()
        if raw_html:
            parts.append(raw_html)
    return "\n".join(parts)


def sanitize_to_text(elements, response):
    texts = []
    for el in elements:
        text = el.xpath("string()").get()
        if text and text.strip():
            texts.append(text.strip())
    return "\n\n".join(texts)


class ArchiveSpider(scrapy.Spider):
    name = "crasp_archive"

    def __init__(
        self,
        seed_url="",
        max_depth=3,
        max_pages=100,
        css_selectors="article,main,body",
        preserve_html="true",
        hash_algorithm="sha256",
        **kwargs,
    ):
        super().__init__(**kwargs)
        self.seed_url = seed_url
        self.max_depth = int(max_depth)
        self.max_pages = int(max_pages)
        if isinstance(css_selectors, str):
            self.css_selectors = [s.strip() for s in css_selectors.split(",") if s.strip()]
        else:
            self.css_selectors = list(css_selectors)
        self.preserve_html = preserve_html.lower() == "true" if isinstance(preserve_html, str) else bool(preserve_html)
        self.hash_algorithm = hash_algorithm
        self.visited = set()
        self.pages_archived = 0

        parsed = urlparse(seed_url)
        self.seed_domain = parsed.hostname
        self.seed_scheme = parsed.scheme

    def start_requests(self):
        if not self.seed_url:
            return
        normalized = self._normalize_url(self.seed_url)
        self.visited.add(normalized)
        yield scrapy.Request(self.seed_url, callback=self.parse, meta={"depth": 0})

    def parse(self, response):
        depth = response.meta.get("depth", 0)

        title = response.css("title::text").get("").strip() or response.url

        links = []
        for href in response.css("a::attr(href)").getall():
            if href.startswith("#") or href.startswith("javascript:") or href.startswith("mailto:"):
                continue
            abs_url = urljoin(response.url, href)
            abs_url, _ = urldefrag(abs_url)
            links.append(abs_url)

        css_selectors = self.css_selectors + ["article", "main", "body"]
        content = self._extract_content(response, css_selectors, self.preserve_html)

        content_hash = compute_hash(content, self.hash_algorithm)
        content_format = "html" if self.preserve_html else "text"

        self.pages_archived += 1

        yield {
            "url": response.url,
            "depth": depth,
            "title": title,
            "content": content,
            "content_format": content_format,
            "hash": content_hash,
            "hash_algorithm": self.hash_algorithm,
            "discovered_links": len(links),
            "status_code": response.status,
            "timestamp": datetime.now(timezone.utc).isoformat(),
        }

        if depth >= self.max_depth:
            return
        if self.pages_archived >= self.max_pages:
            return

        for link in links:
            if self.pages_archived >= self.max_pages:
                break
            parsed = urlparse(link)
            if parsed.hostname != self.seed_domain:
                continue
            if parsed.scheme != self.seed_scheme:
                continue
            normalized = self._normalize_url(link)
            if normalized in self.visited:
                continue
            self.visited.add(normalized)
            self.pages_archived += 1
            yield scrapy.Request(
                link,
                callback=self.parse,
                meta={"depth": depth + 1},
                errback=self.errback,
            )

    def errback(self, failure):
        self.logger.info(f"Request failed: {failure.request.url}")

    def _extract_content(self, response, selectors, preserve_html):
        for sel_str in selectors:
            try:
                els = response.css(sel_str)
                if els:
                    if preserve_html:
                        return sanitize_to_html(els, response)
                    return sanitize_to_text(els, response)
            except Exception:
                continue
        return ""

    def _normalize_url(self, url):
        parsed = urlparse(url)
        normalized = parsed._replace(fragment="").geturl()
        if normalized.endswith("/"):
            normalized = normalized[:-1]
        return normalized.lower()
