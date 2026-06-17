import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type {
  ArchiveStatus,
  ArchivedPage,
  CrawlConfig,
  CrawlDiscoverPayload,
  ScrapeProgressPayload,
  ArchiveSuccessPayload,
  CrawlStats,
  PageStatus,
} from "@/types/archiver";

const MAX_QUEUE_DISPLAY = 500;

export function useArchiver() {
  const [status, setStatus] = useState<ArchiveStatus>("idle");
  const [pages, setPages] = useState<ArchivedPage[]>([]);
  const [stats, setStats] = useState<CrawlStats>({
    total: 0,
    completed: 0,
    failed: 0,
    skipped: 0,
    discovered: 0,
  });
  const [progress, setProgress] = useState<Map<string, ScrapeProgressPayload>>(
    new Map()
  );
  const [error, setError] = useState<string | null>(null);
  const [config, setConfig] = useState<CrawlConfig>({
    seed_url: "",
    max_depth: 3,
    max_pages: 100,
    concurrency: 4,
    css_selectors: ["article", "main", "body"],
    preserve_html: true,
    hash_algorithm: "sha256",
  });

  const unlisteners = useRef<Array<() => void>>([]);
  const pageMap = useRef<Map<string, ArchivedPage>>(new Map());
  const discoveredSet = useRef<Set<string>>(new Set());

  useEffect(() => {
    const setup = async () => {
      const u1 = await listen<ScrapeProgressPayload>(
        "scrape-progress",
        (event) => {
          setProgress((prev) => {
            const next = new Map(prev);
            next.set(event.payload.url, event.payload);
            return next;
          });
        }
      );

      const u2 = await listen<CrawlDiscoverPayload>(
        "crawl-discover",
        (event) => {
          const url = event.payload.url;
          if (!discoveredSet.current.has(url)) {
            discoveredSet.current.add(url);
            setStats((prev) => ({
              ...prev,
              discovered: prev.discovered + 1,
            }));
          }
        }
      );

      const u3 = await listen<ArchiveSuccessPayload>(
        "archive-success",
        (event) => {
          const page = event.payload as unknown as ArchivedPage;
          pageMap.current.set(page.url, page);
          setPages(
            Array.from(pageMap.current.values()).slice(-MAX_QUEUE_DISPLAY)
          );
          setStats((prev) => {
            const next = { ...prev, total: prev.total + 1 };
            if (isCompleted(page.status)) next.completed++;
            else if (isFailed(page.status)) next.failed++;
            else if (isSkipped(page.status)) next.skipped++;
            return next;
          });
          setProgress((prev) => {
            const next = new Map(prev);
            next.delete(page.url);
            return next;
          });
        }
      );

      unlisteners.current = [u1, u2, u3];
    };

    setup();

    return () => {
      unlisteners.current.forEach((fn) => fn());
    };
  }, []);

  const startCrawl = useCallback(async () => {
    if (status === "crawling" || status === "paused") return;

    pageMap.current.clear();
    discoveredSet.current.clear();
    setPages([]);
    setStats({ total: 0, completed: 0, failed: 0, skipped: 0, discovered: 0 });
    setProgress(new Map());
    setError(null);
    setStatus("crawling");

    try {
      await invoke("start_crawl", { config });
      setStatus("completed");
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }, [config, status]);

  const cancelCrawl = useCallback(async () => {
    try {
      await invoke("cancel_crawl");
      setStatus("idle");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const pauseCrawl = useCallback(async () => {
    try {
      await invoke("pause_crawl");
      setStatus("paused");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const resumeCrawl = useCallback(async () => {
    try {
      await invoke("resume_crawl");
      setStatus("crawling");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const resetCrawl = useCallback(() => {
    pageMap.current.clear();
    discoveredSet.current.clear();
    setPages([]);
    setStats({ total: 0, completed: 0, failed: 0, skipped: 0, discovered: 0 });
    setProgress(new Map());
    setError(null);
    setStatus("idle");
  }, []);

  return {
    status,
    pages,
    stats,
    progress: Array.from(progress.values()),
    error,
    config,
    setConfig,
    startCrawl,
    cancelCrawl,
    pauseCrawl,
    resumeCrawl,
    resetCrawl,
  };
}

function isCompleted(s: PageStatus): boolean {
  return s === "Completed";
}

function isFailed(s: PageStatus): boolean {
  return typeof s === "object" && "Failed" in s;
}

function isSkipped(s: PageStatus): boolean {
  return typeof s === "object" && "Skipped" in s;
}
