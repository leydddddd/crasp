export type Engine = "local" | "cloud" | "local-scrapy";

export type ArchiveStatus = "idle" | "crawling" | "paused" | "completed" | "error" | "cancelled";

export type PageStatus =
  | "Pending"
  | "Fetching"
  | "Scraping"
  | "Archiving"
  | "Completed"
  | { Failed: string }
  | { Skipped: string };

export type ServiceState = "not_configured" | "configured_unverified" | "connected" | "unreachable";

export type PersistTarget =
  | { mongo: { db: string; collection: string } }
  | { local_file: { path: string } };

export type StorageSource =
  | "Mongo"
  | { LocalFile: { path: string } };

export type StorageUsed =
  | "Mongo"
  | { LocalFile: { path: string } }
  | { Both: { local_path: string } };

export type PageStage =
  | { stage: "discovered" }
  | { stage: "fetching" }
  | { stage: "fetched"; status_code: number }
  | { stage: "parsing" }
  | { stage: "sanitizing" }
  | { stage: "preserving" }
  | { stage: "hashing" }
  | { stage: "persisting"; target: PersistTarget }
  | { stage: "persisted"; target: PersistTarget }
  | { stage: "failed"; failed_stage: string; reason: string };

export type HashAlgorithm = "md5" | "sha256";

export interface CrawlConfig {
  seed_url: string;
  max_depth: number;
  max_pages: number;
  concurrency: number;
  css_selectors: string[];
  preserve_html: boolean;
  hash_algorithm: HashAlgorithm;
}

export interface AssetImage {
  src: string;
  alt: string | null;
  caption: string | null;
  in_main_content: boolean;
  width: number | null;
  height: number | null;
}

export interface AssetVideo {
  src: string;
  kind: string;
  video_id: string | null;
  in_main_content: boolean;
}

export interface AssetDocument {
  src: string;
  link_text: string | null;
  mime_type: string | null;
  in_main_content: boolean;
}

export interface PageAssets {
  images: AssetImage[];
  videos: AssetVideo[];
  documents: AssetDocument[];
  og_image: string | null;
  og_description: string | null;
  og_published_time: string | null;
}

export interface ArchivedPage {
  url: string;
  depth: number;
  status: PageStatus;
  title: string;
  content: string | null;
  hash: string | null;
  hash_algorithm: string | null;
  discovered_links: number;
  timestamp: string;
  crawl_id: string | null;
  extracted_title: string | null;
  author: string | null;
  published_date: string | null;
  excerpt: string | null;
  reading_time_minutes: number | null;
  body_text: string | null;
  body_html: string | null;
  assets: PageAssets | null;
  extraction_method: string | null;
  extraction_confidence: number | null;
  thin_content: boolean | null;
  page_type: string | null;
  deep_fetched: boolean | null;
  deep_fetch_duration_ms: number | null;
}

export interface CrawlDiscoverPayload {
  url: string;
  depth?: number;
  parent?: string;
  link_count?: number;
}

export interface ScrapeProgressPayload {
  url: string;
  status: string;
  depth: number;
}

export interface ArchiveSuccessPayload {
  url: string;
  depth: number;
  status: PageStatus;
  title: string;
  hash: string | null;
  timestamp: string;
}

export interface CrawlStats {
  total: number;
  completed: number;
  failed: number;
  skipped: number;
  discovered: number;
}

export interface CrawlDonePayload {
  pages_archived: number;
  pages_completed: number;
  pages_failed: number;
  pages_skipped: number;
  cancelled: boolean;
  crawl_id: string;
  storage_used: StorageUsed | null;
  deep_fetched_count: number;
}

export interface AppStatus {
  mongo_state: ServiceState;
  mongo_detail: string | null;
  zyte_state: ServiceState;
  zyte_detail: string | null;
  zyte_project: string | null;
  zyte_available: boolean;
  deep_fetch_enabled: boolean;
  chrome_available: boolean;
}

export interface CloudProgressPayload {
  job_key: string;
  state: string;
  items_scraped: number | null;
}

export interface LogEntry {
  timestamp: string;
  level: string;
  engine: string;
  message: string;
}

export interface PageSummary {
  url: string;
  title: string;
  depth: number;
  stage: string;
  status_reason: string | null;
  content_size: number;
  timestamp: string;
  source: StorageSource;
  content_preview: string | null;
  extracted_title: string | null;
  author: string | null;
  published_date: string | null;
  excerpt: string | null;
  reading_time_minutes: number | null;
  body_text: string | null;
  body_html: string | null;
  assets: PageAssets | null;
  extraction_method: string | null;
  extraction_confidence: number | null;
  thin_content: boolean | null;
  page_type: string | null;
  deep_fetched: boolean | null;
  deep_fetch_duration_ms: number | null;
}

export interface MongoConnectionStatus {
  ok: boolean;
  db_name: string | null;
  pages_count: number | null;
  message: string | null;
}

export interface ZyteConnectionStatus {
  ok: boolean;
  project_name: string | null;
  message: string | null;
}

export interface PageStageEvent {
  url: string;
  crawl_id: string;
  stage: PageStage;
}

export function storageSourceLabel(source: StorageSource): string {
  if (source === "Mongo") return "MongoDB";
  if (typeof source === "object" && "LocalFile" in source) return `Local file: ${source.LocalFile.path}`;
  return "Unknown";
}

export type ExportFormat = "plain_text" | "markdown" | "html" | "epub";
export type ExportScope = "single_page" | "whole_crawl_one_file" | "whole_crawl_folder";
export type ExportContent = "content_only" | "with_metadata" | "with_assets" | "full";

export interface ExportRequest {
  format: ExportFormat;
  scope: ExportScope;
  content: ExportContent;
  pageUrl?: string;
  crawlId?: string;
  source?: StorageSource;
}

export interface ExportResult {
  path: string;
  page_count: number;
  format: ExportFormat;
  scope: ExportScope;
}

export function storageUsedLabel(su: StorageUsed): string {
  if (su === "Mongo") return "MongoDB (crasp/pages)";
  if (typeof su === "object") {
    if ("LocalFile" in su) return `Local file: ${su.LocalFile.path}`;
    if ("Both" in su) return `MongoDB + Local file: ${su.Both.local_path}`;
  }
  return "Unknown";
}
