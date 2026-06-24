import { useCallback, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileDown, FolderOpen, X } from "lucide-react";
import type {
  ExportContent,
  ExportFormat,
  ExportRequest,
  ExportResult,
  ExportScope,
  PageSummary,
} from "@/types/archiver";

const FORMAT_OPTIONS: { value: ExportFormat; label: string }[] = [
  { value: "plain_text", label: "Plain Text" },
  { value: "markdown", label: "Markdown" },
  { value: "html", label: "Self-contained HTML" },
  { value: "epub", label: "EPUB" },
];

const SCOPE_OPTIONS: { value: ExportScope; label: string }[] = [
  { value: "single_page", label: "This page only" },
  { value: "whole_crawl_one_file", label: "Entire session" },
  { value: "whole_crawl_folder", label: "Session folder" },
  { value: "selected_pages", label: "Selected pages" },
];

const CONTENT_OPTIONS: { value: ExportContent; label: string }[] = [
  { value: "content_only", label: "Content only" },
  { value: "with_metadata", label: "With metadata" },
  { value: "with_assets", label: "With assets" },
  { value: "full", label: "Full" },
];

interface ExportPanelProps {
  context: "single_page" | "whole_crawl";
  page?: PageSummary | null;
  crawlId?: string | null;
  crawlName?: string | null;
  pageCount?: number | null;
  selectedUrls?: string[];
  onCancel: () => void;
}

