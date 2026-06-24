import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  AlertTriangle,
  Archive,
  ArrowLeft,
  ArrowRight,
  CheckCircle2,
  Cloud,
  Database,
  Download,
  Edit3,
  FileDown,
  FolderOpen,
  Globe,
  Image,
  Loader2,
  Monitor,
  Pause,
  Play,
  Search,
  Square,
  Terminal,
  X,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useArchiver } from "@/hooks/useArchiver";
import { ExportPanel } from "./ExportPanel";
import type {
  AppStatus,
  ArchiveStatus,
  AssetRow,
  CrawlSummary,
  Engine,
  FrontierPreviewResult,
  LogEntry,
  PageSummary,
  PageStatus,
  ServiceState,
  StorageUsed,
} from "@/types/archiver";

const ENGINE_OPTIONS: {
  value: Engine;
  label: string;
  icon: React.ReactNode;
}[] = [
  { value: "local", label: "Local", icon: <Database className="h-3 w-3" /> },
  { value: "cloud", label: "Cloud", icon: <Cloud className="h-3 w-3" /> },
  {
    value: "local-scrapy",
    label: "Local Scrapy",
    icon: <Terminal className="h-3 w-3" />,
  },
];

type ViewState =
  | "sessions"
  | "crawl-setup"
  | "live-crawl"
  | "session-review"
  | "export";

type ExportTarget =
  | {
      context: "single_page";
      page: PageSummary;
      crawl: CrawlSummary | null;
    }
  | {
      context: "whole_crawl";
      crawl: CrawlSummary;
    }
  | {
      context: "selected_pages";
      crawl: CrawlSummary;
      selectedUrls: string[];
    };

function serviceStateColor(state: ServiceState | undefined): string {
  switch (state) {
    case "connected":
      return "bg-emerald-500";
    case "configured_unverified":
      return "bg-amber-500";
    case "unreachable":
      return "bg-red-500";
    case "not_configured":
    default:
      return "bg-gray-600";
  }
}

function serviceStateLabel(
  state: ServiceState | undefined,
  name: string,
  detail: string | null,
): string {
  switch (state) {
    case "connected":
      return `${name}: Connected${detail ? ` (${detail})` : ""}`;
    case "configured_unverified":
      return `${name}: Unverified`;
    case "unreachable":
      return `${name}: Unreachable${detail ? ` - ${detail}` : ""}`;
    case "not_configured":
    default:
      return `${name}: Not configured`;
  }
}

function storageUsedBadge(storage: StorageUsed | null): React.ReactNode {
  if (!storage) return null;
  if (storage === "Mongo") {
    return (
      <span className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-[9px] bg-emerald-900/30 text-emerald-400 font-medium">
        <Database className="h-2.5 w-2.5" />
        MongoDB
      </span>
    );
  }
  if (typeof storage === "object" && "LocalFile" in storage) {
    return (
      <span className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-[9px] bg-amber-900/30 text-amber-400 font-medium">
        <FolderOpen className="h-2.5 w-2.5" />
        Local file
      </span>
    );
  }
  if (typeof storage === "object" && "Both" in storage) {
    return (
      <span className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-[9px] bg-blue-900/30 text-blue-400 font-medium">
        <Database className="h-2.5 w-2.5" />
        Mongo + Local
      </span>
    );
  }
  return null;
}

function engineBadge(engine: Engine): React.ReactNode {
  const opt = ENGINE_OPTIONS.find((e) => e.value === engine);
  if (!opt) return null;
  return (
    <span className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-[9px] bg-gray-800 text-gray-300 font-medium">
      {opt.icon}
      {opt.label}
    </span>
  );
}

function pageTypeBadge(pageType: string | null): React.ReactNode {
  if (!pageType) return null;
  switch (pageType) {
    case "article":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-blue-900/30 text-blue-400 font-medium">
          Article
        </span>
      );
    case "spa_application":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-purple-900/30 text-purple-400 font-medium">
          SPA
        </span>
      );
    case "ecommerce_product":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-emerald-900/30 text-emerald-400 font-medium">
          Product
        </span>
      );
    case "navigation_index":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-300 font-medium">
          Index
        </span>
      );
    case "media_gallery":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-orange-900/30 text-orange-400 font-medium">
          Gallery
        </span>
      );
    case "unknown":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-400 font-medium">
          Unknown
        </span>
      );
    default:
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-400 font-medium">
          {pageType}
        </span>
      );
  }
}

function extractionMethodBadge(method: string | null): React.ReactNode {
  if (!method) return null;
  switch (method) {
    case "readability":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-blue-900/30 text-blue-400 font-medium">
          readability
        </span>
      );
    case "css_selector":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-300 font-medium">
          css_selector
        </span>
      );
    case "zyte_autoextract":
      return (
        <span className="inline-flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] bg-emerald-900/30 text-emerald-400 font-medium">
          Zyte
        </span>
      );
    case "readability (browser)":
    case "css_selector (browser)":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-purple-900/30 text-purple-400 font-medium">
          browser
        </span>
      );
    case "failed":
    case "raw":
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-amber-900/30 text-amber-400 font-medium">
          {method}
        </span>
      );
    default:
      return (
        <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-400 font-medium">
          {method}
        </span>
      );
  }
}

function formatDateTime(value: string | null): string {
  if (!value) return "-";
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return value;
  return dt.toLocaleString();
}

function displaySessionName(crawl: CrawlSummary): string {
  if (crawl.name && crawl.name.trim().length > 0) return crawl.name;
  try {
    const url = new URL(crawl.seed_url);
    const date = new Date(crawl.started_at);
    const dateText = Number.isNaN(date.getTime())
      ? crawl.started_at
      : date.toLocaleDateString();
    return `${url.hostname} - ${dateText}`;
  } catch {
    return crawl.seed_url || "Untitled session";
  }
}

function deriveSessionStatus(
  crawl: CrawlSummary,
  activeCrawlId: string | null,
  appStatus: ArchiveStatus,
): "Completed" | "Partial" | "Cancelled" | "In Progress" {
  if (
    activeCrawlId &&
    activeCrawlId === crawl.crawl_id &&
    appStatus === "crawling"
  ) {
    return "In Progress";
  }
  if (
    activeCrawlId &&
    activeCrawlId === crawl.crawl_id &&
    appStatus === "paused"
  ) {
    return "In Progress";
  }
  if (crawl.cancelled) return "Cancelled";
  if (!crawl.finished_at) return "In Progress";
  if (crawl.stats.failed > 0) return "Partial";
  return "Completed";
}

function statusBadge(status: string): React.ReactNode {
  switch (status) {
    case "Completed":
      return (
        <span className="rounded-full bg-emerald-900/30 px-2 py-0.5 text-[10px] text-emerald-400">
          Completed
        </span>
      );
    case "Partial":
      return (
        <span className="rounded-full bg-amber-900/30 px-2 py-0.5 text-[10px] text-amber-400">
          Partial
        </span>
      );
    case "Cancelled":
      return (
        <span className="rounded-full bg-gray-800 px-2 py-0.5 text-[10px] text-gray-400">
          Cancelled
        </span>
      );
    case "In Progress":
    default:
      return (
        <span className="rounded-full bg-blue-900/30 px-2 py-0.5 text-[10px] text-blue-400">
          In Progress
        </span>
      );
  }
}

