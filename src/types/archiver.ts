export type Engine = "local" | "cloud" | "local-scrapy";

export type ArchiveStatus = "idle" | "crawling" | "paused" | "completed" | "error";

export type PageStatus =
  | "Pending"
  | "Fetching"
  | "Scraping"
  | "Archiving"
  | "Completed"
  | { Failed: string }
  | { Skipped: string };

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
  cancelled: boolean;
}

export interface AppStatus {
  mongo_ok: boolean;
  zyte_available: boolean;
  zyte_project: string | null;
}

export interface CloudProgressPayload {
  job_key: string;
  state: string;
  items_scraped: number | null;
}
