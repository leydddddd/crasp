import { useCallback, useMemo, useRef, useState } from "react";
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
} from "lucide-react";
import { useArchiver } from "@/hooks/useArchiver";
import type { ArchivedPage, PageStatus, ArchiveStatus } from "@/types/archiver";

export function ArchiverDashboard() {
  const archiver = useArchiver();
  const [configOpen, setConfigOpen] = useState(true);
  const [selectedPage, setSelectedPage] = useState<ArchivedPage | null>(null);
  const parentRef = useRef<HTMLDivElement>(null);

  const statusColor: Record<ArchiveStatus, string> = {
    idle: "bg-gray-600",
    crawling: "bg-crasp-600 animate-pulse",
    paused: "bg-amber-500",
    completed: "bg-emerald-500",
    error: "bg-red-500",
  };

  const statusLabel: Record<ArchiveStatus, string> = {
    idle: "Ready",
    crawling: "Crawling...",
    paused: "Paused",
    completed: "Completed",
    error: "Error",
  };

  const sortedPages = useMemo(() => {
    return [...archiver.pages].reverse();
  }, [archiver.pages]);

  const rowVirtualizer = useVirtualizer({
    count: sortedPages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 56,
    overscan: 20,
  });

  const handleStart = useCallback(() => {
    archiver.startCrawl();
  }, [archiver.startCrawl]);

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
            <button
              onClick={() => setConfigOpen(!configOpen)}
              className="flex items-center gap-1.5 rounded-md bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-300 hover:bg-gray-700 transition-colors"
            >
              <Settings2 className="h-3.5 w-3.5" />
              {configOpen ? "Hide Config" : "Config"}
            </button>
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
          {(archiver.status === "completed" || archiver.status === "error") && (
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
            configOpen ? "w-80" : "w-0"
          } overflow-hidden`}
        >
          {/* Crawl Config */}
          <div className="border-b border-gray-800 p-4">
            <h2 className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-gray-400">
              <Layers className="h-3.5 w-3.5" />
              Crawl Configuration
            </h2>
            <div className="space-y-3">
              <InputField
                label="Seed URL"
                value={archiver.config.seed_url}
                onChange={(v) =>
                  archiver.setConfig((c) => ({ ...c, seed_url: v }))
                }
                placeholder="https://example.com"
                disabled={archiver.status !== "idle"}
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
                  disabled={archiver.status !== "idle"}
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
                  disabled={archiver.status !== "idle"}
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
                  disabled={archiver.status !== "idle"}
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
                    disabled={archiver.status !== "idle"}
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
                disabled={archiver.status !== "idle"}
              />
              <div className="flex items-center gap-2">
                <button
                  onClick={() =>
                    archiver.setConfig((c) => ({
                      ...c,
                      preserve_html: !c.preserve_html,
                    }))
                  }
                  disabled={archiver.status !== "idle"}
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

            {archiver.status === "idle" && (
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
        <main className="flex flex-1 flex-col overflow-hidden">
          {/* Error bar */}
          {archiver.error && (
            <div className="border-b border-red-900/50 bg-red-950/50 px-4 py-2 text-xs text-red-400">
              {archiver.error}
            </div>
          )}

          {/* Active progress strip */}
          {archiver.progress.length > 0 && (
            <div className="flex items-center gap-2 overflow-x-auto border-b border-gray-800 px-4 py-2">
              <span className="shrink-0 text-[11px] font-medium text-gray-500 uppercase tracking-wider">
                Live:
              </span>
              {archiver.progress.slice(0, 8).map((p) => (
                <span
                  key={p.url}
                  className="shrink-0 flex items-center gap-1 rounded-full bg-gray-800 px-2 py-0.5 text-[11px]"
                >
                  <span className="h-1.5 w-1.5 rounded-full bg-crasp-400 animate-pulse" />
                  <span className="max-w-[120px] truncate text-gray-400">
                    {p.url.replace(/^https?:\/\//, "").slice(0, 30)}
                  </span>
                  <span
                    className={`text-[10px] ${
                      p.status === "fetching"
                        ? "text-blue-400"
                        : p.status === "scraping"
                          ? "text-amber-400"
                          : "text-crasp-400"
                    }`}
                  >
                    {p.status}
                  </span>
                </span>
              ))}
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
              onClose={() => setSelectedPage(null)}
            />
          )}
        </main>
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
  onClose,
}: {
  page: ArchivedPage;
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
          <p className="mt-0.5 text-gray-300">{page.timestamp}</p>
        </div>
      </div>
      {page.content && (
        <div className="border-t border-gray-800/50 px-4 py-3">
          <span className="text-[11px] text-gray-500 uppercase tracking-wider">
            Content Preview
          </span>
          <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-all text-[11px] text-gray-400 font-mono bg-gray-950 rounded-md p-3 border border-gray-800">
            {page.content.slice(0, 2000)}
            {page.content.length > 2000 ? "\n\n... (truncated)" : ""}
          </pre>
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
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
  disabled?: boolean;
}) {
  return (
    <div>
      <label className="mb-1 block text-[11px] text-gray-500">{label}</label>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        className="w-full rounded-md border border-gray-700 bg-gray-800 px-2.5 py-1.5 text-xs text-gray-200 placeholder-gray-600 focus:border-crasp-500 focus:outline-none disabled:opacity-50"
      />
    </div>
  );
}
