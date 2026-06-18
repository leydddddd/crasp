# Crasp

A hybrid desktop web archiver built with **Tauri 2 + Rust + React 18**. Crasp crawls
a seed URL within a bounded depth, sanitizes and extracts page content, computes
cryptographic content hashes (MD5 / SHA-256), and streams live progress to a
virtualized dashboard. It supports three interchangeable crawl engines behind a
single UI so that heavy network I/O can be offloaded to the cloud or run entirely
locally with no code changes.

---

## Features

### Crawl engines (three modes, one item schema)

1. **Local Rust engine (default)** — a fully async Tokio crawler (`crawler.rs`):
   - Bounded-depth BFS frontier with same-domain + same-scheme link filtering
   - `DashSet` visited-set dedup with normalized URLs (fragment-stripped, trailing-slash trimmed, lowercased)
   - Concurrency-limited workers via a `tokio::Semaphore`
   - Gzip / Brotli / Deflate-aware HTTP client with connect + response timeouts
   - Pause / resume / cancel via `tokio::watch` channels
   - Live Tauri events: `scrape-progress`, `crawl-discover`, `archive-success`, `crawl-done`

2. **Zyte Scrapy Cloud engine** — offloads network I/O to a remote worker drone:
   - `start_cloud_crawl` submits a spider job to the Zyte API, polls status, and
     batch-fetches results as JSON Lines (`zyte.rs`)
   - Items are parsed on `spawn_blocking` so the UI never blocks
   - Emits `cloud-progress` + `archive-success` events to the same dashboard

3. **Local Scrapy engine** — runs the same Python spider locally as a fallback:
   - `local_scrapy_crawl` shells out to `scrapy crawl` via `tokio::process::Command`
   - Produces the identical JSON Lines item schema as the Zyte path

> The native Scrapy spider (`scrapy_spider/crasp_spider/`) is written with **no
> cloud-specific dependencies**, so the exact same code runs in Zyte and locally.

### Content processing

- HTML parsing via the `scraper` crate with CSS-selector content extraction
  (user-configurable selectors, falling back to `article` → `main` → `body`)
- Sanitization: strips `<script>`, `<style>`, `<noscript>`, `<iframe>`, forms,
  embeds, SVG/canvas; removes inline event handlers, `style`, and tracking/data
  attributes; preserves or text-extracts content based on the `preserve_html` toggle
- Cryptographic hashing of extracted content with selectable **MD5** or **SHA-256**

### Persistence (MongoDB, zero-vendor-lock-in)

- `ArchiveStore` connects from a single connection string (`CRASP_MONGO_URI`) and
  works identically against an Atlas M10 cluster, the forever-free M0 tier, or a
  local Docker MongoDB — no application code changes between tiers
- Three collections with standard (M0-compatible) indexes:
  - `pages` — archived page documents (content, hash, depth, status, `duplicate_group_id`, `search_blob`)
  - `crawls` — per-run metadata + stats
  - `content_hashes` — cross-crawl cryptographic dedup registry
- `ensure_indexes()` creates all indexes on startup; `persist_batch()` inserts in
  chunks sized for the shared-tier rate limits

### Desktop UI

- Dark, single-window dashboard (`ArchiverDashboard.tsx`) styled with Tailwind CSS
- Crawl configuration panel: seed URL, max depth, max pages, concurrency,
  hash algorithm, CSS selectors, preserve-HTML toggle
- Live statistics (discovered / processing / archived / failed) with progress bar
- Real-time progress strip of in-flight pages
- **Virtualized** page list (`@tanstack/react-virtual`) for thousands of rows
- Detail drawer with URL, hash, timestamp, and a content preview
- Pause / resume / cancel / reset controls

---

## Architecture

```
                ┌─────────────── Desktop App (Tauri) ───────────────┐
                │                                                   │
                │   React UI  ◀──events──▶  Rust/Tokio backend      │
                │      │  invoke()              │                    │
                │      ▼                         │                    │
                │   useArchiver hook            ├─▶ Local Rust engine │
                │                               ├─▶ ZyteClient ─HTTPS─▶ Zyte Cloud
                │                               │      (JSON Lines items)
                │                               ├─▶ Local Scrapy (tokio::process::Command)
                │                               ▼                     │
                │                          Item ingester              │
                │                               ▼                     │
                │                    ArchiveStore (one MONGO_URI)     │
                └───────────────────────────────┼─────────────────────┘
                                                ▼
                          MongoDB  (Atlas M10 / M0 / local Docker)
                          collections: pages · crawls · content_hashes
```

