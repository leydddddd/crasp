import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type {
  ArchiveStatus,
  ArchivedPage,
  CrawlConfig,
  CrawlDiscoverPayload,
  ScrapeProgressPayload,
  CloudProgressPayload,
  CrawlStats,
  CrawlDonePayload,
  PageStatus,
  Engine,
  AppStatus,
} from "@/types/archiver";

const MAX_QUEUE_DISPLAY = 500;

export function useArchiver() {
  const [status, setStatus] = useState<ArchiveStatus>("idle");
  const [engine, setEngine] = useState<Engine>("local");
  const [appStatus, setAppStatus] = useState<AppStatus | null>(null);
  const [zyteApiKey, setZyteApiKey] = useState("");
  const [zyteProjectId, setZyteProjectId] = useState("");
  const [pages, setPages] = useState<ArchivedPage[]>([]);
  const [stats, setStats] = useState<CrawlStats>({
    total: 0,
    completed: 0,
    failed: 0,
    skipped: 0,
    discovered: 0,
  });
  const [progressRev, bumpProgress] = useReducer((c: number) => c + 1, 0);
  const progressMapRef = useRef<Map<string, ScrapeProgressPayload>>(new Map());
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

  const pageMap = useRef<Map<string, ArchivedPage>>(new Map());
  const discoveredSet = useRef<Set<string>>(new Set());

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let cancelled = false;

    const setup = async () => {
      const u1 = await listen<ScrapeProgressPayload>(
        "scrape-progress",
        (event) => {
          progressMapRef.current.set(event.payload.url, event.payload);
          bumpProgress();
        }
      );
      if (cancelled) {
        u1();
        return;
      }
      unlisteners.push(u1);

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
      if (cancelled) {
        u2();
        return;
      }
      unlisteners.push(u2);

      const u3 = await listen<ArchivedPage>(
        "archive-success",
        (event) => {
          const page = event.payload;
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
          progressMapRef.current.delete(page.url);
          bumpProgress();
        }
      );
      if (cancelled) {
        u3();
        return;
      }
      unlisteners.push(u3);

      const u4 = await listen<CrawlDonePayload>("crawl-done", (event) => {
        setStatus((prev) => {
          if (prev === "crawling" || prev === "paused") {
            return event.payload.cancelled ? "idle" : "completed";
          }
          return prev;
        });
      });
      if (cancelled) {
        u4();
        return;
      }
      unlisteners.push(u4);

      const u5 = await listen<AppStatus>("app-ready", (event) => {
        setAppStatus(event.payload);
        if (event.payload.zyte_project) {
          setZyteProjectId(event.payload.zyte_project);
        }
      });
      if (cancelled) {
        u5();
        return;
      }
      unlisteners.push(u5);

      const u6 = await listen<{ mongo_ok: boolean; zyte_available: boolean; zyte_project: string | null }>("app-error", (event) => {
        setAppStatus(event.payload as AppStatus);
        const errMsg = (event.payload as Record<string, unknown>).error;
        if (typeof errMsg === "string") {
          setError(errMsg);
        }
      });
      if (cancelled) {
        u6();
        return;
      }
      unlisteners.push(u6);

      const u7 = await listen<CloudProgressPayload>("cloud-progress", () => {
        bumpProgress();
      });
      if (cancelled) {
        u7();
        return;
      }
      unlisteners.push(u7);

      try {
        const appState = await invoke<AppStatus>("get_app_status");
        setAppStatus(appState);
        if (appState.zyte_project) {
          setZyteProjectId(appState.zyte_project);
        }
      } catch {
        // get_app_status not available yet (app still initializing)
      }
    };

    setup();

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  const progress = useMemo(
    () => Array.from(progressMapRef.current.values()),
    [progressRev]
  );

  const startCrawl = useCallback(async () => {
    pageMap.current.clear();
    discoveredSet.current.clear();
    progressMapRef.current.clear();
    bumpProgress();
    setPages([]);
    setStats({ total: 0, completed: 0, failed: 0, skipped: 0, discovered: 0 });
    setError(null);
    setStatus("crawling");

    try {
      if (engine === "cloud") {
        if (!appStatus?.zyte_available && !zyteApiKey.trim()) {
          setError("Set ZYTE_API_KEY or enter a key to enable cloud engine");
          setStatus("error");
          return;
        }
        await invoke("start_cloud_crawl", {
          config,
          apiKey: zyteApiKey,
          projectId: zyteProjectId,
        });
      } else if (engine === "local-scrapy") {
        await invoke("local_scrapy_crawl", { config });
      } else {
        await invoke("start_crawl", { config });
      }
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }, [config, engine, appStatus, zyteApiKey, zyteProjectId]);

  const cancelCrawl = useCallback(async () => {
    try {
      await invoke("cancel_crawl");
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
    progressMapRef.current.clear();
    bumpProgress();
    setPages([]);
    setStats({ total: 0, completed: 0, failed: 0, skipped: 0, discovered: 0 });
    setError(null);
    setStatus("idle");
  }, []);

  return {
    status,
    pages,
    stats,
    progress,
    error,
    config,
    setConfig,
    engine,
    setEngine,
    appStatus,
    zyteApiKey,
    setZyteApiKey,
    zyteProjectId,
    setZyteProjectId,
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
