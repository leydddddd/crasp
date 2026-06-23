import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Play,
  Pause,
  Square,
  RotateCcw,
  Globe,
  CheckCircle2,
  XCircle,
  Database,
  Hash,
  Layers,
  ArrowDownToLine,
  Settings2,
  Activity,
  Cloud,
  Terminal,
  Archive,
  Loader2,
  Search,
  FolderOpen,
  X,
  AlertTriangle,
  FileDown,
  Zap,
  Monitor,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useArchiver } from "@/hooks/useArchiver";
import { ExportPanel } from "@/components/ExportPanel";
import type {
  ArchivedPage,
  PageStatus,
  ArchiveStatus,
  Engine,
  ServiceState,
  LogEntry,
  PageSummary,
  StorageSource,
  StorageUsed,
} from "@/types/archiver";
// Note: deprecated export_page and export_crawl_epub handlers removed in WI-32-E
import { storageUsedLabel } from "@/types/archiver";

const ENGINE_OPTIONS: { value: Engine; label: string; icon: React.ReactNode }[] = [
  { value: "local", label: "Local", icon: <Database className="h-3 w-3" /> },
  { value: "cloud", label: "Cloud", icon: <Cloud className="h-3 w-3" /> },
  { value: "local-scrapy", label: "Local Scrapy", icon: <Terminal className="h-3 w-3" /> },
];

function serviceStateColor(state: ServiceState | undefined): string {
  switch (state) {
    case "connected": return "bg-emerald-500";
    case "configured_unverified": return "bg-amber-500";
    case "unreachable": return "bg-red-500";
    case "not_configured": default: return "bg-gray-600";
  }
}

function serviceStateLabel(state: ServiceState | undefined, name: string, detail: string | null): string {
  switch (state) {
    case "connected": return `${name}: Connected${detail ? ` (${detail})` : ""}`;
    case "configured_unverified": return `${name}: Unverified`;
    case "unreachable": return `${name}: Unreachable${detail ? ` � ${detail}` : ""}`;
    case "not_configured": default: return `${name}: Not configured`;
  }
}

function sourceBadge(source: StorageSource): React.ReactNode {
  if (source === "Mongo") {
    return (
      <span className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-[9px] bg-emerald-900/30 text-emerald-400 font-medium">
        <Database className="h-2.5 w-2.5" />
        MongoDB
      </span>
    );
  }
  if (typeof source === "object" && "LocalFile" in source) {
    return (
      <span className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-[9px] bg-amber-900/30 text-amber-400 font-medium" title={source.LocalFile.path}>
        <FolderOpen className="h-2.5 w-2.5" />
        Local file
      </span>
    );
  }
  return null;
}

function extractionMethodBadge(method: string | null): React.ReactNode {
  if (!method) return null;
  switch (method) {
    case "readability":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-blue-900/30 text-blue-400 font-medium">readability</span>;
    case "css_selector":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-300 font-medium">css_selector</span>;
    case "zyte_autoextract":
      return <span className="inline-flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] bg-emerald-900/30 text-emerald-400 font-medium"><Zap className="h-2.5 w-2.5" />Zyte</span>;
    case "readability (browser)":
    case "css_selector (browser)":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-purple-900/30 text-purple-400 font-medium">browser</span>;
    case "failed":
    case "raw":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-amber-900/30 text-amber-400 font-medium">{method}</span>;
    default:
      return <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-400 font-medium">{method}</span>;
  }
}

function pageTypeBadge(pageType: string | null): React.ReactNode {
  if (!pageType) return null;
  switch (pageType) {
    case "article":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-blue-900/30 text-blue-400 font-medium">Article</span>;
    case "spa_application":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-purple-900/30 text-purple-400 font-medium">SPA</span>;
    case "ecommerce_product":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-emerald-900/30 text-emerald-400 font-medium">Product</span>;
    case "navigation_index":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-300 font-medium">Index</span>;
    case "media_gallery":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-orange-900/30 text-orange-400 font-medium">Gallery</span>;
    case "unknown":
      return <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-400 font-medium">Unknown</span>;
    default:
      return <span className="rounded px-1 py-0.5 text-[9px] bg-gray-700/50 text-gray-400 font-medium">{pageType}</span>;
  }
}