The cloud and local-Scrapy engines emit the same JSON Lines item shape that the
local Rust engine produces internally, so all three feed a single event pipeline.

---

## Project structure

```
Crasp/
├── index.html                      # Vite entry
├── package.json                    # Frontend manifest (name: crasp)
├── tailwind.config.js              # Tailwind config (crasp color palette)
├── vite.config.ts
├── src/                            # React frontend
│   ├── main.tsx
│   ├── App.tsx
│   ├── index.css
│   ├── components/
│   │   └── ArchiverDashboard.tsx   # Main dashboard UI
│   ├── hooks/
│   │   └── useArchiver.ts          # Tauri event + invoke orchestration
│   └── types/
│       └── archiver.ts             # Shared TS types
├── scrapy_spider/                  # Native Scrapy spider (cloud + local)
│   ├── setup.py
│   └── crasp_spider/
│       ├── settings.py
│       └── spiders/
│           ├── archive.py          # ArchiveSpider (name: crasp_archive)
│           └── run.py
├── mongo-init.js                   # MongoDB index bootstrap script
└── src-tauri/                      # Rust/Tauri backend
    ├── Cargo.toml                  # Crate name: crasp (lib: crasp_lib)
    ├── tauri.conf.json
    ├── capabilities/default.json
    └── src/
        ├── main.rs
        ├── lib.rs                  # Tauri builder + command registration
        ├── commands.rs             # Tauri commands (crawl lifecycle)
        ├── crawler.rs              # Local async Rust crawler
        ├── zyte.rs                 # Zyte Scrapy Cloud HTTP client
        ├── local_scrapy.rs         # Local Scrapy process launcher
        ├── store.rs                # MongoDB ArchiveStore
        ├── schema.rs               # BSON document schemas
        └── runtime.rs              # AppContext / env-based wiring
```

---

## Prerequisites

- **Node.js** 18+ and npm
- **Rust** (stable) with the Tauri 2 CLI: `npm install -g @tauri-apps/cli` or use the
  project-local `npm run tauri`
- **Python 3.9+** with `scrapy` and `parsel` (only required for the Scrapy engines):
  `pip install scrapy parsel`
- **MongoDB** — one of:
  - A local Docker instance: `docker run -d -p 27017:27017 --name crasp-mongo mongo:7`
  - MongoDB Atlas (M0 free tier or M10+)

---

## Configuration

Crasp is configured through environment variables (read by `runtime.rs` on startup):

| Variable            | Required | Default                    | Purpose                                           |
| ------------------- | -------- | -------------------------- | ------------------------------------------------- |
| `CRASP_MONGO_URI`   | no       | `mongodb://localhost:27017`| MongoDB connection string (M10 / M0 / local)      |
| `ZYTE_API_KEY`      | no       | —                          | Zyte API key; enables the cloud engine when set   |
| `CRASP_ZYTE_PROJECT`| no       | —                          | Zyte Scrapy Cloud project ID (cloud engine)       |
| `CRASP_FEED_URI`    | no       | —                          | Scrapy feed output URI (used by the spider)       |

The desktop UI drives crawl parameters (seed URL, depth, page cap, concurrency,
selectors, hash algorithm, preserve-HTML) at runtime — no rebuild required.

---

## Enabling the Cloud and Local-Scrapy Engines

Follow these steps in order:

1. **MongoDB** — Set `CRASP_MONGO_URI` (or use the default local connection). All
   engines require MongoDB to persist scraped items. Without it, the local Rust
   engine still runs but items are not stored.

2. **Local Scrapy** — Install Python dependencies for the spider:
   ```bash
   pip install scrapy parsel
   ```
   Then select **Local Scrapy** in the dashboard engine selector. No additional
   env vars are needed; the spider runs as a local subprocess.

