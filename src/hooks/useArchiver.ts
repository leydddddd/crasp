import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type {
  ArchiveStatus,
  ArchivedPage,
  CrawlConfig,
  CrawlDiscoverPayload,
  ScrapeProgressPayload,
  CrawlStats,
  CrawlDonePayload,
  PageStatus,
  Engine,
  AppStatus,
  LogEntry,
  PageStageEvent,
  PageStage,
  PersistTarget,
  MongoConnectionStatus,
  ZyteConnectionStatus,
  PageSummary,
  StorageSource,
} from "@/types/archiver";

const MAX_QUEUE_DISPLAY = 500;
const FLUSH_INTERVAL_MS = 250;
const FLUSH_BATCH_SIZE = 50;
const MAX_LOG_ENTRIES = 2000;

function formatPersistTarget(target: PersistTarget): string {
  if ("mongo" in target) {
    return `MongoDB (${target.mongo.db}/${target.mongo.collection})`;
  }
  if ("local_file" in target) {
    return `Local file: ${target.local_file.path}`;
  }
  return "Unknown";
}

function formatStageName(stage: PageStage): string {
  if ("stage" in stage) {
    const s = stage.stage;
    switch (s) {
      case "discovered": return "Discovered";
      case "fetching": return "Fetching";
      case "fetched": return `Fetched (${stage.status_code})`;
      case "parsing": return "Parsing";
      case "sanitizing": return "Sanitizing HTML";
      case "preserving": return "Preserving content";
      case "hashing": return "Computing hash";
      case "persisting": return `Persisting → ${formatPersistTarget(stage.target)}`;
      case "persisted": return `Persisted ✓ ${formatPersistTarget(stage.target)}`;
      case "failed": return `Failed at ${stage.failed_stage}: ${stage.reason}`;
    }
  }
  return "Unknown";
}