export function ExportPanel({
  context,
  page,
  crawlId,
  crawlName,
  pageCount,
  selectedUrls,
  onCancel,
}: ExportPanelProps) {
  const [format, setFormat] = useState<ExportFormat>("plain_text");
  const [scope, setScope] = useState<ExportScope>(
    context === "single_page" ? "single_page" : "whole_crawl_one_file",
  );
  const [content, setContent] = useState<ExportContent>("with_metadata");
  const [exporting, setExporting] = useState(false);
  const [result, setResult] = useState<ExportResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [outputName, setOutputName] = useState("");

  const isEpub = format === "epub";

  const availableScopes = useMemo(() => {
    if (isEpub) {
      return SCOPE_OPTIONS.filter((s) => s.value === "whole_crawl_one_file");
    }
    if (context === "single_page") {
      return SCOPE_OPTIONS.filter((s) => s.value === "single_page");
    }
    const scopes = SCOPE_OPTIONS.filter((s) => s.value !== "single_page");
    if (!selectedUrls || selectedUrls.length === 0) {
      return scopes.filter((s) => s.value !== "selected_pages");
    }
    return scopes;
  }, [isEpub, context, selectedUrls]);

  const previewText = useMemo(() => {
    const pages = scope === "single_page" ? 1 : scope === "selected_pages" ? (selectedUrls?.length || 0) : (pageCount || 0);
    const ext =
      format === "plain_text"
        ? "txt"
        : format === "markdown"
          ? "md"
          : format === "html"
            ? "html"
            : "epub";
    const baseName =
      outputName || crawlName || page?.url?.replace(/^https?:\/\//, "") || "export";
    const filename =
      scope === "whole_crawl_folder"
        ? `${baseName}-export/`
        : `${baseName}.${ext}`;
    return `${pages} page${pages === 1 ? "" : "s"} - ${filename}`;
  }, [format, scope, pageCount, crawlName, page, selectedUrls, outputName]);

  const handleExport = useCallback(async () => {
    setExporting(true);
    setError(null);
    setResult(null);
    try {
      const request: ExportRequest = {
        format,
        scope: scope as "single_page" | "whole_crawl_one_file" | "whole_crawl_folder" | "selected_pages",
        content,
        pageUrl: context === "single_page" ? page?.url : undefined,
        crawlId: context === "whole_crawl" ? crawlId || undefined : crawlId || undefined,
        source: page?.source,
        outputName: outputName || undefined,
        selectedUrls: scope === "selected_pages" ? selectedUrls : undefined,
      };
      const res = await invoke<ExportResult>("export_content", { request });
      setResult(res);
    } catch (e) {
      setError(String(e));
    } finally {
      setExporting(false);
    }
  }, [format, scope, content, context, page, crawlId, selectedUrls, outputName]);

  const handleReveal = useCallback(async () => {
    if (result?.path) {
      try {
        await invoke("reveal_in_explorer", { path: result.path });
      } catch (e) {
        console.error("Failed to reveal:", e);
      }
    }
  }, [result]);

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-gray-800 px-6 py-4">
        <div>
          <h2 className="text-lg font-semibold text-gray-100">Export</h2>
          <p className="text-xs text-gray-500">
            Exporting: {context === "single_page" ? page?.url : crawlName}
          </p>
        </div>
        <button
          onClick={onCancel}
          className="text-gray-500 hover:text-gray-300"
        >
          <X className="h-5 w-5" />
        </button>
      </div>

      <div className="flex-1 overflow-auto px-6 py-6">
        <div className="max-w-xl space-y-6">
          <section className="rounded-lg border border-gray-800 bg-gray-900/50 p-5">
            <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-gray-400">
              Format
            </h3>
            <div className="space-y-2">
              {FORMAT_OPTIONS.map((opt) => (
                <label
                  key={opt.value}
                  className="flex items-center gap-2 text-sm text-gray-300"
                >
                  <input
                    type="radio"
                    checked={format === opt.value}
                    onChange={() => setFormat(opt.value)}
                    className="h-3 w-3 text-crasp-600"
                  />
                  {opt.label}
                </label>
              ))}
            </div>
          </section>

          <section className="rounded-lg border border-gray-800 bg-gray-900/50 p-5">
            <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-gray-400">
              Scope
            </h3>
            <div className="space-y-2">
              {availableScopes.map((opt) => (
                <label
                  key={opt.value}
                  className="flex items-center gap-2 text-sm text-gray-300"
                >
                  <input
                    type="radio"
                    checked={scope === opt.value}
                    onChange={() => setScope(opt.value)}
                    className="h-3 w-3 text-crasp-600"
                  />
                  {opt.label}
                </label>
              ))}
            </div>
          </section>

          {!isEpub && (
            <section className="rounded-lg border border-gray-800 bg-gray-900/50 p-5">
              <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-gray-400">
                Content Level
              </h3>
              <div className="space-y-2">
                {CONTENT_OPTIONS.map((opt) => (
                  <label
                    key={opt.value}
                    className="flex items-center gap-2 text-sm text-gray-300"
                  >
                    <input
                      type="radio"
                      checked={content === opt.value}
                      onChange={() => setContent(opt.value)}
                      className="h-3 w-3 text-crasp-600"
                    />
                    {opt.label}
                  </label>
                ))}
              </div>
            </section>
          )}

          {isEpub && (
            <p className="text-xs text-gray-500">
              EPUB export always includes chapter metadata.
            </p>
          )}

          {scope !== "whole_crawl_folder" && (
            <section className="rounded-lg border border-gray-800 bg-gray-900/50 p-5">
              <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-gray-400">
                Filename
              </h3>
              <input
                type="text"
                value={outputName}
                onChange={(e) => setOutputName(e.target.value)}
                placeholder="Auto-generated"
                className="w-full rounded-md border border-gray-700 bg-gray-800 px-2.5 py-1.5 text-xs text-gray-200 placeholder-gray-600 focus:border-crasp-500 focus:outline-none"
              />
              <p className="mt-1 text-[11px] text-gray-500">
                Leave empty for auto-generated name.
              </p>
            </section>
          )}

          <div className="rounded-md bg-gray-800 p-3 text-xs text-gray-400">
            Preview: {previewText}
          </div>

          {error && <p className="text-xs text-red-400">{error}</p>}

          {result && (
            <div className="rounded-md bg-emerald-900/20 p-3 text-xs text-emerald-400">
              <p className="font-medium">Exported successfully!</p>
              <p className="mt-1 break-all">{result.path}</p>
              <button
                onClick={handleReveal}
                className="mt-2 flex items-center gap-1 rounded bg-emerald-600/20 px-2 py-1 text-emerald-400 hover:bg-emerald-600/30"
              >
                <FolderOpen className="h-3 w-3" />
                Show in Explorer
              </button>
            </div>
          )}

          <div className="flex gap-2 pt-2">
            <button
              onClick={handleExport}
              disabled={exporting}
              className="flex flex-1 items-center justify-center gap-2 rounded-md bg-crasp-600 px-4 py-2 text-sm font-medium text-white hover:bg-crasp-500 transition-colors disabled:opacity-50"
            >
              {exporting ? (
                <span className="h-4 w-4 animate-spin rounded-full border-2 border-white border-t-transparent" />
              ) : (
                <FileDown className="h-4 w-4" />
              )}
              Export
            </button>
            <button
              onClick={onCancel}
              className="rounded-md bg-gray-800 px-4 py-2 text-sm text-gray-300 hover:bg-gray-700 transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