export function ArchiverDashboard() {
  const archiver = useArchiver();
  const [activeTab, setActiveTab] = useState<"config" | "logs" | "archive">("config");
  const [selectedPage, setSelectedPage] = useState<ArchivedPage | null>(null);
  const [selectedArchivePage, setSelectedArchivePage] = useState<PageSummary | null>(null);
  const [exportPanelOpen, setExportPanelOpen] = useState(false);
  const [exportPanelContext, setExportPanelContext] = useState<"single_page" | "whole_crawl">("single_page");
  const [exportPanelPage, setExportPanelPage] = useState<PageSummary | null>(null);
  const [exportPanelCrawlId, setExportPanelCrawlId] = useState<string | null>(null);
  const parentRef = useRef<HTMLDivElement>(null);

  const statusColor: Record<ArchiveStatus, string> = {
    idle: "bg-gray-600",
    crawling: "bg-crasp-600 animate-pulse",
    paused: "bg-amber-500",
    completed: "bg-emerald-500",
    cancelled: "bg-rose-500",
    error: "bg-red-500",
  };

  const statusLabel: Record<ArchiveStatus, string> = {
    idle: "Ready",
    crawling: "Crawling...",
    paused: "Paused",
    completed: "Completed",
    cancelled: "Cancelled",
    error: "Error",
  };

  const sortedPages = useMemo(() => {
    return [...archiver.pages].reverse();
  }, [archiver.pages]);

  const rowVirtualizer = useVirtualizer({
    count: sortedPages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 56,
    overscan: 5,
  });

  const handleStart = useCallback(() => {
    archiver.startCrawl();
  }, [archiver.startCrawl]);

  const isIdle = archiver.status === "idle";

  const openExportPanel = useCallback((context: "single_page" | "whole_crawl", page?: PageSummary | null, crawlId?: string | null) => {
    setExportPanelContext(context);
    setExportPanelPage(page || null);
    setExportPanelCrawlId(crawlId || null);
    setExportPanelOpen(true);
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col bg-gray-950 text-gray-100 overflow-hidden">
      {/* Header */}
      <header className="flex items-center justify-between border-b border-gray-800 px-4 py-2.5">
        <div className="flex items-center gap-3">
          <Database className="h-5 w-5 text-crasp-400" />
          <h1 className="text-lg font-semibold tracking-tight">Crasp</h1>
          <div
            className={`ml-3 flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${
              statusColor[archiver.status]
            } text-white`}
          >
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                archiver.status === "crawling" ? "animate-ping" : ""
              } bg-white`}
            />
            {statusLabel[archiver.status]}
          </div>
        </div>
        <div className="flex items-center gap-2">
          {archiver.status !== "crawling" && archiver.status !== "paused" && (
            <>
              <button
                onClick={() => setActiveTab("config")}
                className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                  activeTab === "config"
                    ? "bg-gray-700 text-gray-200"
                    : "bg-gray-800 text-gray-300 hover:bg-gray-700"
                }`}
              >
                <Settings2 className="h-3.5 w-3.5" />
                Config
              </button>
              <button
                onClick={() => setActiveTab("logs")}
                className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                  activeTab === "logs"
                    ? "bg-gray-700 text-gray-200"
                    : "bg-gray-800 text-gray-300 hover:bg-gray-700"
                }`}
              >
                <Terminal className="h-3.5 w-3.5" />
                Logs
              </button>
              <button
                onClick={() => {
                  setActiveTab("archive");
                  archiver.loadArchivedPages();
                }}
                className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                  activeTab === "archive"
                    ? "bg-gray-700 text-gray-200"
                    : "bg-gray-800 text-gray-300 hover:bg-gray-700"
                }`}
              >
                <Archive className="h-3.5 w-3.5" />
                Archive
              </button>
            </>
          )}
          {archiver.status === "crawling" && (
            <>
              <button
                onClick={archiver.pauseCrawl}
                className="flex items-center gap-1.5 rounded-md bg-amber-600/20 px-3 py-1.5 text-xs font-medium text-amber-400 hover:bg-amber-600/30 transition-colors"
              >
                <Pause className="h-3.5 w-3.5" />
                Pause
              </button>
              <button
                onClick={archiver.cancelCrawl}
                className="flex items-center gap-1.5 rounded-md bg-red-600/20 px-3 py-1.5 text-xs font-medium text-red-400 hover:bg-red-600/30 transition-colors"
              >
                <Square className="h-3.5 w-3.5" />
                Cancel
              </button>
            </>
          )}
          {archiver.status === "paused" && (
            <>
              <button
                onClick={archiver.resumeCrawl}
                className="flex items-center gap-1.5 rounded-md bg-emerald-600/20 px-3 py-1.5 text-xs font-medium text-emerald-400 hover:bg-emerald-600/30 transition-colors"
              >
                <Play className="h-3.5 w-3.5" />
                Resume
              </button>
              <button
                onClick={archiver.cancelCrawl}
                className="flex items-center gap-1.5 rounded-md bg-red-600/20 px-3 py-1.5 text-xs font-medium text-red-400 hover:bg-red-600/30 transition-colors"
              >
                <Square className="h-3.5 w-3.5" />
                Cancel
              </button>
            </>
          )}
          {(archiver.status === "completed" || archiver.status === "error" || archiver.status === "cancelled") && (
            <button
              onClick={archiver.resetCrawl}
              className="flex items-center gap-1.5 rounded-md bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-300 hover:bg-gray-700 transition-colors"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              Reset
            </button>
          )}
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden">
        {/* Left Panel - Config / Stats */}
        <aside
          className={`flex flex-col border-r border-gray-800 transition-all duration-300 ${
            activeTab === "config" || activeTab === "logs" || activeTab === "archive" ? "w-80" : "w-0"
          } overflow-hidden`}
        >
          {/* Crawl Config */}
          <div className="border-b border-gray-800 p-4">
            <h2 className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-gray-400">
              <Layers className="h-3.5 w-3.5" />
              Crawl Configuration
            </h2>
            <div className="space-y-3">
              {/* Engine selector */}
              <div>
                <label className="mb-1.5 block text-[11px] text-gray-500">Engine</label>
                <div className="flex rounded-md border border-gray-700 overflow-hidden">
                  {ENGINE_OPTIONS.map((opt) => (
                    <button
                      key={opt.value}
                      onClick={() => archiver.setEngine(opt.value)}
                      disabled={!isIdle}
                      className={`flex flex-1 items-center justify-center gap-1 px-2 py-1.5 text-[11px] font-medium transition-colors disabled:opacity-50 ${
                        archiver.engine === opt.value
                          ? "bg-crasp-600 text-white"
                          : "bg-gray-800 text-gray-400 hover:bg-gray-700"
                      }`}
                    >
                      {opt.icon}
                      {opt.label}
                    </button>
                  ))}
                </div>
              </div>

              {/* Cloud engine config */}
              {archiver.engine === "cloud" && (
                <div className="rounded-md border border-gray-800 bg-gray-900/50 p-3 space-y-2.5">
                  <div className="flex items-center gap-1.5 text-[11px]">
                    <span
                      className={`h-2 w-2 rounded-full ${serviceStateColor(archiver.appStatus?.zyte_state)}`}
                      title={serviceStateLabel(archiver.appStatus?.zyte_state, "Zyte", archiver.appStatus?.zyte_detail ?? null)}
                    />
                    <span className="text-gray-400">
                      {serviceStateLabel(archiver.appStatus?.zyte_state, "Zyte", archiver.appStatus?.zyte_detail ?? null)}
                    </span>
                  </div>
                  <InputField
                    label="Zyte API Key"
                    value={archiver.zyteApiKey}
                    onChange={archiver.setZyteApiKey}
                    placeholder="Enter API key..."
                    disabled={!isIdle}
                  />
                  <InputField
                    label="Project ID"
                    value={archiver.zyteProjectId}
                    onChange={archiver.setZyteProjectId}
                    placeholder="Enter project ID..."
                    disabled={!isIdle}
                    testBtn={{
                      label: "Test",
                      loading: archiver.testingZyte,
                      onClick: () => {
                        if (archiver.zyteApiKey && archiver.zyteProjectId) {
                          archiver.testZyteConnection(archiver.zyteApiKey, archiver.zyteProjectId);
                        }
                      },
                    }}
                  />
                </div>
              )}

              <InputField
                label="Seed URL"
                value={archiver.config.seed_url}
                onChange={(v) =>
                  archiver.setConfig((c) => ({ ...c, seed_url: v }))
                }
                placeholder="https://example.com"
                disabled={!isIdle}
              />
              <div className="grid grid-cols-2 gap-3">
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
                  disabled={!isIdle}
                />
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
                  disabled={!isIdle}
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <InputField
                  label="Concurrency"
                  value={String(archiver.config.concurrency)}
                  onChange={(v) =>
                    archiver.setConfig((c) => ({
                      ...c,
                      concurrency: Math.max(1, Math.min(16, parseInt(v) || 1)),
                    }))
                  }
                  type="number"
                  disabled={!isIdle}
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
                    disabled={!isIdle}
                    className="w-full rounded-md border border-gray-700 bg-gray-800 px-2.5 py-1.5 text-xs text-gray-200 focus:border-crasp-500 focus:outline-none disabled:opacity-50"
                  >
                    <option value="sha256">SHA-256</option>
                    <option value="md5">MD5</option>
                  </select>
                </div>
              </div>
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
                disabled={!isIdle}
              />
              <div className="flex items-center gap-2">
                <button
                  onClick={() =>
                    archiver.setConfig((c) => ({
                      ...c,
                      preserve_html: !c.preserve_html,
                    }))
                  }
                  disabled={!isIdle}
                  className={`relative h-4 w-8 rounded-full transition-colors disabled:opacity-50 ${
                    archiver.config.preserve_html
                      ? "bg-crasp-600"
                      : "bg-gray-700"
                  }`}
                >
                  <span
                    className={`absolute top-0.5 h-3 w-3 rounded-full bg-white transition-transform ${
                      archiver.config.preserve_html ? "left-[18px]" : "left-0.5"
                    }`}
                  />
                </button>
                <span className="text-xs text-gray-400">
                  {archiver.config.preserve_html
                    ? "Preserve Raw HTML"
                    : "Extract Text Only"}
                </span>
              </div>
            </div>

            {isIdle && (
              <button
                onClick={handleStart}
                disabled={!archiver.config.seed_url.trim()}
                className="mt-4 flex w-full items-center justify-center gap-2 rounded-md bg-crasp-600 px-4 py-2 text-sm font-semibold text-white hover:bg-crasp-500 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              >
                <Play className="h-4 w-4" />
                Start Archiving
              </button>
            )}
          </div>

          {/* Services Status */}
          <div className="border-b border-gray-800 px-4 py-3">
            <h2 className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-gray-400">
              <Activity className="h-3.5 w-3.5" />
              Services
            </h2>
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 text-[11px]">
                  <span
                    className={`h-2 w-2 rounded-full ${serviceStateColor(archiver.appStatus?.mongo_state)}`}
                    title={serviceStateLabel(archiver.appStatus?.mongo_state, "MongoDB", archiver.appStatus?.mongo_detail ?? null)}
                  />
                  <span className="text-gray-400">
                    {serviceStateLabel(archiver.appStatus?.mongo_state, "MongoDB", archiver.appStatus?.mongo_detail ?? null)}
                  </span>
                </div>
                <button
                  onClick={() => {
                    const uri = prompt("Mongo URI:", "mongodb://localhost:27017");
                    if (uri) archiver.testMongoConnection(uri);
                  }}
                  disabled={archiver.testingMongo}
                  className="rounded px-1.5 py-0.5 text-[10px] bg-gray-800 text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors disabled:opacity-50"
                >
                  {archiver.testingMongo ? <Loader2 className="h-3 w-3 animate-spin" /> : "Test"}
                </button>
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 text-[11px]">
                  <span
                    className={`h-2 w-2 rounded-full ${serviceStateColor(archiver.appStatus?.zyte_state)}`}
                    title={serviceStateLabel(archiver.appStatus?.zyte_state, "Zyte", archiver.appStatus?.zyte_detail ?? null)}
                  />
                  <span className="text-gray-400">
                    {serviceStateLabel(archiver.appStatus?.zyte_state, "Zyte", archiver.appStatus?.zyte_detail ?? null)}
                  </span>
                </div>
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 text-[11px]">
                  <span
                    className={`h-2 w-2 rounded-full ${archiver.appStatus?.chrome_available ? "bg-emerald-500" : "bg-gray-600"}`}
                    title={archiver.appStatus?.chrome_available ? "Chrome ?" : "No Chrome � deep fetch unavailable"}
                  />
                  <span className="text-gray-400">
                    {archiver.appStatus?.chrome_available ? "Chrome ?" : "No Chrome � deep fetch unavailable"}
                  </span>
                </div>
              </div>
            </div>
          </div>

          {/* Stats */}
          <div className="flex-1 overflow-auto p-4">
            <h2 className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-gray-400">
              <Activity className="h-3.5 w-3.5" />
              Live Statistics
            </h2>
            <div className="grid grid-cols-2 gap-2">
              <StatCard
                icon={<Globe className="h-4 w-4 text-crasp-400" />}
                label="Discovered"
                value={archiver.stats.discovered}
              />
              <StatCard
                icon={<ArrowDownToLine className="h-4 w-4 text-blue-400" />}
                label="Processing"
                value={archiver.progress.length}
              />
              <StatCard
                icon={<CheckCircle2 className="h-4 w-4 text-emerald-400" />}
                label="Archived"
                value={archiver.stats.completed}
              />
              <StatCard
                icon={<XCircle className="h-4 w-4 text-red-400" />}
                label="Failed"
                value={archiver.stats.failed}
              />
            </div>

            {/* Progress bar */}
            {archiver.config.max_pages > 0 && archiver.stats.discovered > 0 && (
              <div className="mt-4">
                <div className="mb-1 flex items-center justify-between text-[11px] text-gray-500">
                  <span>Progress</span>
                  <span>
                    {Math.min(
                      archiver.stats.completed,
                      archiver.config.max_pages,
                    )}{" "}
                    / {archiver.config.max_pages}
                  </span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-gray-800">
                  <div
                    className="h-full rounded-full bg-crasp-600 transition-all duration-300"
                    style={{
                      width: `${Math.min(
                        100,
                        (archiver.stats.completed / archiver.config.max_pages) *
                          100,
                      )}%`,
                    }}
                  />
                </div>
              </div>
            )}
          </div>
        </aside>

        {/* Main Area */}
        {activeTab === "logs" ? (
          <main className="flex flex-1 flex-col overflow-hidden">
            <StructuredLogViewer
              logs={archiver.logs}
              logFilter={archiver.logFilter}
              setLogFilter={archiver.setLogFilter}
              clearLogs={archiver.clearLogs}
              autoScroll={archiver.autoScroll}
              setAutoScroll={archiver.setAutoScroll}
            />
          </main>
        ) : activeTab === "archive" ? (
          <main className="flex flex-1 flex-col overflow-hidden">
            <ArchiveViewer
              pages={archiver.archivedPages}
              loading={archiver.loadingArchived}
              selectedPage={selectedArchivePage}
              onSelectPage={setSelectedArchivePage}
              onExportPage={(page) => openExportPanel("single_page", page)}
              onExportCrawl={(crawlId) => openExportPanel("whole_crawl", null, crawlId)}
              zyteAvailable={archiver.appStatus?.zyte_available === true}
              chromeAvailable={archiver.appStatus?.chrome_available === true}
            />
            <ExportPanel
              open={exportPanelOpen}
              onClose={() => setExportPanelOpen(false)}
              context={exportPanelContext}
              page={exportPanelPage}
              crawlId={exportPanelCrawlId}
            />
          </main>
        ) : (
        <main className="flex flex-1 flex-col overflow-hidden">
          {/* Error bar */}
          {archiver.error && (
            <div className="border-b border-red-900/50 bg-red-950/50 px-4 py-2 text-xs text-red-400">
              {archiver.error}
            </div>
          )}

          {/* WI-30-A: Post-crawl summary panel */}
          {archiver.crawlSummary && (
            <CrawlSummaryPanel
              summary={archiver.crawlSummary}
              onDismiss={archiver.dismissSummary}
              onOpenDataFolder={archiver.openDataFolder}
              onViewFailed={() => {
                setActiveTab("archive");
                archiver.loadArchivedPages();
              }}
            />
          )}

          {/* Active progress strip with real stage names */}
          {archiver.progress.length > 0 && (
            <div className="flex items-center gap-2 overflow-x-auto border-b border-gray-800 px-4 py-2">
              <span className="shrink-0 text-[11px] font-medium text-gray-500 uppercase tracking-wider">
                Live:
              </span>
              {archiver.progress.slice(0, 8).map((p, index) => {
                const stageLabel = archiver.getPageStageLabel(p.url);
                const isPersist = stageLabel?.startsWith("Persist");
                const isFailed = stageLabel?.startsWith("Failed");
                return (
                  <span
                    key={`${p.url}-${index}`}
                    className="shrink-0 flex items-center gap-1 rounded-full bg-gray-800 px-2 py-0.5 text-[11px]"
                  >
                    <span className={`h-1.5 w-1.5 rounded-full ${isFailed ? "bg-red-400" : isPersist ? "bg-emerald-400" : "bg-crasp-400 animate-pulse"}`} />
                    <span className="max-w-[120px] truncate text-gray-400">
                      {p.url.replace(/^https?:\/\//, "").slice(0, 30)}
                    </span>
                    <span
                      className={`text-[10px] ${
                        isFailed ? "text-red-400" : isPersist ? "text-emerald-400" : "text-crasp-400"
                      }`}
                    >
                      {stageLabel || p.status}
                    </span>
                  </span>
                );
              })}
              {archiver.progress.length > 8 && (
                <span className="shrink-0 text-[11px] text-gray-600">
                  +{archiver.progress.length - 8} more
                </span>
              )}
            </div>
          )}

          {/* Page List */}
          <div ref={parentRef} className="flex-1 overflow-auto">
            {sortedPages.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center text-gray-600">
                <Globe className="mb-3 h-12 w-12 opacity-30" />
                <p className="text-sm">No pages archived yet</p>
                <p className="text-xs text-gray-700">
                  Configure a seed URL and start archiving
                </p>
              </div>
            ) : (
              <div
                style={{
                  height: `${rowVirtualizer.getTotalSize()}px`,
                  width: "100%",
                  position: "relative",
                }}
              >
                {rowVirtualizer.getVirtualItems().map((virtualRow) => {
                  const page = sortedPages[virtualRow.index];
                  const stageLabel = archiver.getPageStageLabel(page.url);
                  return (
                    <div
                      key={virtualRow.key}
                      style={{
                        position: "absolute",
                        top: 0,
                        left: 0,
                        width: "100%",
                        height: `${virtualRow.size}px`,
                        transform: `translateY(${virtualRow.start}px)`,
                      }}
                      className="flex items-center gap-3 border-b border-gray-800/50 px-4 hover:bg-gray-900/50 cursor-pointer transition-colors"
                      onClick={() =>
                        setSelectedPage(
                          selectedPage?.url === page.url ? null : page,
                        )
                      }
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
                      {stageLabel && (
                        <span className="shrink-0 max-w-[160px] truncate text-[10px] text-crasp-400/80" title={stageLabel}>
                          {stageLabel}
                        </span>
                      )}
                      <span className="shrink-0 text-[11px] text-gray-600">
                        Depth {page.depth}
                      </span>
                      <span className="shrink-0 text-[11px] text-gray-600">
                        {page.discovered_links} links
                      </span>
                      {page.hash && (
                        <span className="hidden shrink-0 items-center gap-1 text-[11px] text-crasp-400/60 xl:flex">
                          <Hash className="h-3 w-3" />
                          {page.hash.slice(0, 12)}
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Detail drawer */}
          {selectedPage && (
            <DetailDrawer
              page={selectedPage}
              stageLabel={archiver.getPageStageLabel(selectedPage.url)}
              onClose={() => setSelectedPage(null)}
            />
          )}

          {/* WI-30-B: Persistent footer with Open data folder link */}
          <div className="flex items-center justify-between border-t border-gray-800 px-4 py-1.5">
            <button
              onClick={archiver.openDataFolder}
              className="flex items-center gap-1.5 text-[11px] text-gray-500 hover:text-gray-300 transition-colors"
            >
              <FolderOpen className="h-3 w-3" />
              Open data folder
            </button>
            <span className="text-[10px] text-gray-700">
              {archiver.stats.completed} archived
              {archiver.stats.failed > 0 && ` � ${archiver.stats.failed} failed`}
            </span>
          </div>
        </main>
        )}
      </div>
    </div>
  );
}

function PageStatusIcon({ status }: { status: PageStatus }) {
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
    return <XCircle className="h-4 w-4 shrink-0 text-red-500" />;
  }
  if (typeof status === "object" && "Skipped" in status) {
    return (
      <div className="h-4 w-4 shrink-0 rounded-full border-2 border-amber-600" />
    );
  }
  const _exhaustive: never = status;
  void _exhaustive;
  return <div className="h-4 w-4 shrink-0 rounded-full bg-gray-700" />;
}

function StatCard({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: number;
}) {
  return (
    <div className="rounded-lg border border-gray-800 bg-gray-900/50 p-2.5">
      <div className="flex items-center gap-1.5 mb-1">
        {icon}
        <span className="text-[11px] text-gray-500 uppercase tracking-wider">
          {label}
        </span>
      </div>
      <p className="text-xl font-bold text-gray-100">{value}</p>
    </div>
  );
}

function DetailDrawer({
  page,
  stageLabel,
  onClose,
}: {
  page: ArchivedPage;
  stageLabel: string | null;
  onClose: () => void;
}) {
  return (
    <div className="border-t border-gray-800 bg-gray-900">
      <div className="flex items-center justify-between border-b border-gray-800/50 px-4 py-2">
        <h3 className="text-sm font-semibold text-gray-200 truncate flex-1 mr-4">
          {page.title}
        </h3>
        <button
          onClick={onClose}
          className="text-xs text-gray-500 hover:text-gray-300 transition-colors"
        >
          Close
        </button>
      </div>
      <div className="grid grid-cols-3 gap-4 p-4 text-xs">
        <div>
          <span className="text-gray-500">URL</span>
          <p className="mt-0.5 truncate text-gray-300">{page.url}</p>
        </div>
        <div>
          <span className="text-gray-500">Hash ({page.hash_algorithm})</span>
          <p className="mt-0.5 font-mono text-gray-300 truncate">
            {page.hash || "N/A"}
          </p>
        </div>
        <div>
          <span className="text-gray-500">Timestamp</span>
          <p className="mt-0.5 text-gray-300">
            {page.timestamp ? new Date(page.timestamp).toLocaleString() : "\u2014"}
          </p>
        </div>
      </div>
      {(page.author || page.published_date || page.reading_time_minutes || page.excerpt || page.thin_content) && (
        <div className="grid grid-cols-3 gap-4 px-4 pb-4 text-xs">
          {page.author && (
            <div>
              <span className="text-gray-500">Author</span>
              <p className="mt-0.5 text-gray-300">{page.author}</p>
            </div>
          )}
          {page.published_date && (
            <div>
              <span className="text-gray-500">Published</span>
              <p className="mt-0.5 text-gray-300">{page.published_date}</p>
            </div>
          )}
          {page.reading_time_minutes != null && page.reading_time_minutes > 0 && (
            <div>
              <span className="text-gray-500">Reading time</span>
              <p className="mt-0.5 text-gray-300">{page.reading_time_minutes} min</p>
            </div>
          )}
        </div>
      )}
      {page.excerpt && (
        <div className="px-4 pb-3 text-xs">
          <span className="text-gray-500">Excerpt</span>
          <p className="mt-0.5 text-gray-400 italic">{page.excerpt}</p>
        </div>
      )}
      {page.thin_content && (
        <div className="mx-4 mb-3 flex items-center gap-2">
          <div className="flex items-center gap-1.5 rounded-md bg-amber-900/20 px-2.5 py-1.5 text-xs text-amber-400 border border-amber-800/40">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            <span>Thin content � JS rendering may be needed</span>
          </div>
          {page.deep_fetched && (
            <span className="inline-flex items-center gap-1 rounded-md bg-emerald-900/20 px-2.5 py-1.5 text-xs text-emerald-400 border border-emerald-800/40">
              <Monitor className="h-3.5 w-3.5" />
              Deep Fetched
            </span>
          )}
        </div>
      )}
      {(page.extraction_method || page.page_type) && (
        <div className="mx-4 mb-3 flex items-center gap-2 text-xs">
          <span className="text-gray-500">Extraction:</span>
          {page.page_type && pageTypeBadge(page.page_type)}
          {page.extraction_method && extractionMethodBadge(page.extraction_method)}
          {page.extraction_confidence != null && (
            <span className="text-gray-600">confidence: {(page.extraction_confidence * 100).toFixed(0)}%</span>
          )}
        </div>
      )}
      {stageLabel && (
        <div className="border-t border-gray-800/50 px-4 py-2 text-xs">
          <span className="text-gray-500">Stage:</span>{" "}
          <span className="text-crasp-400">{stageLabel}</span>
        </div>
      )}
      <div className="border-t border-gray-800/50 px-4 py-3">
        <span className="text-[11px] text-gray-500 uppercase tracking-wider">
          Content Preview
        </span>
        <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-all text-[11px] text-gray-400 font-mono bg-gray-950 rounded-md p-3 border border-gray-800">
          {page.content !== null && page.content !== undefined && page.content.trim().length > 0 ? (
            <>
              {page.content.slice(0, 2000)}
              {page.content.length > 2000 ? "\n\n... (truncated)" : ""}
            </>
          ) : (
            <span className="text-gray-500 italic">No content captured.</span>
          )}
        </pre>
      </div>
    </div>
  );
}

// WI-30-A: Post-crawl summary panel
function CrawlSummaryPanel({
  summary,
  onDismiss,
  onOpenDataFolder,
  onViewFailed,
}: {
  summary: {
    pages_archived: number;
    pages_completed: number;
    pages_failed: number;
    pages_skipped: number;
    cancelled: boolean;
    crawl_id: string;
    storage_used: StorageUsed | null;
    deep_fetched_count: number;
  };
  onDismiss: () => void;
  onOpenDataFolder: () => void;
  onViewFailed: () => void;
}) {
  const storageLabel = summary.storage_used ? storageUsedLabel(summary.storage_used) : "No storage recorded";

  return (
    <div className="border-b border-crasp-800/50 bg-crasp-950/30 px-4 py-3">
      <div className="flex items-start justify-between mb-2">
        <h3 className="text-sm font-semibold text-gray-200">
          Crawl Complete
        </h3>
        <button
          onClick={onDismiss}
          className="text-gray-500 hover:text-gray-300 transition-colors"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <div className="grid grid-cols-4 gap-3 mb-3 text-xs">
        <div className="rounded border border-gray-800 bg-gray-900/50 p-2">
          <span className="text-gray-500 text-[10px] uppercase tracking-wider">Completed</span>
          <p className="text-lg font-bold text-emerald-400">{summary.pages_completed}</p>
        </div>
        <div className="rounded border border-gray-800 bg-gray-900/50 p-2">
          <span className="text-gray-500 text-[10px] uppercase tracking-wider">Failed</span>
          <p className="text-lg font-bold text-red-400">{summary.pages_failed}</p>
        </div>
        <div className="rounded border border-gray-800 bg-gray-900/50 p-2">
          <span className="text-gray-500 text-[10px] uppercase tracking-wider">Skipped</span>
          <p className="text-lg font-bold text-amber-400">{summary.pages_skipped}</p>
        </div>
        <div className="rounded border border-gray-800 bg-gray-900/50 p-2">
          <span className="text-gray-500 text-[10px] uppercase tracking-wider">Total</span>
          <p className="text-lg font-bold text-gray-200">{summary.pages_archived}</p>
        </div>
      </div>
      <div className="mb-3 text-xs">
        <span className="text-gray-500">Data stored in:</span>{" "}
        <span className="text-crasp-400 font-medium">{storageLabel}</span>
        {summary.deep_fetched_count > 0 && (
          <>
            {" "}&middot;{" "}
            <span className="text-emerald-400">Auto deep-fetched: {summary.deep_fetched_count} pages (headless browser)</span>
          </>
        )}
      </div>
      <div className="flex items-center gap-2">
        <button
          onClick={onOpenDataFolder}
          className="flex items-center gap-1.5 rounded-md bg-crasp-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-crasp-500 transition-colors"
        >
          <FolderOpen className="h-3 w-3" />
          Open data location
        </button>
        {summary.pages_failed > 0 && (
          <button
            onClick={onViewFailed}
            className="flex items-center gap-1.5 rounded-md bg-red-600/20 px-3 py-1.5 text-xs font-medium text-red-400 hover:bg-red-600/30 transition-colors"
          >
            <AlertTriangle className="h-3 w-3" />
            View {summary.pages_failed} failed
          </button>
        )}
        <button
          onClick={onDismiss}
          className="flex items-center gap-1.5 rounded-md bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors ml-auto"
        >
          Dismiss
        </button>
      </div>
    </div>
  );
}

// WI-27: Structured Log Viewer with level/engine filters
function StructuredLogViewer({
  logs,
  logFilter,
  setLogFilter,
  clearLogs,
  autoScroll,
  setAutoScroll,
}: {
  logs: LogEntry[];
  logFilter: { level: string; engine: string; search: string };
  setLogFilter: (f: { level: string; engine: string; search: string }) => void;
  clearLogs: () => void;
  autoScroll: boolean;
  setAutoScroll: (v: boolean) => void;
}) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (autoScroll) {
      bottomRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [logs.length, autoScroll]);

  const levelColor = (level: string) => {
    switch (level) {
      case "error": return "bg-red-900/30 text-red-400";
      case "warn": return "bg-amber-900/30 text-amber-400";
      case "info": default: return "bg-blue-900/30 text-blue-400";
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-gray-800 px-4 py-2 gap-2">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-gray-400 shrink-0">
          Logs ({logs.length})
        </h2>
        <div className="flex items-center gap-2 flex-1 justify-end">
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
              checked={autoScroll}
              onChange={(e) => setAutoScroll(e.target.checked)}
              className="h-3 w-3 rounded border-gray-700 bg-gray-800 text-crasp-600 focus:ring-crasp-500"
            />
            Auto-scroll
          </label>
          <button
            onClick={clearLogs}
            className="rounded-md bg-gray-800 px-2 py-1 text-[11px] text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors shrink-0"
          >
            Clear
          </button>
        </div>
      </div>
      <div className="flex-1 overflow-auto p-2">
        {logs.length === 0 && (
          <div className="text-center text-xs text-gray-600 py-8">No log entries yet.</div>
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
              <span className={`rounded px-1 py-0.5 font-medium text-[10px] shrink-0 ${levelColor(entry.level)}`}>
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

// WI-28/WI-29-A/WI-30-C: Archive Viewer with source labels, content preview, and dual-source export
function ArchiveViewer({
  pages,
  loading,
  selectedPage,
  onSelectPage,
  onExportPage,
  onExportCrawl,
  zyteAvailable,
  chromeAvailable,
}: {
  pages: PageSummary[];
  loading: boolean;
  selectedPage: PageSummary | null;
  onSelectPage: (p: PageSummary | null) => void;
  onExportPage: (page: PageSummary) => void;
  onExportCrawl: (crawlId: string) => void;
  zyteAvailable: boolean;
  chromeAvailable: boolean;
}) {
  const [previewPage, setPreviewPage] = useState<PageSummary | null>(null);
  const [previewContent, setPreviewContent] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [deepFetchingUrl, setDeepFetchingUrl] = useState<string | null>(null);

  const handleDeepFetch = useCallback(async (page: PageSummary) => {
    setDeepFetchingUrl(page.url);
    try {
      await invoke("deep_fetch_page", {
        url: page.url,
        crawlId: page.source === "Mongo" ? "" : "",
      });
    } catch (e) {
      console.error("Deep fetch failed:", e);
    } finally {
      setDeepFetchingUrl(null);
    }
  }, []);

  const handlePreview = useCallback(async (page: PageSummary) => {
    if (previewPage?.url === page.url) {
      setPreviewPage(null);
      setPreviewContent(null);
      return;
    }
    setPreviewPage(page);
    setPreviewLoading(true);

    if (page.content_preview) {
      setPreviewContent(page.content_preview);
      setPreviewLoading(false);
      return;
    }

    try {
      const source = page.source === "Mongo"
        ? "Mongo"
        : { LocalFile: { path: (page.source as { LocalFile: { path: string } }).LocalFile.path } };
      const content = await invoke<string | null>("get_page_content", {
        url: page.url,
        source,
        crawlId: null,
      });
      setPreviewContent(content || "No content captured.");
    } catch {
      setPreviewContent("Failed to load content.");
    } finally {
      setPreviewLoading(false);
    }
  }, [previewPage]);

  if (loading && pages.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-gray-600">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }

  if (pages.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center text-gray-600">
        <Archive className="mb-3 h-12 w-12 opacity-30" />
        <p className="text-sm">No archived crawls found.</p>
        <p className="text-xs text-gray-700">Start a crawl to see results here.</p>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-gray-800 px-4 py-2">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-gray-400">
          Archived Pages ({pages.length})
          {assetCounts.images > 0 && (
            <span className="ml-2 text-[10px] font-normal text-gray-600">
              {assetCounts.images} images, {assetCounts.documents} documents across {assetCounts.pagesWithAssets} pages
            </span>
          )}
        </h2>
        {crawlIds.size > 0 && (
          <div className="flex items-center gap-1.5">
            {Array.from(crawlIds.entries())
              .filter(([, count]) => count > 1)
              .map(([crawlId, count]) => (
                <button
                  key={crawlId}
                  onClick={() => onExportCrawl(crawlId)}
                  className="flex items-center gap-1 rounded-md bg-crasp-600/20 px-2.5 py-1 text-[10px] font-medium text-crasp-400 hover:bg-crasp-600/30 transition-colors"
                >
                  <FileDown className="h-3 w-3" />
                  Export ({count} pages)
                </button>
              ))}
          </div>
        )}
      </div>
      <div className="flex-1 overflow-auto">
        <table className="w-full text-xs">
          <thead className="sticky top-0 bg-gray-950 text-gray-500 text-[10px] uppercase tracking-wider">
            <tr>
              <th className="px-3 py-2 text-left">Title</th>
              <th className="px-3 py-2 text-left">URL</th>
              <th className="px-3 py-2 text-right">Depth</th>
              <th className="px-3 py-2 text-left">Status</th>
              <th className="px-3 py-2 text-left">Source</th>
              <th className="px-3 py-2 text-left">Method</th>
              <th className="px-3 py-2 text-right">Size</th>
              <th className="px-3 py-2 text-left">Timestamp</th>
              <th className="px-3 py-2 text-right">Export</th>
            </tr>
          </thead>
          <tbody>
            {pages.map((page) => (
              <tr
                key={`${page.url}-${page.source}`}
                className={`border-t border-gray-800/50 hover:bg-gray-900/50 cursor-pointer ${
                  selectedPage?.url === page.url ? "bg-gray-900" : ""
                }`}
                onClick={() => onSelectPage(selectedPage?.url === page.url ? null : page)}
              >
                <td className="px-3 py-2 truncate max-w-[140px]">{page.title || "\u2014"}</td>
                <td className="px-3 py-2 truncate max-w-[180px] text-gray-400">{page.url}</td>
                <td className="px-3 py-2 text-right text-gray-500">{page.depth}</td>
                <td className="px-3 py-2">
                  <span className={`rounded px-1 py-0.5 text-[10px] ${
                    page.stage === "Completed" ? "bg-emerald-900/30 text-emerald-400" :
                    page.stage === "Failed" ? "bg-red-900/30 text-red-400" :
                    "bg-gray-800 text-gray-300"
                  }`}>
                    {page.stage}
                    {page.status_reason ? `: ${page.status_reason}` : ""}
                  </span>
                  {page.thin_content && (
                    <span className="ml-1 inline-flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] bg-amber-900/20 text-amber-400 border border-amber-800/30">
                      <AlertTriangle className="h-2.5 w-2.5" />thin
                    </span>
                  )}
                </td>
                <td className="px-3 py-2">
                  {sourceBadge(page.source)}
                </td>
                <td className="px-3 py-2">
                  <div className="flex items-center gap-1">
                    {extractionMethodBadge(page.extraction_method)}
                    {page.deep_fetched && (
                      <span className="inline-flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] bg-purple-900/20 text-purple-400">
                        <Monitor className="h-2.5 w-2.5" />
                      </span>
                    )}
                  </div>
                </td>
                <td className="px-3 py-2 text-right text-gray-500">
                  {page.content_size > 1024 ? `${(page.content_size / 1024).toFixed(1)} KB` : `${page.content_size} B`}
                </td>
                <td className="px-3 py-2 text-gray-500">
                  {page.timestamp ? new Date(page.timestamp).toLocaleString() : "\u2014"}
                </td>
                <td className="px-3 py-2 text-right">
                  <div className="flex items-center justify-end gap-1">
                    {page.thin_content && (zyteAvailable || chromeAvailable) && !page.deep_fetched && (
                      <button
                        onClick={(e) => { e.stopPropagation(); handleDeepFetch(page); }}
                        disabled={deepFetchingUrl === page.url}
                        className="rounded px-1 py-0.5 text-[10px] bg-purple-600/20 text-purple-400 hover:bg-purple-600/30 transition-colors disabled:opacity-50"
                        title={chromeAvailable ? "Deep fetch with headless browser" : "Deep fetch with Zyte browser rendering"}
                      >
                        {deepFetchingUrl === page.url ? <Loader2 className="h-3 w-3 animate-spin" /> : <Zap className="h-3 w-3" />}
                      </button>
                    )}
                    <button
                      onClick={(e) => { e.stopPropagation(); handlePreview(page); }}
                      className="rounded px-1 py-0.5 text-[10px] bg-gray-800 text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors"
                      title="Preview content"
                    >
                      <Search className="h-3 w-3" />
                    </button>
                    <button
                      onClick={(e) => { e.stopPropagation(); onExportPage(page); }}
                      className="rounded px-1 py-0.5 text-[10px] bg-gray-800 text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors"
                    >
                      Export
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* WI-30-C: Content preview drawer */}
      {previewPage && (
        <div className="border-t border-gray-800 bg-gray-900">
          <div className="flex items-center justify-between border-b border-gray-800/50 px-4 py-2">
            <h3 className="text-sm font-semibold text-gray-200 truncate flex-1 mr-4">
              Content Preview � {previewPage.title || previewPage.url}
            </h3>
            <div className="flex items-center gap-2">
              {sourceBadge(previewPage.source)}
              <button
                onClick={() => { setPreviewPage(null); setPreviewContent(null); }}
                className="text-xs text-gray-500 hover:text-gray-300 transition-colors"
              >
                Close
              </button>
            </div>
          </div>
          <div className="p-4">
            {(previewPage.page_type || previewPage.extraction_method) && (
              <div className="mb-3 flex flex-wrap items-center gap-2 text-[11px]">
                {pageTypeBadge(previewPage.page_type)}
                {extractionMethodBadge(previewPage.extraction_method)}
                {previewPage.extraction_confidence != null && (
                  <span className="text-gray-600">confidence: {(previewPage.extraction_confidence * 100).toFixed(0)}%</span>
                )}
              </div>
            )}
            {(previewPage.author || previewPage.published_date || (previewPage.reading_time_minutes != null && previewPage.reading_time_minutes > 0) || previewPage.thin_content) && (
              <div className="mb-3 flex flex-wrap items-center gap-3 text-[11px]">
                {previewPage.author && (
                  <span className="text-gray-500">Author: <span className="text-gray-300">{previewPage.author}</span></span>
                )}
                {previewPage.published_date && (
                  <span className="text-gray-500">Published: <span className="text-gray-300">{previewPage.published_date}</span></span>
                )}
                {previewPage.reading_time_minutes != null && previewPage.reading_time_minutes > 0 && (
                  <span className="text-gray-500">Reading time: <span className="text-gray-300">{previewPage.reading_time_minutes} min</span></span>
                )}
                {previewPage.excerpt && (
                  <span className="text-gray-500" title={previewPage.excerpt}>Excerpt: <span className="text-gray-400 italic truncate max-w-[200px] inline-block align-bottom">{previewPage.excerpt}</span></span>
                )}
                {previewPage.thin_content && (
                  <span className="inline-flex items-center gap-1 rounded bg-amber-900/20 px-1.5 py-0.5 text-amber-400 border border-amber-800/30">
                    <AlertTriangle className="h-2.5 w-2.5" />Thin content
                  </span>
                )}
              </div>
            )}
            {previewLoading ? (
              <div className="flex items-center justify-center py-6 text-gray-500">
                <Loader2 className="h-5 w-5 animate-spin mr-2" />
                Loading content...
              </div>
            ) : (
              <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all text-[11px] text-gray-400 font-mono bg-gray-950 rounded-md p-3 border border-gray-800">
                {previewContent ? (
                  <>
                    {previewContent.slice(0, 500)}
                    {previewContent.length > 500 ? "\n\n... (truncated preview)" : ""}
                  </>
                ) : (
                  <span className="text-gray-500 italic">No content captured.</span>
                )}
              </pre>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function InputField({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
  disabled = false,
  testBtn,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
  disabled?: boolean;
  testBtn?: { label: string; loading: boolean; onClick: () => void };
}) {
  return (
    <div>
      <label className="mb-1 block text-[11px] text-gray-500">{label}</label>
      <div className="flex gap-1.5">
        <input
          type={type}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          className="flex-1 min-w-0 rounded-md border border-gray-700 bg-gray-800 px-2.5 py-1.5 text-xs text-gray-200 placeholder-gray-600 focus:border-crasp-500 focus:outline-none disabled:opacity-50"
        />
        {testBtn && (
          <button
            onClick={testBtn.onClick}
            disabled={testBtn.loading}
            className="shrink-0 rounded-md bg-crasp-600 px-2 py-1.5 text-[11px] font-medium text-white hover:bg-crasp-500 transition-colors disabled:opacity-50"
          >
            {testBtn.loading ? <Loader2 className="h-3 w-3 animate-spin" /> : testBtn.label}
          </button>
        )}
      </div>
    </div>
  );
}