function serializeSource(source: StorageSource): StorageSource {
  if (source === "Mongo") return "Mongo";
  return source;
}

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

  const pageStagesRef = useRef<Map<string, PageStage>>(new Map());
  const [pageStageRev, bumpPageStage] = useReducer((c: number) => c + 1, 0);

  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [logFilter, setLogFilter] = useState<{ level: string; engine: string; search: string }>({
    level: "all",
    engine: "all",
    search: "",
  });
  const [autoScroll, setAutoScroll] = useState(true);

  const appendStructuredLog = useCallback((entry: LogEntry) => {
    setLogs((prev) => {
      const next = [...prev, entry];
      if (next.length > MAX_LOG_ENTRIES) {
        return next.slice(next.length - MAX_LOG_ENTRIES);
      }
      return next;
    });
  }, []);

  const clearLogs = useCallback(() => setLogs([]), []);

  const [testingMongo, setTestingMongo] = useState(false);
  const [testingZyte, setTestingZyte] = useState(false);

  const testMongoConnection = useCallback(async (uri: string) => {
    setTestingMongo(true);
    try {
      const result = await invoke<MongoConnectionStatus>("test_mongo_connection", { uri });
      const newStatus = await invoke<AppStatus>("get_app_status");
      setAppStatus(newStatus);
      return result;
    } catch (e) {
      return { ok: false, db_name: null, pages_count: null, message: String(e) } as MongoConnectionStatus;
    } finally {
      setTestingMongo(false);
    }
  }, []);

  const testZyteConnection = useCallback(async (apiKey: string, projectId: string) => {
    setTestingZyte(true);
    try {
      const result = await invoke<ZyteConnectionStatus>("test_zyte_connection", {
        apiKey,
        projectId,
      });
      const newStatus = await invoke<AppStatus>("get_app_status");
      setAppStatus(newStatus);
      return result;
    } catch (e) {
      return { ok: false, project_name: null, message: String(e) } as ZyteConnectionStatus;
    } finally {
      setTestingZyte(false);
    }
  }, []);

  const [archivedPages, setArchivedPages] = useState<PageSummary[]>([]);
  const [loadingArchived, setLoadingArchived] = useState(false);

  const loadArchivedPages = useCallback(async (crawlId?: string) => {
    setLoadingArchived(true);
    try {
      const result = await invoke<PageSummary[]>("list_archived_pages", { crawlId: crawlId || null });
      const normalized: PageSummary[] = result.map((p) => ({
        ...p,
        source: serializeSource(p.source),
      }));
      setArchivedPages(normalized);
    } catch {
      setArchivedPages([]);
    } finally {
      setLoadingArchived(false);
    }
  }, []);

  const [crawlSummary, setCrawlSummary] = useState<CrawlDonePayload | null>(null);

  const openDataFolder = useCallback(async () => {
    try {
      await invoke("open_data_folder");
    } catch (e) {
      console.error("Failed to open data folder:", e);
    }
  }, []);

  const pageMap = useRef<Map<string, ArchivedPage>>(new Map());
  const discoveredSet = useRef<Set<string>>(new Set());

  const batchRef = useRef<ArchivedPage[]>([]);
  const batchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const scheduleFlush = useCallback(() => {
    if (batchTimerRef.current !== null) return;
    batchTimerRef.current = setTimeout(() => {
      const batch = batchRef.current;
      batchRef.current = [];

      if (batch.length === 0) {
        batchTimerRef.current = null;
        return;
      }

      for (const page of batch) {
        pageMap.current.set(page.url, page);
      }

      if (pageMap.current.size > MAX_QUEUE_DISPLAY) {
        const excess = pageMap.current.size - MAX_QUEUE_DISPLAY;
        let evicted = 0;
        for (const key of pageMap.current.keys()) {
          if (evicted >= excess) break;
          pageMap.current.delete(key);
          evicted++;
        }
      }

      setPages(Array.from(pageMap.current.values()));
      batchTimerRef.current = null;
    }, FLUSH_INTERVAL_MS);
  }, []);

  const enqueuePage = useCallback(
    (page: ArchivedPage) => {
      batchRef.current.push(page);

      setStats((prev) => {
        const next = { ...prev, total: prev.total + 1 };
        if (isCompleted(page.status)) next.completed++;
        else if (isFailed(page.status)) next.failed++;
        else if (isSkipped(page.status)) next.skipped++;
        return next;
      });

      progressMapRef.current.delete(page.url);
      bumpProgress();

      if (batchRef.current.length >= FLUSH_BATCH_SIZE) {
        if (batchTimerRef.current !== null) {
          clearTimeout(batchTimerRef.current);
          batchTimerRef.current = null;
        }
        const batch = batchRef.current;
        batchRef.current = [];
        for (const p of batch) {
          pageMap.current.set(p.url, p);
        }
        if (pageMap.current.size > MAX_QUEUE_DISPLAY) {
          const excess = pageMap.current.size - MAX_QUEUE_DISPLAY;
          let evicted = 0;
          for (const key of pageMap.current.keys()) {
            if (evicted >= excess) break;
            pageMap.current.delete(key);
            evicted++;
          }
        }
        setPages(Array.from(pageMap.current.values()));
      } else {
        scheduleFlush();
      }
    },
    [scheduleFlush],
  );

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let cancelled = false;

    const setup = async () => {
      const listeners: Array<[string, (event: Record<string, unknown>) => void]> = [
        [
          "scrape-progress",
          (event: Record<string, unknown>) => {
            const payload = event.payload as ScrapeProgressPayload;
            progressMapRef.current.set(payload.url, payload);
            bumpProgress();
          },
        ],
        [
          "crawl-discover",
          (event: Record<string, unknown>) => {
            const payload = event.payload as CrawlDiscoverPayload;
            const url = payload.url;
            if (!discoveredSet.current.has(url)) {
              discoveredSet.current.add(url);
              setStats((prev) => ({
                ...prev,
                discovered: prev.discovered + 1,
              }));
            }
          },
        ],
        [
          "archive-success",
          (event: Record<string, unknown>) => {
            const page = event.payload as ArchivedPage;
            enqueuePage(page);
          },
        ],
        [
          "archive-failed",
          (event: Record<string, unknown>) => {
            const page = event.payload as ArchivedPage;
            enqueuePage(page);
          },
        ],
        [
          "page-stage",
          (event: Record<string, unknown>) => {
            const payload = event.payload as PageStageEvent;
            pageStagesRef.current.set(payload.url, payload.stage);
            bumpPageStage();
          },
        ],
        [
          "crawl-done",
          (event: Record<string, unknown>) => {
            const payload = event.payload as CrawlDonePayload;
            setCrawlSummary(payload);
            setStatus((prev) => {
              if (prev === "crawling" || prev === "paused") {
                return payload.cancelled ? "cancelled" : "completed";
              }
              return prev;
            });
          },
        ],
        [
          "app-ready",
          (event: Record<string, unknown>) => {
            const payload = event.payload as AppStatus;
            setAppStatus(payload);
            if (payload.zyte_project) {
              setZyteProjectId(payload.zyte_project);
            }
          },
        ],
        [
          "app-error",
          (event: Record<string, unknown>) => {
            const payload = event.payload as Record<string, unknown>;
            setAppStatus(payload as unknown as AppStatus);
            const errMsg = payload.error;
            if (typeof errMsg === "string") {
              setError(errMsg);
            }
          },
        ],
        [
          "cloud-progress",
          (event: Record<string, unknown>) => {
            void event.payload;
            bumpProgress();
          },
        ],
        [
          "app-log",
          (event: Record<string, unknown>) => {
            const entry = event.payload as LogEntry;
            appendStructuredLog(entry);
          },
        ],
      ];

      for (const [eventName, handler] of listeners) {
        if (cancelled) break;
        const unlisten = await listen(eventName, handler as any);
        if (cancelled) {
          unlisten();
          break;
        }
        unlisteners.push(unlisten);
      }

      if (!cancelled) {
        try {
          const appState = await invoke<AppStatus>("get_app_status");
          if (!cancelled) {
            setAppStatus(appState);
            if (appState.zyte_project) {
              setZyteProjectId(appState.zyte_project);
            }
          }
        } catch {
          // App still initializing
        }
      }
    };

    setup();

    return () => {
      cancelled = true;
      if (batchTimerRef.current !== null) {
        clearTimeout(batchTimerRef.current);
        batchTimerRef.current = null;
      }
      for (let i = unlisteners.length - 1; i >= 0; i--) {
        unlisteners[i]();
      }
      unlisteners.length = 0;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const progress = useMemo(
    () => Array.from(progressMapRef.current.values()),
    [progressRev],
  );

  const pageStages = useMemo(
    () => new Map(pageStagesRef.current),
    [pageStageRev],
  );

  const filteredLogs = useMemo(() => {
    return logs.filter((entry) => {
      if (logFilter.level !== "all" && entry.level !== logFilter.level) return false;
      if (logFilter.engine !== "all" && entry.engine !== logFilter.engine) return false;
      if (logFilter.search && !entry.message.toLowerCase().includes(logFilter.search.toLowerCase())) return false;
      return true;
    });
  }, [logs, logFilter]);

  const getPageStageLabel = useCallback(
    (url: string): string | null => {
      const stage = pageStages.get(url);
      if (!stage) return null;
      return formatStageName(stage);
    },
    [pageStages],
  );

  const startCrawl = useCallback(async () => {
    pageMap.current.clear();
    discoveredSet.current.clear();
    progressMapRef.current.clear();
    pageStagesRef.current.clear();
    batchRef.current = [];
    bumpProgress();
    bumpPageStage();
    setPages([]);
    setStats({ total: 0, completed: 0, failed: 0, skipped: 0, discovered: 0 });
    setError(null);
    setCrawlSummary(null);
    setStatus("crawling");

    try {
      if (engine === "cloud") {
        if (appStatus?.zyte_state === "not_configured" && !zyteApiKey.trim()) {
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
    pageStagesRef.current.clear();
    batchRef.current = [];
    bumpProgress();
    bumpPageStage();
    setPages([]);
    setStats({ total: 0, completed: 0, failed: 0, skipped: 0, discovered: 0 });
    setError(null);
    setCrawlSummary(null);
    setStatus("idle");
  }, []);

  const dismissSummary = useCallback(() => {
    setCrawlSummary(null);
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
    testingMongo,
    testingZyte,
    testMongoConnection,
    testZyteConnection,
    pageStages,
    getPageStageLabel,
    logs: filteredLogs,
    allLogs: logs,
    logFilter,
    setLogFilter,
    clearLogs,
    autoScroll,
    setAutoScroll,
    archivedPages,
    loadingArchived,
    loadArchivedPages,
    crawlSummary,
    dismissSummary,
    openDataFolder,
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