3. **Zyte Cloud** — Set both `ZYTE_API_KEY` and `CRASP_ZYTE_PROJECT` in your
   environment (or enter them in the dashboard's **Cloud** engine panel). The
   dashboard shows a green dot when Zyte is configured.

The **Engine** selector in the dashboard sidebar switches between the three
engines at runtime. When **Cloud** is selected, two input fields appear for
the API key and project ID (project ID is pre-filled from `CRASP_ZYTE_PROJECT`
when available). The **Services** row shows real-time readiness of MongoDB and
Zyte with colored indicators.

---

## Getting started

### 1. Frontend + backend (local Rust engine)

```bash
npm install
npm run tauri dev
```

This launches the Vite dev server and the Tauri Rust backend together.

### 2. (Optional) MongoDB bootstrap

Start a local MongoDB and run the index bootstrap script:

```bash
docker run -d -p 27017:27017 --name crasp-mongo mongo:7
mongosh "mongodb://localhost:27017" mongo-init.js
```

Or set `CRASP_MONGO_URI` to your Atlas connection string.

### 3. (Optional) Scrapy engines

```bash
cd scrapy_spider
pip install -e .
# run the spider directly (same code Zyte runs remotely):
scrapy crawl crasp_archive -a seed_url=https://example.com -a max_depth=3 -a max_pages=100 -o items.jl
```

---

## Tauri command surface

| Command            | Engine        | Description                                            |
| ------------------ | ------------- | ------------------------------------------------------ |
| `start_crawl`      | Local Rust    | Start the async Tokio crawler with a `CrawlConfig`     |
| `start_cloud_crawl`| Zyte Cloud    | Submit + monitor + ingest a remote spider job          |
| `local_scrapy_crawl`| Local Scrapy | Run the native spider via `tokio::process::Command`    |
| `cancel_crawl`     | —             | Cancel the active crawl                                |
| `pause_crawl`      | —             | Pause the active crawl                                 |
| `resume_crawl`     | —             | Resume a paused crawl                                  |
| `validate_url`     | —             | Validate a seed URL                                    |
| `default_config`   | —             | Return the default `CrawlConfig`                       |
| `get_app_status`   | —             | Return MongoDB/Zyte readiness flags (for UI polling)  |

---

## MongoDB schema

### `pages`

```jsonc
{
  "crawl_id": "uuid",
  "url": "https://example.com/page",
  "url_normalized": "https://example.com/page",
  "depth": 2,
  "title": "Page Title",
  "status": "Completed",
  "status_code": 200,
  "content": "<article>…</article>",
  "content_format": "html",
  "discovered_links": 23,
  "timestamp": "2026-06-17T18:48:01Z",
  "duplicate_group_id": 0,
  "search_blob": ""
}
```

### `crawls`

```jsonc
{
  "crawl_id": "uuid",
  "seed_url": "https://example.com",
  "config": { "max_depth": 3, "max_pages": 100, "concurrency": 4, "css_selectors": ["article","main","body"], "preserve_html": true, "hash_algorithm": "sha256" },
  "source": "local",
  "zyte_job_key": null,
  "started_at": "2026-06-17T18:46:42Z",
  "finished_at": null,
  "stats": { "discovered": 0, "archived": 0, "failed": 0, "skipped": 0 },
  "cancelled": false
}
```

### `content_hashes`

```jsonc
{
  "hash": "9f86d081884c7d659…",
  "hash_algorithm": "sha256",
  "first_seen_crawl_id": "uuid",
  "first_seen_url": "https://example.com/page",
  "first_seen_at": "2026-06-17T18:48:01Z",
  "occurrences": 1
}
```

Indexes are created automatically by `ArchiveStore::ensure_indexes()` and mirror
`mongo-init.js`.

---

## Tech stack

- **Backend:** Rust, Tauri 2, Tokio, reqwest, scraper, sha2, md-5, dashmap,
  parking_lot, mongodb, bson
- **Frontend:** React 18, TypeScript, Vite 6, Tailwind CSS 3,
  @tanstack/react-virtual, lucide-react
- **Spider:** Python, Scrapy, parsel

---

## License

Private project. All rights reserved.