export function ArchiverDashboard() {
  const archiver = useArchiver();
  const [view, setView] = useState<ViewState>("sessions");
  const [selectedCrawlId, setSelectedCrawlId] = useState<string | null>(null);
  const [exportTarget, setExportTarget] = useState<ExportTarget | null>(null);
  const [showLogs, setShowLogs] = useState(false);

  const [searchTerm, setSearchTerm] = useState("");
  const [statusFilter, setStatusFilter] = useState<
    "all" | "completed" | "partial" | "cancelled" | "in-progress"
  >("all");

  const [editingCrawlId, setEditingCrawlId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState("");

  const [seedValidation, setSeedValidation] = useState<
    | { state: "idle" }
    | { state: "valid"; message: string }
    | { state: "invalid"; message: string }
  >({ state: "idle" });

  const [liveExpandedUrl, setLiveExpandedUrl] = useState<string | null>(null);
  const [reviewExpandedUrl, setReviewExpandedUrl] = useState<string | null>(
    null,
  );

  const [frontierPreview, setFrontierPreview] = useState<FrontierPreviewResult | null>(null);
  const [frontierPreviewLoading, setFrontierPreviewLoading] = useState(false);
  const [frontierPreviewError, setFrontierPreviewError] = useState<string | null>(null);
  const [frontierSelectedUrls, setFrontierSelectedUrls] = useState<Set<string>>(new Set());

  const [reviewTab, setReviewTab] = useState<"pages" | "assets" | "log">("pages");
  const [reviewPageTypeFilter, setReviewPageTypeFilter] = useState<string>("all");
  const [reviewAssetTypeFilter, setReviewAssetTypeFilter] = useState<string>("all");
  const [headerEditingCrawlId, setHeaderEditingCrawlId] = useState<string | null>(null);
  const [headerEditingName, setHeaderEditingName] = useState("");

  const selectedCrawl = useMemo(() => {
    if (!selectedCrawlId) return null;
    return archiver.crawls.find((c) => c.crawl_id === selectedCrawlId) || null;
  }, [archiver.crawls, selectedCrawlId]);

  useEffect(() => {
    if (view === "sessions") {
      archiver.loadCrawls();
    }
  }, [view, archiver]);

  useEffect(() => {
    if (view === "session-review" && selectedCrawlId) {
      archiver.loadArchivedPages(selectedCrawlId);
    }
  }, [view, selectedCrawlId, archiver]);

  useEffect(() => {
    if (view === "session-review" && reviewTab === "assets" && selectedCrawlId) {
      archiver.loadAssets(selectedCrawlId);
    }
  }, [view, reviewTab, selectedCrawlId, archiver]);

  const handlePreviewFrontier = useCallback(async () => {
    if (!archiver.config.seed_url.trim()) {
      setFrontierPreviewError("Enter a seed URL first.");
      return;
    }
    setFrontierPreviewLoading(true);
    setFrontierPreviewError(null);
    setFrontierPreview(null);
    try {
      const result = await archiver.previewFrontier(archiver.config.seed_url.trim(), archiver.config.max_pages);
      setFrontierPreview(result);
      setFrontierSelectedUrls(new Set(result.sample_urls));
    } catch (e) {
      setFrontierPreviewError(String(e));
    } finally {
      setFrontierPreviewLoading(false);
    }
  }, [archiver]);

  const handleExportSelectedPages = useCallback(
    (crawl: CrawlSummary, selectedUrls: string[]) => {
      setExportTarget({ context: "selected_pages", crawl, selectedUrls });
      setView("export");
    },
    [],
  );

  const filteredCrawls = useMemo(() => {
    const term = searchTerm.toLowerCase();
    return archiver.crawls.filter((crawl) => {
      const status = deriveSessionStatus(
        crawl,
        archiver.activeCrawlId,
        archiver.status,
      );
      if (statusFilter === "completed" && status !== "Completed") return false;
      if (statusFilter === "partial" && status !== "Partial") return false;
      if (statusFilter === "cancelled" && status !== "Cancelled") return false;
      if (statusFilter === "in-progress" && status !== "In Progress")
        return false;
      if (statusFilter === "all") {
        // no-op
      }
      if (!term) return true;
      const name = displaySessionName(crawl).toLowerCase();
      return crawl.seed_url.toLowerCase().includes(term) || name.includes(term);
    });
  }, [
    archiver.crawls,
    archiver.activeCrawlId,
    archiver.status,
    searchTerm,
    statusFilter,
  ]);

  const handleStartArchiving = useCallback(async () => {
    await archiver.startCrawl();
    setView("live-crawl");
  }, [archiver]);

  const handleReviewSession = useCallback((crawlId: string) => {
    setSelectedCrawlId(crawlId);
    setView("session-review");
  }, []);

  const handleExportSession = useCallback((crawl: CrawlSummary) => {
    setExportTarget({ context: "whole_crawl", crawl });
    setView("export");
  }, []);

  const handleExportPage = useCallback(
    (page: PageSummary) => {
      setExportTarget({ context: "single_page", page, crawl: selectedCrawl });
      setView("export");
    },
    [selectedCrawl],
  );

  const handleValidateSeed = useCallback(async () => {
    if (!archiver.config.seed_url.trim()) {
      setSeedValidation({ state: "invalid", message: "Enter a seed URL." });
      return;
    }
    try {
      const normalized = await invoke<string>("validate_url", {
        url: archiver.config.seed_url.trim(),
      });
      setSeedValidation({
        state: "valid",
        message: `URL validated: ${normalized}`,
      });
    } catch (e) {
      setSeedValidation({
        state: "invalid",
        message: String(e),
      });
    }
  }, [archiver.config.seed_url]);

  const breadcrumb = useMemo(() => {
    const items = ["Sessions"];
    if (view === "crawl-setup") items.push("Crawl Setup");
    if (view === "live-crawl") items.push("Live Crawl");
    if (view === "session-review" && selectedCrawl) {
      items.push(displaySessionName(selectedCrawl));
      items.push("Review");
    }
    if (view === "export") {
      if (exportTarget?.context === "single_page") {
        items.push("Export Page");
      } else if (exportTarget?.context === "whole_crawl") {
        items.push(displaySessionName(exportTarget.crawl));
        items.push("Export");
      } else {
        items.push("Export");
      }
    }
    return items.join(" > ");
  }, [view, selectedCrawl, exportTarget]);

  const livePages = useMemo(() => {
    return [...archiver.pages].reverse().slice(0, 200);
  }, [archiver.pages]);

  const thinCount = useMemo(() => {
    return archiver.pages.filter((p) => p.thin_content).length;
  }, [archiver.pages]);

  const filteredReviewPages = useMemo(() => {
    return archiver.archivedPages.filter((page) => {
      const term = searchTerm.toLowerCase();
      if (term) {
        const match =
          page.url.toLowerCase().includes(term) ||
          page.title.toLowerCase().includes(term);
        if (!match) return false;
      }
      if (statusFilter === "partial") {
        return page.stage === "Failed" || page.stage === "Skipped";
      }
      if (statusFilter === "completed") {
        return page.stage === "Completed";
      }
      if (reviewPageTypeFilter !== "all" && page.page_type !== reviewPageTypeFilter) {
        return false;
      }
      return true;
    });
  }, [archiver.archivedPages, searchTerm, statusFilter, reviewPageTypeFilter]);

  return (
    <div className="flex h-screen w-screen flex-col bg-gray-950 text-gray-100 overflow-hidden">
      <header className="flex items-center justify-between border-b border-gray-800 px-4 py-2.5">
        <div className="flex items-center gap-3">
          <Database className="h-5 w-5 text-crasp-400" />
          <h1 className="text-lg font-semibold tracking-tight">Crasp</h1>
        </div>
        <div className="flex-1 px-6">
          <p className="text-xs text-gray-400 text-center">{breadcrumb}</p>
        </div>
        <div className="flex items-center gap-3 text-[11px] text-gray-400">
          <span
            className={`h-2 w-2 rounded-full ${serviceStateColor(
              archiver.appStatus?.mongo_state,
            )}`}
            title={serviceStateLabel(
              archiver.appStatus?.mongo_state,
              "MongoDB",
              archiver.appStatus?.mongo_detail ?? null,
            )}
          />
          <span
            className={`h-2 w-2 rounded-full ${serviceStateColor(
              archiver.appStatus?.zyte_state,
            )}`}
            title={serviceStateLabel(
              archiver.appStatus?.zyte_state,
              "Zyte",
              archiver.appStatus?.zyte_detail ?? null,
            )}
          />
          <span
            className={`h-2 w-2 rounded-full ${
              archiver.appStatus?.chrome_available
                ? "bg-emerald-500"
                : "bg-gray-600"
            }`}
            title={
              archiver.appStatus?.chrome_available
                ? "Chrome available"
                : "Chrome not found"
            }
          />
        </div>
      </header>

      <main className="flex-1 overflow-hidden">
        {view === "sessions" && (
          <section className="flex h-full flex-col">
            <div className="flex items-center justify-between border-b border-gray-800 px-6 py-4">
              <div>
                <h2 className="text-lg font-semibold text-gray-100">
                  Sessions
                </h2>
                <p className="text-xs text-gray-500">
                  Manage past archives or start a new crawl.
                </p>
              </div>
              <button
                onClick={() => setView("crawl-setup")}
                className="rounded-md bg-crasp-600 px-4 py-2 text-sm font-semibold text-white hover:bg-crasp-500 transition-colors"
              >
                New Session
              </button>
            </div>

            <div className="flex items-center gap-3 border-b border-gray-800 px-6 py-3">
              <div className="relative flex-1">
                <Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-500" />
                <input
                  value={searchTerm}
                  onChange={(e) => setSearchTerm(e.target.value)}
                  placeholder="Search by session name or URL"
                  className="w-full rounded-md border border-gray-800 bg-gray-900 pl-8 pr-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
                />
              </div>
              <select
                value={statusFilter}
                onChange={(e) =>
                  setStatusFilter(
                    e.target.value as
                      | "all"
                      | "completed"
                      | "partial"
                      | "cancelled"
                      | "in-progress",
                  )
                }
                className="rounded-md border border-gray-800 bg-gray-900 px-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
              >
                <option value="all">All</option>
                <option value="completed">Completed</option>
                <option value="partial">Partial</option>
                <option value="cancelled">Cancelled</option>
                <option value="in-progress">In Progress</option>
              </select>
            </div>

            <div className="flex-1 overflow-auto px-6 py-4">
              {archiver.loadingCrawls ? (
                <div className="flex h-full items-center justify-center text-gray-600">
                  <Loader2 className="h-6 w-6 animate-spin" />
                </div>
              ) : filteredCrawls.length === 0 ? (
                <div className="flex h-full flex-col items-center justify-center text-gray-600">
                  <Archive className="mb-3 h-12 w-12 opacity-30" />
                  <p className="text-sm">No archives yet.</p>
                  <p className="text-xs text-gray-700">
                    Start your first session to see it here.
                  </p>
                </div>
              ) : (
                <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
                  {filteredCrawls.map((crawl) => {
                    const status = deriveSessionStatus(
                      crawl,
                      archiver.activeCrawlId,
                      archiver.status,
                    );
                    return (
                      <div
                        key={crawl.crawl_id}
                        className="rounded-lg border border-gray-800 bg-gray-900/60 p-4 hover:border-gray-700 transition-colors"
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0">
                            <div className="flex items-center gap-2">
                              {editingCrawlId === crawl.crawl_id ? (
                                <input
                                  value={editingName}
                                  onChange={(e) =>
                                    setEditingName(e.target.value)
                                  }
                                  onBlur={() => {
                                    archiver.renameCrawl(
                                      crawl.crawl_id,
                                      editingName.trim() || null,
                                    );
                                    setEditingCrawlId(null);
                                  }}
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") {
                                      archiver.renameCrawl(
                                        crawl.crawl_id,
                                        editingName.trim() || null,
                                      );
                                      setEditingCrawlId(null);
                                    }
                                  }}
                                  className="w-full rounded-md border border-gray-700 bg-gray-950 px-2 py-1 text-sm text-gray-100 focus:border-crasp-500 focus:outline-none"
                                />
                              ) : (
                                <h3 className="text-sm font-semibold text-gray-100 truncate">
                                  {displaySessionName(crawl)}
                                </h3>
                              )}
                              <button
                                onClick={() => {
                                  setEditingCrawlId(crawl.crawl_id);
                                  setEditingName(displaySessionName(crawl));
                                }}
                                className="text-gray-500 hover:text-gray-300"
                                title="Rename session"
                              >
                                <Edit3 className="h-3.5 w-3.5" />
                              </button>
                            </div>
                            <p className="mt-1 text-xs text-gray-500 truncate">
                              {crawl.seed_url}
                            </p>
                          </div>
                          <div className="flex flex-col items-end gap-2">
                            {statusBadge(status)}
                            {engineBadge(crawl.source)}
                            {storageUsedBadge(crawl.storage_used)}
                          </div>
                        </div>
                        <div className="mt-3 text-xs text-gray-500">
                          <span>{formatDateTime(crawl.started_at)}</span>
                          {crawl.finished_at && (
                            <span> - {formatDateTime(crawl.finished_at)}</span>
                          )}
                        </div>
                        <div className="mt-2 text-xs text-gray-400">
                          {crawl.stats.archived} pages
                          {crawl.stats.failed > 0 &&
                            ` - ${crawl.stats.failed} failed`}
                          {crawl.stats.skipped > 0 &&
                            ` - ${crawl.stats.skipped} skipped`}
                        </div>
                        <div className="mt-4 flex items-center gap-2">
                          <button
                            onClick={() => handleReviewSession(crawl.crawl_id)}
                            className="rounded-md bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-200 hover:bg-gray-700 transition-colors"
                          >
                            Review
                          </button>
                          <button
                            onClick={() => handleExportSession(crawl)}
                            className="rounded-md bg-crasp-600/20 px-3 py-1.5 text-xs font-medium text-crasp-400 hover:bg-crasp-600/30 transition-colors"
                          >
                            Export
                          </button>
                          {status === "In Progress" && (
                            <button
                              onClick={() => setView("live-crawl")}
                              className="ml-auto inline-flex items-center gap-1 rounded-md bg-blue-600/20 px-3 py-1.5 text-xs font-medium text-blue-300 hover:bg-blue-600/30 transition-colors"
                            >
                              Live
                              <ArrowRight className="h-3 w-3" />
                            </button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </section>
        )}

        {view === "crawl-setup" && (
          <section className="flex h-full flex-col">
            <div className="flex items-center gap-3 border-b border-gray-800 px-6 py-4">
              <button
                onClick={() => setView("sessions")}
                className="inline-flex items-center gap-2 text-sm text-gray-400 hover:text-gray-200"
              >
                <ArrowLeft className="h-4 w-4" />
                Back
              </button>
              <h2 className="text-lg font-semibold text-gray-100">
                Crawl Setup
              </h2>
            </div>

            <div className="flex-1 overflow-auto px-6 py-6">
              <div className="max-w-3xl space-y-6">
                <section className="rounded-lg border border-gray-800 bg-gray-900/50 p-5">
                  <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-gray-400">
                    Target
                  </h3>
                  <InputField
                    label="Seed URL"
                    value={archiver.config.seed_url}
                    onChange={(v) =>
                      archiver.setConfig((c) => ({ ...c, seed_url: v }))
                    }
                    placeholder="https://example.com"
                    onBlur={() => {
                      if (archiver.config.seed_url.trim()) {
                        handleValidateSeed();
                      }
                    }}
                  />
                  <div className="mt-2 flex items-center gap-2">
                    <button
                      onClick={handleValidateSeed}
                      className="rounded-md bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-300 hover:bg-gray-700 transition-colors"
                    >
                      Check URL
                    </button>
                    {seedValidation.state === "valid" && (
                      <span className="text-xs text-emerald-400">
                        {seedValidation.message}
                      </span>
                    )}
                    {seedValidation.state === "invalid" && (
                      <span className="text-xs text-red-400">
                        {seedValidation.message}
                      </span>
                    )}
                  </div>
                </section>

                <section className="rounded-lg border border-gray-800 bg-gray-900/50 p-5">
                  <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-gray-400">
                    Scope
                  </h3>
                  <div className="grid grid-cols-2 gap-4">
                    <InputField
                      label="Max Pages"
                      value={String(archiver.config.max_pages)}
                      onChange={(v) =>
                        archiver.setConfig((c) => ({
                          ...c,
                          max_pages: Math.max(1, parseInt(v) || 1),
                        }))
                      }
                      type="number"
                    />
                    <InputField
                      label="Max Depth"
                      value={String(archiver.config.max_depth)}
                      onChange={(v) =>
                        archiver.setConfig((c) => ({
                          ...c,
                          max_depth: Math.max(1, parseInt(v) || 1),
                        }))
                      }
                      type="number"
                    />
                    <InputField
                      label="Concurrency"
                      value={String(archiver.config.concurrency)}
                      onChange={(v) =>
                        archiver.setConfig((c) => ({
                          ...c,
                          concurrency: Math.max(
                            1,
                            Math.min(16, parseInt(v) || 1),
                          ),
                        }))
                      }
                      type="number"
                    />
                    <div>
                      <label className="mb-1 block text-[11px] text-gray-500">
                        Hash Algorithm
                      </label>
                      <select
                        value={archiver.config.hash_algorithm}
                        onChange={(e) =>
                          archiver.setConfig((c) => ({
                            ...c,
                            hash_algorithm: e.target.value as "md5" | "sha256",
                          }))
                        }
                        className="w-full rounded-md border border-gray-700 bg-gray-800 px-2.5 py-1.5 text-xs text-gray-200 focus:border-crasp-500 focus:outline-none"
                      >
                        <option value="sha256">SHA-256</option>
                        <option value="md5">MD5</option>
                      </select>
                    </div>
                  </div>
                  <div className="mt-4">
                    <InputField
                      label="CSS Selectors (comma-separated)"
                      value={archiver.config.css_selectors.join(", ")}
                      onChange={(v) =>
                        archiver.setConfig((c) => ({
                          ...c,
                          css_selectors: v
                            .split(",")
                            .map((s) => s.trim())
                            .filter(Boolean),
                        }))
                      }
                      placeholder="article, main, body"
                    />
                    <p className="mt-1 text-[11px] text-gray-500">
                      Use selectors to focus extraction on main content.
                    </p>
                  </div>
                  <div className="mt-4 flex items-center gap-2">
                    <button
                      onClick={() =>
                        archiver.setConfig((c) => ({
                          ...c,
                          preserve_html: !c.preserve_html,
                        }))
                      }
                      className={`relative h-4 w-8 rounded-full transition-colors ${
                        archiver.config.preserve_html
                          ? "bg-crasp-600"
                          : "bg-gray-700"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 h-3 w-3 rounded-full bg-white transition-transform ${
                          archiver.config.preserve_html
                            ? "left-[18px]"
                            : "left-0.5"
                        }`}
                      />
                    </button>
                    <span className="text-xs text-gray-400">
                      {archiver.config.preserve_html
                        ? "Preserve raw HTML"
                        : "Sanitize and extract text"}
                    </span>
                  </div>
                </section>

                <section className="rounded-lg border border-gray-800 bg-gray-900/50 p-5">
                  <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-gray-400">
                    Options
                  </h3>
                  <div className="space-y-3">
                    <div>
                      <label className="mb-1.5 block text-[11px] text-gray-500">
                        Engine
                      </label>
                      <div className="flex rounded-md border border-gray-700 overflow-hidden">
                        {ENGINE_OPTIONS.map((opt) => {
                          const isCloud = opt.value === "cloud";
                          const disabled =
                            isCloud && !archiver.appStatus?.zyte_available;
                          return (
                            <button
                              key={opt.value}
                              onClick={() => archiver.setEngine(opt.value)}
                              disabled={disabled}
                              className={`flex flex-1 items-center justify-center gap-1 px-2 py-1.5 text-[11px] font-medium transition-colors disabled:opacity-50 ${
                                archiver.engine === opt.value
                                  ? "bg-crasp-600 text-white"
                                  : "bg-gray-800 text-gray-400 hover:bg-gray-700"
                              }`}
                            >
                              {opt.icon}
                              {opt.label}
                            </button>
                          );
                        })}
                      </div>
                      {!archiver.appStatus?.zyte_available && (
                        <p className="mt-1 text-[11px] text-gray-500">
                          Configure Zyte to enable the cloud engine.
                        </p>
                      )}
                    </div>
                    <div className="flex items-center gap-2 text-xs text-gray-400">
                      <Monitor className="h-3.5 w-3.5" />
                      <span>
                        Deep fetch runs automatically when Chrome is available.
                      </span>
                    </div>
                  </div>
                </section>

                <div className="flex items-center justify-between">
                  <button
                    onClick={handlePreviewFrontier}
                    disabled={!archiver.config.seed_url.trim() || frontierPreviewLoading}
                    className="text-xs text-gray-400 hover:text-gray-200 disabled:opacity-40"
                  >
                    {frontierPreviewLoading ? "Loading..." : "Preview Frontier"}
                  </button>
                  <button
                    onClick={() => {
                      if (frontierSelectedUrls.size > 0 && frontierPreview) {
                        archiver.setConfig((c) => ({ ...c, allowed_urls: Array.from(frontierSelectedUrls) }));
                      }
                      handleStartArchiving();
                    }}
                    disabled={!archiver.config.seed_url.trim()}
                    className="rounded-md bg-crasp-600 px-5 py-2.5 text-sm font-semibold text-white hover:bg-crasp-500 transition-colors disabled:opacity-40"
                  >
                    Start Archiving
                  </button>
                </div>

                {frontierPreviewError && (
                  <p className="text-xs text-red-400">{frontierPreviewError}</p>
                )}

                {frontierPreview && (
                  <div className="rounded-lg border border-gray-800 bg-gray-900/50 p-4">
                    <div className="flex items-center justify-between mb-3">
                      <h4 className="text-xs font-semibold text-gray-300">Frontier Preview</h4>
                      <button onClick={() => setFrontierPreview(null)} className="text-gray-500 hover:text-gray-300">
                        <X className="h-3.5 w-3.5" />
                      </button>
                    </div>
                    <p className="text-xs text-gray-400 mb-2">{frontierPreview.total_count} URLs discovered</p>
                    <div className="flex items-center gap-2 mb-2">
                      <button
                        onClick={() => setFrontierSelectedUrls(new Set(frontierPreview.sample_urls))}
                        className="text-[10px] text-crasp-400 hover:text-crasp-300"
                      >
                        Select all
                      </button>
                      <button
                        onClick={() => setFrontierSelectedUrls(new Set())}
                        className="text-[10px] text-gray-500 hover:text-gray-300"
                      >
                        Clear
                      </button>
                      <span className="text-[10px] text-gray-500">
                        {frontierSelectedUrls.size}/{frontierPreview.sample_urls.length} selected
                      </span>
                    </div>
                    <div className="max-h-48 overflow-auto space-y-1">
                      {frontierPreview.sample_urls.map((url) => (
                        <label key={url} className="flex items-center gap-2 text-[11px] text-gray-400 hover:text-gray-300 cursor-pointer">
                          <input
                            type="checkbox"
                            checked={frontierSelectedUrls.has(url)}
                            onChange={() => {
                              setFrontierSelectedUrls((prev) => {
                                const next = new Set(prev);
                                if (next.has(url)) next.delete(url); else next.add(url);
                                return next;
                              });
                            }}
                            className="h-3 w-3 rounded border-gray-700 bg-gray-800 text-crasp-600"
                          />
                          <span className="truncate">{url}</span>
                        </label>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </div>
          </section>
        )}

        {view === "live-crawl" && (
          <section className="flex h-full">
            <div className="w-80 border-r border-gray-800 p-5 space-y-4">
              <div className="flex items-center justify-between">
                <h2 className="text-sm font-semibold text-gray-100">
                  Live Crawl
                </h2>
                <button
                  onClick={() => setView("sessions")}
                  className="text-xs text-gray-400 hover:text-gray-200"
                >
                  Dismiss
                </button>
              </div>
              <div className="text-xs text-gray-400">
                <div className="truncate">{archiver.config.seed_url}</div>
                <div className="mt-1">
                  Started: {formatDateTime(archiver.crawlStartedAt)}
                </div>
              </div>
              <div className="rounded-md border border-gray-800 bg-gray-900/50 p-3">
                <div className="flex items-center justify-between text-[11px] text-gray-500">
                  <span>Progress</span>
                  <span>
                    {Math.min(
                      archiver.stats.completed,
                      archiver.config.max_pages,
                    )}
                    {archiver.config.max_pages > 0 &&
                      ` / ${archiver.config.max_pages}`}
                  </span>
                </div>
                <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-gray-800">
                  <div
                    className="h-full rounded-full bg-crasp-600 transition-all"
                    style={{
                      width: `${Math.min(
                        100,
                        (archiver.stats.completed /
                          Math.max(1, archiver.config.max_pages)) *
                          100,
                      )}%`,
                    }}
                  />
                </div>
                <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-gray-400">
                  <Stat label="Completed" value={archiver.stats.completed} />
                  <Stat label="Failed" value={archiver.stats.failed} />
                  <Stat label="Skipped" value={archiver.stats.skipped} />
                  <Stat label="Thin" value={thinCount} />
                </div>
              </div>
              <div className="space-y-2">
                {archiver.status === "crawling" && (
                  <button
                    onClick={archiver.pauseCrawl}
                    className="flex w-full items-center justify-center gap-2 rounded-md bg-amber-600/20 px-3 py-2 text-xs font-medium text-amber-400 hover:bg-amber-600/30"
                  >
                    <Pause className="h-3.5 w-3.5" />
                    Pause
                  </button>
                )}
                {archiver.status === "paused" && (
                  <button
                    onClick={archiver.resumeCrawl}
                    className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600/20 px-3 py-2 text-xs font-medium text-emerald-400 hover:bg-emerald-600/30"
                  >
                    <Play className="h-3.5 w-3.5" />
                    Resume
                  </button>
                )}
                <button
                  onClick={() => {
                    if (
                      window.confirm(
                        "Cancel this crawl? Pages archived so far will be saved.",
                      )
                    ) {
                      archiver.cancelCrawl();
                    }
                  }}
                  className="flex w-full items-center justify-center gap-2 rounded-md bg-red-600/20 px-3 py-2 text-xs font-medium text-red-400 hover:bg-red-600/30"
                >
                  <Square className="h-3.5 w-3.5" />
                  Cancel
                </button>
              </div>
              <div className="rounded-md border border-gray-800 bg-gray-900/50 p-3 text-xs text-gray-400">
                {engineBadge(archiver.engine)}
                <div className="mt-2 flex items-center gap-1">
                  <span>Storage:</span>
                  {storageUsedBadge(
                    archiver.crawlSummary?.storage_used ?? null,
                  ) || <span className="text-gray-500">Pending</span>}
                </div>
              </div>
            </div>
            <div className="flex-1 overflow-hidden">
              <div className="border-b border-gray-800 px-6 py-3 text-xs text-gray-500">
                Live feed - showing most recent 200 pages
              </div>
              <div className="h-full overflow-auto">
                {livePages.length === 0 ? (
                  <div className="flex h-full flex-col items-center justify-center text-gray-600">
                    <Globe className="mb-3 h-12 w-12 opacity-30" />
                    <p className="text-sm">Waiting for pages...</p>
                  </div>
                ) : (
                  <div className="divide-y divide-gray-900">
                    {livePages.map((page) => {
                      const stageLabel = archiver.getPageStageLabel(page.url);
                      const isExpanded = liveExpandedUrl === page.url;
                      return (
                        <div key={page.url}>
                          <button
                            onClick={() =>
                              setLiveExpandedUrl(isExpanded ? null : page.url)
                            }
                            className="flex w-full items-center gap-3 px-6 py-3 text-left hover:bg-gray-900/50"
                          >
                            <PageStatusIcon status={page.status} />
                            <div className="min-w-0 flex-1">
                              <p className="truncate text-sm text-gray-200">
                                {page.title || page.url}
                              </p>
                              <p className="truncate text-[11px] text-gray-600">
                                {page.url}
                              </p>
                            </div>
                            <span className="text-[11px] text-crasp-400">
                              {stageLabel || "Queued"}
                            </span>
                          </button>
                          {isExpanded && (
                            <div className="bg-gray-900/60 px-6 py-3 text-xs text-gray-400">
                              <div className="flex flex-wrap items-center gap-2">
                                {page.page_type &&
                                  pageTypeBadge(page.page_type)}
                                {extractionMethodBadge(page.extraction_method)}
                                {page.extraction_confidence != null && (
                                  <span>
                                    confidence:{" "}
                                    {(page.extraction_confidence * 100).toFixed(
                                      0,
                                    )}
                                    %
                                  </span>
                                )}
                              </div>
                              {page.excerpt && (
                                <p className="mt-2 text-gray-300 italic">
                                  {page.excerpt.slice(0, 200)}
                                </p>
                              )}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>

            {archiver.crawlSummary && (
              <div className="absolute inset-0 flex items-center justify-center bg-black/60">
                <div className="w-full max-w-lg rounded-lg border border-gray-800 bg-gray-950 p-6 shadow-xl">
                  <h3 className="text-lg font-semibold text-gray-100">
                    {archiver.crawlSummary.cancelled
                      ? "Crawl Cancelled"
                      : "Archiving Complete"}
                  </h3>
                  <p className="mt-2 text-xs text-gray-400">
                    {archiver.crawlSummary.pages_completed} completed -{" "}
                    {archiver.crawlSummary.pages_failed} failed -{" "}
                    {archiver.crawlSummary.pages_skipped} skipped
                  </p>
                  <div className="mt-4 flex items-center gap-2">
                    <button
                      onClick={() => {
                        setSelectedCrawlId(
                          archiver.crawlSummary?.crawl_id ?? null,
                        );
                        setView("session-review");
                        archiver.loadCrawls();
                      }}
                      className="rounded-md bg-crasp-600 px-4 py-2 text-xs font-medium text-white hover:bg-crasp-500"
                    >
                      Review Session
                    </button>
                    <button
                      onClick={() => {
                        setView("crawl-setup");
                        archiver.resetCrawl();
                      }}
                      className="rounded-md bg-gray-800 px-4 py-2 text-xs text-gray-300 hover:bg-gray-700"
                    >
                      New Session
                    </button>
                    <button
                      onClick={() => {
                        setView("sessions");
                        archiver.resetCrawl();
                      }}
                      className="ml-auto text-xs text-gray-400 hover:text-gray-200"
                    >
                      Dismiss
                    </button>
                  </div>
                </div>
              </div>
            )}
          </section>
        )}

        {view === "session-review" && selectedCrawl && (
          <section className="flex h-full flex-col">
            <div className="flex items-center justify-between border-b border-gray-800 px-6 py-4">
              <div>
                <button
                  onClick={() => setView("sessions")}
                  className="mb-2 inline-flex items-center gap-2 text-xs text-gray-400 hover:text-gray-200"
                >
                  <ArrowLeft className="h-3 w-3" />
                  Sessions
                </button>
                {headerEditingCrawlId === selectedCrawl.crawl_id ? (
                  <input
                    value={headerEditingName}
                    onChange={(e) => setHeaderEditingName(e.target.value)}
                    onBlur={() => {
                      archiver.renameCrawl(selectedCrawl.crawl_id, headerEditingName.trim() || null);
                      setHeaderEditingCrawlId(null);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        archiver.renameCrawl(selectedCrawl.crawl_id, headerEditingName.trim() || null);
                        setHeaderEditingCrawlId(null);
                      }
                    }}
                    className="text-lg font-semibold text-gray-100 bg-gray-950 border border-gray-700 rounded px-2 py-0.5 focus:border-crasp-500 focus:outline-none"
                  />
                ) : (
                  <div className="flex items-center gap-2">
                    <h2 className="text-lg font-semibold text-gray-100">
                      {displaySessionName(selectedCrawl)}
                    </h2>
                    <button
                      onClick={() => {
                        setHeaderEditingCrawlId(selectedCrawl.crawl_id);
                        setHeaderEditingName(displaySessionName(selectedCrawl));
                      }}
                      className="text-gray-500 hover:text-gray-300"
                      title="Rename session"
                    >
                      <Edit3 className="h-3.5 w-3.5" />
                    </button>
                  </div>
                )}
                <p className="text-xs text-gray-500">
                  {selectedCrawl.seed_url}
                </p>
                <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-gray-500">
                  {statusBadge(
                    deriveSessionStatus(
                      selectedCrawl,
                      archiver.activeCrawlId,
                      archiver.status,
                    ),
                  )}
                  {engineBadge(selectedCrawl.source)}
                  {storageUsedBadge(selectedCrawl.storage_used)}
                  <span>{formatDateTime(selectedCrawl.started_at)}</span>
                </div>
              </div>
              <div className="flex items-center gap-2">
                {archiver.selectedPageUrls.size > 0 && (
                  <button
                    onClick={() => handleExportSelectedPages(selectedCrawl, Array.from(archiver.selectedPageUrls))}
                    className="rounded-md bg-emerald-600/20 px-3 py-2 text-xs font-medium text-emerald-300 hover:bg-emerald-600/30"
                  >
                    <FileDown className="h-3 w-3 inline mr-1" />
                    Export Selected ({archiver.selectedPageUrls.size})
                  </button>
                )}
                <button
                  onClick={() => handleExportSession(selectedCrawl)}
                  className="rounded-md bg-crasp-600 px-4 py-2 text-xs font-medium text-white hover:bg-crasp-500"
                >
                  Export Session
                </button>
              </div>
            </div>

            <div className="flex border-b border-gray-800 px-6">
              {(["pages", "assets", "log"] as const).map((tab) => (
                <button
                  key={tab}
                  onClick={() => setReviewTab(tab)}
                  className={`px-4 py-2.5 text-xs font-medium capitalize transition-colors ${
                    reviewTab === tab
                      ? "text-crasp-400 border-b-2 border-crasp-400"
                      : "text-gray-500 hover:text-gray-300"
                  }`}
                >
                  {tab}
                  {tab === "pages" && archiver.archivedPages.length > 0 && (
                    <span className="ml-1 text-[9px] text-gray-600">({archiver.archivedPages.length})</span>
                  )}
                  {tab === "assets" && archiver.assets.length > 0 && (
                    <span className="ml-1 text-[9px] text-gray-600">({archiver.assets.length})</span>
                  )}
                </button>
              ))}
            </div>

            {reviewTab === "pages" && (
              <>
                <div className="flex items-center gap-3 border-b border-gray-800 px-6 py-3">
                  <div className="relative flex-1">
                    <Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-500" />
                    <input
                      value={searchTerm}
                      onChange={(e) => setSearchTerm(e.target.value)}
                      placeholder="Search by URL or title"
                      className="w-full rounded-md border border-gray-800 bg-gray-900 pl-8 pr-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
                    />
                  </div>
                  <select
                    value={statusFilter}
                    onChange={(e) =>
                      setStatusFilter(e.target.value as "all" | "completed" | "partial")
                    }
                    className="rounded-md border border-gray-800 bg-gray-900 px-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
                  >
                    <option value="all">All status</option>
                    <option value="completed">Completed</option>
                    <option value="partial">Failed</option>
                  </select>
                  <select
                    value={reviewPageTypeFilter}
                    onChange={(e) => setReviewPageTypeFilter(e.target.value)}
                    className="rounded-md border border-gray-800 bg-gray-900 px-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
                  >
                    <option value="all">All types</option>
                    <option value="article">Article</option>
                    <option value="spa_application">SPA</option>
                    <option value="ecommerce_product">Product</option>
                    <option value="navigation_index">Index</option>
                    <option value="media_gallery">Gallery</option>
                    <option value="unknown">Unknown</option>
                  </select>
                  {archiver.selectedPageUrls.size > 0 && (
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => archiver.selectAllPages(filteredReviewPages.map((p) => p.url))}
                        className="text-[10px] text-crasp-400 hover:text-crasp-300"
                      >
                        Select all
                      </button>
                      <button
                        onClick={archiver.clearPageSelection}
                        className="text-[10px] text-gray-500 hover:text-gray-300"
                      >
                        Clear
                      </button>
                    </div>
                  )}
                </div>

                <ReviewPagesList
                  pages={filteredReviewPages}
                  loading={archiver.loadingArchived}
                  expandedUrl={reviewExpandedUrl}
                  onExpand={setReviewExpandedUrl}
                  selectedUrls={archiver.selectedPageUrls}
                  onToggleSelect={archiver.togglePageSelection}
                  onExportPage={handleExportPage}
                  selectedCrawl={selectedCrawl}
                  appStatus={archiver.appStatus}
                  onDeepFetchAndReload={() => {
                    if (selectedCrawlId) archiver.loadArchivedPages(selectedCrawlId);
                  }}
                  crawlId={selectedCrawl.crawl_id}
                />
              </>
            )}

            {reviewTab === "assets" && (
              <ReviewAssetsTab
                assets={archiver.assets}
                loading={archiver.loadingAssets}
                assetTypeFilter={reviewAssetTypeFilter}
                onAssetTypeFilterChange={setReviewAssetTypeFilter}
              />
            )}

            {reviewTab === "log" && (
              <ReviewLogTab
                logs={archiver.logs}
                allLogs={archiver.allLogs}
                logFilter={archiver.logFilter}
                setLogFilter={archiver.setLogFilter}
                crawlId={selectedCrawl.crawl_id}
                onExportLogs={archiver.exportLogs}
                onRevealInExplorer={archiver.revealInExplorer}
              />
            )}
          </section>
        )}

        {view === "export" && exportTarget && (
          <ExportPanel
            context={exportTarget.context === "selected_pages" ? "whole_crawl" : exportTarget.context}
            page={
              exportTarget.context === "single_page" ? exportTarget.page : null
            }
            crawlId={
              exportTarget.context === "whole_crawl" || exportTarget.context === "selected_pages"
                ? exportTarget.crawl.crawl_id
                : exportTarget.crawl?.crawl_id
            }
            crawlName={
              exportTarget.context === "whole_crawl" || exportTarget.context === "selected_pages"
                ? displaySessionName(exportTarget.crawl)
                : exportTarget.crawl
                  ? displaySessionName(exportTarget.crawl)
                  : null
            }
            pageCount={
              exportTarget.context === "whole_crawl"
                ? exportTarget.crawl.stats.archived
                : exportTarget.context === "selected_pages"
                  ? exportTarget.selectedUrls.length
                  : 1
            }
            selectedUrls={
              exportTarget.context === "selected_pages"
                ? exportTarget.selectedUrls
                : undefined
            }
            onCancel={() => {
              if (exportTarget.context === "whole_crawl" || exportTarget.context === "selected_pages") {
                setView("session-review");
                setSelectedCrawlId(exportTarget.crawl.crawl_id);
              } else if (exportTarget.crawl) {
                setView("session-review");
                setSelectedCrawlId(exportTarget.crawl.crawl_id);
              } else {
                setView("sessions");
              }
            }}
          />
        )}
      </main>

      <footer className="flex items-center justify-between border-t border-gray-800 px-4 py-2 text-[11px] text-gray-500">
        <button
          onClick={archiver.openDataFolder}
          className="flex items-center gap-1.5 hover:text-gray-300"
        >
          <FolderOpen className="h-3 w-3" />
          Open data folder
        </button>
        <button
          onClick={() => setShowLogs((prev) => !prev)}
          className="flex items-center gap-1.5 hover:text-gray-300"
        >
          <Terminal className="h-3 w-3" />
          Log
        </button>
      </footer>

      {showLogs && (
        <div className="fixed inset-0 z-40 flex justify-end bg-black/40">
          <div className="h-full w-full max-w-lg border-l border-gray-800 bg-gray-950">
            <div className="flex items-center justify-between border-b border-gray-800 px-4 py-2">
              <h2 className="text-xs font-semibold uppercase tracking-wider text-gray-400">
                Logs ({archiver.allLogs.length})
              </h2>
              <button
                onClick={() => setShowLogs(false)}
                className="text-gray-500 hover:text-gray-300"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <StructuredLogViewer
              logs={archiver.allLogs}
              logFilter={archiver.logFilter}
              setLogFilter={archiver.setLogFilter}
              clearLogs={archiver.clearLogs}
              autoScroll={archiver.autoScroll}
              setAutoScroll={archiver.setAutoScroll}
            />
          </div>
        </div>
      )}
    </div>
  );
}

function PageStatusIcon({ status }: { status: PageStatus | string }) {
  if (status === "Completed") {
    return <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-500" />;
  }
  if (status === "Pending") {
    return (
      <div className="h-4 w-4 shrink-0 rounded-full border-2 border-gray-700" />
    );
  }
  if (
    status === "Fetching" ||
    status === "Scraping" ||
    status === "Archiving"
  ) {
    return (
      <div className="h-4 w-4 shrink-0 rounded-full border-2 border-crasp-400 animate-pulse" />
    );
  }
  if (typeof status === "object" && "Failed" in status) {
    return <AlertTriangle className="h-4 w-4 shrink-0 text-red-500" />;
  }
  if (typeof status === "object" && "Skipped" in status) {
    return (
      <div className="h-4 w-4 shrink-0 rounded-full border-2 border-amber-600" />
    );
  }
  if (status === "Failed") {
    return <AlertTriangle className="h-4 w-4 shrink-0 text-red-500" />;
  }
  if (status === "Skipped") {
    return (
      <div className="h-4 w-4 shrink-0 rounded-full border-2 border-amber-600" />
    );
  }
  return <div className="h-4 w-4 shrink-0 rounded-full bg-gray-700" />;
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded border border-gray-800 bg-gray-900/50 p-2">
      <span className="text-[10px] uppercase tracking-wider text-gray-500">
        {label}
      </span>
      <p className="text-sm font-semibold text-gray-100">{value}</p>
    </div>
  );
}

function StructuredLogViewer({
  logs,
  logFilter,
  setLogFilter,
  clearLogs,
  autoScroll,
  setAutoScroll,
}: {
  logs: { timestamp: string; level: string; engine: string; message: string; crawl_id?: string | null }[];
  logFilter: { level: string; engine: string; search: string; crawl_id: string };
  setLogFilter: (f: { level: string; engine: string; search: string; crawl_id: string }) => void;
  clearLogs: () => void;
  autoScroll: boolean;
  setAutoScroll: (v: boolean) => void;
}) {
  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (autoScroll) {
      bottomRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [logs.length, autoScroll]);

  const levelColor = (level: string) => {
    switch (level) {
      case "error":
        return "bg-red-900/30 text-red-400";
      case "warn":
        return "bg-amber-900/30 text-amber-400";
      case "info":
      default:
        return "bg-blue-900/30 text-blue-400";
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-gray-800 px-4 py-2 gap-2">
        <div className="flex items-center gap-2">
          <select
            value={logFilter.level}
            onChange={(e) =>
              setLogFilter({ ...logFilter, level: e.target.value })
            }
            className="rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300"
          >
            <option value="all">All levels</option>
            <option value="info">Info</option>
            <option value="warn">Warn</option>
            <option value="error">Error</option>
          </select>
          <select
            value={logFilter.engine}
            onChange={(e) =>
              setLogFilter({ ...logFilter, engine: e.target.value })
            }
            className="rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300"
          >
            <option value="all">All engines</option>
            <option value="local">Local</option>
            <option value="cloud">Cloud</option>
            <option value="local-scrapy">Local Scrapy</option>
            <option value="system">System</option>
          </select>
          <input
            type="text"
            value={logFilter.crawl_id}
            onChange={(e) =>
              setLogFilter({ ...logFilter, crawl_id: e.target.value })
            }
            placeholder="Crawl ID"
            className="rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300 w-20 placeholder-gray-600 focus:border-crasp-500 focus:outline-none"
          />
          <div className="relative">
            <Search className="absolute left-1.5 top-1/2 -translate-y-1/2 h-3 w-3 text-gray-600" />
            <input
              type="text"
              value={logFilter.search}
              onChange={(e) =>
                setLogFilter({ ...logFilter, search: e.target.value })
              }
              placeholder="Search..."
              className="rounded border border-gray-700 bg-gray-800 pl-6 pr-2 py-0.5 text-[10px] text-gray-300 w-24 placeholder-gray-600 focus:border-crasp-500 focus:outline-none"
            />
          </div>
          <label className="flex items-center gap-1 text-[10px] text-gray-500 shrink-0">
            <input
              type="checkbox"
              checked={autoScroll}
              onChange={(e) => setAutoScroll(e.target.checked)}
              className="h-3 w-3 rounded border-gray-700 bg-gray-800 text-crasp-600 focus:ring-crasp-500"
            />
            Auto-scroll
          </label>
        </div>
        <button
          onClick={clearLogs}
          className="rounded-md bg-gray-800 px-2 py-1 text-[11px] text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors shrink-0"
        >
          Clear
        </button>
      </div>
      <div className="flex-1 overflow-auto p-2">
        {logs.length === 0 && (
          <div className="text-center text-xs text-gray-600 py-8">
            No log entries yet.
          </div>
        )}
        <div className="space-y-0.5">
          {logs.map((entry, i) => (
            <div
              key={`${entry.timestamp}-${i}`}
              className="flex items-start gap-2 rounded px-2 py-1 text-[11px] hover:bg-gray-900/50"
            >
              <span className="font-mono text-gray-600 shrink-0 text-[10px]">
                {new Date(entry.timestamp).toLocaleTimeString()}
              </span>
              <span
                className={`rounded px-1 py-0.5 font-medium text-[10px] shrink-0 ${levelColor(entry.level)}`}
              >
                {entry.level}
              </span>
              <span className="rounded px-1 py-0.5 bg-gray-800 text-gray-300 text-[10px] shrink-0">
                {entry.engine}
              </span>
              <span className="text-gray-300 break-all">{entry.message}</span>
            </div>
          ))}
          <div ref={bottomRef} />
        </div>
      </div>
    </div>
  );
}

function ReviewPagesList({
  pages,
  loading,
  expandedUrl,
  onExpand,
  selectedUrls,
  onToggleSelect,
  onExportPage,
  selectedCrawl: _selectedCrawl,
  appStatus,
  onDeepFetchAndReload,
  crawlId,
}: {
  pages: PageSummary[];
  loading: boolean;
  expandedUrl: string | null;
  onExpand: (url: string | null) => void;
  selectedUrls: Set<string>;
  onToggleSelect: (url: string) => void;
  onExportPage: (page: PageSummary) => void;
  selectedCrawl: CrawlSummary;
  appStatus: AppStatus | null;
  onDeepFetchAndReload: () => void;
  crawlId: string;
}) {
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: pages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 48,
    overscan: 20,
  });

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center text-gray-600">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }

  if (pages.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center text-gray-600">
        <Archive className="mb-3 h-12 w-12 opacity-30" />
        <p className="text-sm">No pages archived.</p>
      </div>
    );
  }

  return (
    <div ref={parentRef} className="flex-1 overflow-auto">
      <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const page = pages[virtualRow.index];
          const isExpanded = expandedUrl === page.url;
          return (
            <div
              key={page.url}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualRow.start}px)`,
              }}
            >
              <div className="flex items-center gap-2 px-4 py-2.5 hover:bg-gray-900/50 border-b border-gray-900">
                <input
                  type="checkbox"
                  checked={selectedUrls.has(page.url)}
                  onChange={() => onToggleSelect(page.url)}
                  onClick={(e) => e.stopPropagation()}
                  className="h-3.5 w-3.5 rounded border-gray-700 bg-gray-800 text-crasp-600 shrink-0"
                />
                <PageStatusIcon status={page.stage as PageStatus} />
                <div
                  className="min-w-0 flex-1 cursor-pointer"
                  onClick={() => onExpand(isExpanded ? null : page.url)}
                >
                  <p className="truncate text-sm text-gray-200">
                    {page.title || page.url}
                  </p>
                  <p className="truncate text-[11px] text-gray-600">
                    {page.url}
                  </p>
                </div>
                <div className="flex items-center gap-2 text-[11px] text-gray-500 shrink-0">
                  {pageTypeBadge(page.page_type)}
                  {extractionMethodBadge(page.extraction_method)}
                  {page.thin_content && (
                    <span className="inline-flex items-center gap-1 rounded bg-amber-900/20 px-1.5 py-0.5 text-amber-400">
                      <AlertTriangle className="h-2.5 w-2.5" />
                      thin
                    </span>
                  )}
                  <span className="text-gray-500">
                    {page.content_size > 1024
                      ? `${(page.content_size / 1024).toFixed(1)} KB`
                      : `${page.content_size} B`}
                  </span>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onExportPage(page);
                    }}
                    className="rounded-md bg-gray-800 px-2 py-1 text-[11px] text-gray-300 hover:bg-gray-700"
                  >
                    Export
                  </button>
                </div>
              </div>
              {isExpanded && (
                <div className="bg-gray-900/60 px-6 py-4 text-xs text-gray-400">
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <span className="text-gray-500">Author</span>
                      <p className="text-gray-300">{page.author || "-"}</p>
                    </div>
                    <div>
                      <span className="text-gray-500">Published</span>
                      <p className="text-gray-300">{page.published_date || "-"}</p>
                    </div>
                    <div>
                      <span className="text-gray-500">Confidence</span>
                      <p className="text-gray-300">
                        {page.extraction_confidence != null
                          ? `${(page.extraction_confidence * 100).toFixed(0)}%`
                          : "-"}
                      </p>
                    </div>
                    <div>
                      <span className="text-gray-500">Depth</span>
                      <p className="text-gray-300">{page.depth}</p>
                    </div>
                  </div>
                  {page.excerpt && (
                    <p className="mt-3 text-gray-300 italic">
                      {page.excerpt.slice(0, 300)}
                    </p>
                  )}
                  {page.thin_content && appStatus?.chrome_available && (
                    <button
                      onClick={async () => {
                        await invoke("deep_fetch_page", { url: page.url, crawlId });
                        onDeepFetchAndReload();
                      }}
                      className="mt-3 inline-flex items-center gap-2 rounded-md bg-purple-600/20 px-3 py-1.5 text-[11px] text-purple-300 hover:bg-purple-600/30"
                    >
                      <Monitor className="h-3 w-3" />
                      Deep Fetch
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ReviewAssetsTab({
  assets,
  loading,
  assetTypeFilter,
  onAssetTypeFilterChange,
}: {
  assets: AssetRow[];
  loading: boolean;
  assetTypeFilter: string;
  onAssetTypeFilterChange: (v: string) => void;
}) {
  const parentRef = useRef<HTMLDivElement>(null);
  const filtered = useMemo(
    () => assetTypeFilter === "all" ? assets : assets.filter((a) => a.asset_type === assetTypeFilter),
    [assets, assetTypeFilter],
  );
  const virtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 40,
    overscan: 20,
  });

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center text-gray-600">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }

  return (
    <>
      <div className="flex items-center gap-3 border-b border-gray-800 px-6 py-3">
        <select
          value={assetTypeFilter}
          onChange={(e) => onAssetTypeFilterChange(e.target.value)}
          className="rounded-md border border-gray-800 bg-gray-900 px-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
        >
          <option value="all">All types</option>
          <option value="image">Images</option>
          <option value="video">Videos</option>
          <option value="document">Documents</option>
        </select>
        <span className="text-xs text-gray-500">{filtered.length} assets</span>
        <button
          disabled
          title="Coming soon"
          className="ml-auto rounded-md bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-500 cursor-not-allowed"
        >
          <Download className="h-3 w-3 inline mr-1" />
          Download Selected (Coming soon)
        </button>
      </div>
      <div ref={parentRef} className="flex-1 overflow-auto">
        {filtered.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center text-gray-600">
            <Image className="mb-3 h-12 w-12 opacity-30" />
            <p className="text-sm">No assets found.</p>
          </div>
        ) : (
          <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const asset = filtered[virtualRow.index];
              return (
                <div
                  key={`${asset.src}-${asset.page_url}-${virtualRow.index}`}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                  className="flex items-center gap-3 px-6 py-2 hover:bg-gray-900/50 border-b border-gray-900 text-xs"
                >
                  <span className="w-16 shrink-0">
                    <span className={`rounded px-1.5 py-0.5 text-[9px] font-medium ${
                      asset.asset_type === "image" ? "bg-blue-900/30 text-blue-400" :
                      asset.asset_type === "video" ? "bg-purple-900/30 text-purple-400" :
                      "bg-emerald-900/30 text-emerald-400"
                    }`}>
                      {asset.asset_type}
                    </span>
                  </span>
                  <span className="truncate flex-1 text-gray-300" title={asset.src}>
                    {asset.src}
                  </span>
                  <span className="truncate w-32 text-gray-500 shrink-0" title={asset.alt_or_link_text || undefined}>
                    {asset.alt_or_link_text || "-"}
                  </span>
                  <span className="truncate w-40 text-gray-600 shrink-0" title={asset.page_url}>
                    {asset.page_url}
                  </span>
                  <span className="w-20 shrink-0 text-gray-600">
                    {asset.in_main_content ? "main" : "sidebar"}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </>
  );
}

function ReviewLogTab({
  logs: _logs,
  allLogs,
  logFilter,
  setLogFilter,
  crawlId,
  onExportLogs,
  onRevealInExplorer,
}: {
  logs: LogEntry[];
  allLogs: LogEntry[];
  logFilter: { level: string; engine: string; search: string; crawl_id: string };
  setLogFilter: (f: { level: string; engine: string; search: string; crawl_id: string }) => void;
  crawlId: string;
  onExportLogs: (logs: LogEntry[]) => Promise<string>;
  onRevealInExplorer: (path: string) => void;
}) {
  const [showAll, setShowAll] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportPath, setExportPath] = useState<string | null>(null);
  const parentRef = useRef<HTMLDivElement>(null);

  const effectiveFilter = useMemo(() => ({
    ...logFilter,
    crawl_id: showAll ? "" : crawlId,
  }), [logFilter, showAll, crawlId]);

  const displayLogs = useMemo(() => {
    return allLogs.filter((entry) => {
      if (effectiveFilter.level !== "all" && entry.level !== effectiveFilter.level) return false;
      if (effectiveFilter.engine !== "all" && entry.engine !== effectiveFilter.engine) return false;
      if (effectiveFilter.search && !entry.message.toLowerCase().includes(effectiveFilter.search.toLowerCase())) return false;
      if (effectiveFilter.crawl_id && entry.crawl_id !== effectiveFilter.crawl_id) return false;
      return true;
    });
  }, [allLogs, effectiveFilter]);

  const virtualizer = useVirtualizer({
    count: displayLogs.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 28,
    overscan: 50,
  });

  const levelColor = (level: string) => {
    switch (level) {
      case "error": return "bg-red-900/30 text-red-400";
      case "warn": return "bg-amber-900/30 text-amber-400";
      default: return "bg-blue-900/30 text-blue-400";
    }
  };

  const handleExport = useCallback(async () => {
    setExporting(true);
    try {
      const path = await onExportLogs(displayLogs);
      setExportPath(path);
    } catch (e) {
      console.error("Log export failed:", e);
    } finally {
      setExporting(false);
    }
  }, [onExportLogs, displayLogs]);

  return (
    <>
      <div className="flex items-center gap-3 border-b border-gray-800 px-6 py-3">
        <select
          value={logFilter.level}
          onChange={(e) => setLogFilter({ ...logFilter, level: e.target.value })}
          className="rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300"
        >
          <option value="all">All levels</option>
          <option value="info">Info</option>
          <option value="warn">Warn</option>
          <option value="error">Error</option>
        </select>
        <select
          value={logFilter.engine}
          onChange={(e) => setLogFilter({ ...logFilter, engine: e.target.value })}
          className="rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300"
        >
          <option value="all">All engines</option>
          <option value="local">Local</option>
          <option value="cloud">Cloud</option>
          <option value="local-scrapy">Local Scrapy</option>
          <option value="system">System</option>
        </select>
        <div className="relative">
          <Search className="absolute left-1.5 top-1/2 -translate-y-1/2 h-3 w-3 text-gray-600" />
          <input
            type="text"
            value={logFilter.search}
            onChange={(e) => setLogFilter({ ...logFilter, search: e.target.value })}
            placeholder="Search..."
            className="rounded border border-gray-700 bg-gray-800 pl-6 pr-2 py-0.5 text-[10px] text-gray-300 w-24 placeholder-gray-600 focus:border-crasp-500 focus:outline-none"
          />
        </div>
        <label className="flex items-center gap-1 text-[10px] text-gray-500 shrink-0">
          <input
            type="checkbox"
            checked={showAll}
            onChange={(e) => setShowAll(e.target.checked)}
            className="h-3 w-3 rounded border-gray-700 bg-gray-800 text-crasp-600"
          />
          All logs
        </label>
        <span className="text-[10px] text-gray-600">{displayLogs.length} entries</span>
        <button
          onClick={handleExport}
          disabled={exporting || displayLogs.length === 0}
          className="ml-auto rounded-md bg-gray-800 px-2 py-1 text-[11px] text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors disabled:opacity-50"
        >
          <FileDown className="h-3 w-3 inline mr-1" />
          {exporting ? "Exporting..." : "Export"}
        </button>
      </div>
      {exportPath && (
        <div className="mx-6 mt-2 rounded-md bg-emerald-900/20 p-2 text-[11px] text-emerald-400 flex items-center gap-2">
          <span className="truncate">Exported to: {exportPath}</span>
          <button
            onClick={() => onRevealInExplorer(exportPath)}
            className="shrink-0 text-emerald-300 hover:text-emerald-200"
          >
            Show
          </button>
          <button onClick={() => setExportPath(null)} className="shrink-0 text-gray-500 hover:text-gray-300">
            <X className="h-3 w-3" />
          </button>
        </div>
      )}
      <div ref={parentRef} className="flex-1 overflow-auto p-2">
        {displayLogs.length === 0 ? (
          <div className="text-center text-xs text-gray-600 py-8">No log entries.</div>
        ) : (
          <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const entry = displayLogs[virtualRow.index];
              return (
                <div
                  key={`${entry.timestamp}-${virtualRow.index}`}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                  className="flex items-start gap-2 rounded px-2 py-0.5 text-[11px] hover:bg-gray-900/50"
                >
                  <span className="font-mono text-gray-600 shrink-0 text-[10px]">
                    {new Date(entry.timestamp).toLocaleTimeString()}
                  </span>
                  <span className={`rounded px-1 py-0.5 font-medium text-[10px] shrink-0 ${levelColor(entry.level)}`}>
                    {entry.level}
                  </span>
                  <span className="rounded px-1 py-0.5 bg-gray-800 text-gray-300 text-[10px] shrink-0">
                    {entry.engine}
                  </span>
                  <span className="text-gray-300 break-all">{entry.message}</span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </>
  );
}

function InputField({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
  disabled = false,
  onBlur,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
  disabled?: boolean;
  onBlur?: () => void;
}) {
  return (
    <div>
      <label className="mb-1 block text-[11px] text-gray-500">{label}</label>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
        placeholder={placeholder}
        disabled={disabled}
        className="w-full rounded-md border border-gray-700 bg-gray-800 px-2.5 py-1.5 text-xs text-gray-200 placeholder-gray-600 focus:border-crasp-500 focus:outline-none disabled:opacity-50"
      />
    </div>
  );
}
