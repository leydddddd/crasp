import { useState, useMemo, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileDown, X, FolderOpen } from "lucide-react";
import type {
  ExportFormat,
  ExportScope,
  ExportContent,
  ExportRequest,
  ExportResult,
  PageSummary,
} from "@/types/archiver";

const FORMAT_OPTIONS: { value: ExportFormat; label: string }[] = [
  { value: "plain_text", label: "Plain Text" },
  { value: "markdown", label: "Markdown" },
  { value: "html", label: "HTML" },
  { value: "epub", label: "EPUB" },
];

const SCOPE_OPTIONS: { value: ExportScope; label: string }[] = [
  { value: "single_page", label: "Single Page" },
  { value: "whole_crawl_one_file", label: "One File" },
  { value: "whole_crawl_folder", label: "Folder" },
];

const CONTENT_OPTIONS: { value: ExportContent; label: string }[] = [
  { value: "content_only", label: "Content Only" },
  { value: "with_metadata", label: "With Metadata" },
  { value: "with_assets", label: "With Assets" },
  { value: "full", label: "Full" },
];

interface ExportPanelProps {
  open: boolean;
  onClose: () => void;
  context: "single_page" | "whole_crawl";
  page?: PageSummary | null;
  crawlId?: string | null;
}

export function ExportPanel({ open, onClose, context, page, crawlId }: ExportPanelProps) {
  const [format, setFormat] = useState<ExportFormat>("plain_text");
  const [scope, setScope] = useState<ExportScope>(
    context === "single_page" ? "single_page" : "whole_crawl_one_file"
  );
  const [content, setContent] = useState<ExportContent>("with_metadata");
  const [exporting, setExporting] = useState(false);
  const [result, setResult] = useState<ExportResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const isEpub = format === "epub";

  const availableScopes = useMemo(() => {
    if (isEpub) {
      return SCOPE_OPTIONS.filter((s) => s.value === "whole_crawl_one_file");
    }
    if (context === "single_page") {
      return SCOPE_OPTIONS.filter((s) => s.value === "single_page");
    }
    return SCOPE_OPTIONS.filter((s) => s.value !== "single_page");
  }, [isEpub, context]);

  const previewText = useMemo(() => {
    let pages = 1;
    if (scope !== "single_page") {
      pages = 5; // rough page count estimate for preview display
    }
    const ext =
      format === "plain_text" ? "txt" : format === "markdown" ? "md" : format === "html" ? "html" : "epub";
    const filename = scope === "whole_crawl_folder" ? `export_folder/` : `export.${ext}`;
    return `${pages} page${pages > 1 ? "s" : ""} → ${filename}`;
  }, [format, scope]);

  const handleExport = useCallback(async () => {
    setExporting(true);
    setError(null);
    setResult(null);
    try {
      const request: ExportRequest = {
        format,
        scope,
        content,
        pageUrl: context === "single_page" ? page?.url : undefined,
        crawlId: context === "whole_crawl" ? crawlId || undefined : undefined,
        source: page?.source,
      };
      const res = await invoke<ExportResult>("export_content", { request });
      setResult(res);
    } catch (e) {
      setError(String(e));
    } finally {
      setExporting(false);
    }
  }, [format, scope, content, context, page, crawlId]);

  const handleReveal = useCallback(async () => {
    if (result?.path) {
      try {
        await invoke("reveal_in_explorer", { path: result.path });
      } catch (e) {
        console.error("Failed to reveal:", e);
      }
    }
  }, [result]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-full max-w-md rounded-lg border border-gray-700 bg-gray-900 p-6 shadow-xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold text-gray-100">Export Options</h2>
          <button onClick={onClose} className="text-gray-400 hover:text-gray-200">
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-xs text-gray-500">Format</label>
            <select
              value={format}
              onChange={(e) => setFormat(e.target.value as ExportFormat)}
              className="w-full rounded-md border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
            >
              {FORMAT_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="mb-1 block text-xs text-gray-500">Scope</label>
            <select
              value={scope}
              onChange={(e) => setScope(e.target.value as ExportScope)}
              disabled={availableScopes.length === 1}
              className="w-full rounded-md border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none disabled:opacity-50"
            >
              {availableScopes.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
          </div>

          {!isEpub && (
            <div>
              <label className="mb-1 block text-xs text-gray-500">Content</label>
              <select
                value={content}
                onChange={(e) => setContent(e.target.value as ExportContent)}
                className="w-full rounded-md border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-200 focus:border-crasp-500 focus:outline-none"
              >
                {CONTENT_OPTIONS.map((opt) => (
                  <option key={opt.value} value={opt.value}>
                    {opt.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          {isEpub && (
            <p className="text-xs text-gray-500">
              EPUB always includes metadata per chapter.
            </p>
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
              onClick={onClose}
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
