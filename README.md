# mcp-docs-rs

Let AI agents quickly search the **latest docs of local crates**.

The HTML produced by `cargo doc` is a poor fit for agents to read directly: it mixes in navigation and noisy blocks (`Auto Trait Implementations`, `Blanket Implementations`), and docs for large crates are too big to read in full. This project **splits rustdoc HTML into per-item markdown** plus a flat index, and exposes a "search first, then read" retrieval capability through an MCP server.

> English | [简体中文](README.zh-CN.md)

## Technical Highlights

- **Pure Rust toolchain**: Rust 2024, a 3-crate workspace (library core / CLI / MCP server) with unified versions in `[workspace.dependencies]`.
- **Parses rustdoc HTML directly**: uses `scraper` (html5ever) to parse `cargo doc` output — **no rustdoc JSON** (still unstable), no nightly or `RUSTC_BOOTSTRAP` required; relies only on long-term stable structural invariants, so it is robust across rustdoc versions.
- **Local-only, offline**: the data source is only the local `target/doc`; never touches docs.rs.
- **Per-item splitting**: every struct / trait / fn / method / variant / field is its own item; members are both written to disk individually and inlined into the parent item.
- **Two-tier output**: `index.json` (lightweight summaries, the first thing an agent reads) + per-item markdown bodies on disk.
- **Denoising**: strips UI noise (`copy-path`, sidebar, anchors) and `blanket` / `synthetic` impl blocks; wraps code blocks in ```rust fences; `htmd` handles HTML→Markdown.
- **Searchable**: substring / prefix / fuzzy matching over name / path / summary, multi-word AND, filtering by crate and kind, with relevance scoring.
- **Incremental & cached**: rebuilds only what changed based on an artifact fingerprint and file mtimes; bodies are parsed on demand with an LRU cache.
- **Multi-project + shared index store**: one process serves many projects; when several projects depend on the same crate (same version), it is parsed and rendered once and reused across projects (materialized via hard links).
- **MCP integration**: official SDK `rmcp` 3.5 + `tokio`, stdio transport; 18 tools + `rustdoc://` resources + prompts, with structured output and resource-change notifications.
- **Engineering robustness**: `rayon` parallel builds, `spawn_blocking` for CPU-heavy work, Windows-safe file names (`encode_fs_name`), atomic writes, and a non-blocking cold-start `initialize`.

## Features

### 1. Discovery & parsing

- **Full discovery**: builds the item inventory recursively from `crates.js` and each level's `sidebar-items.js`, covering all crates in the default `cargo doc` output (dependencies included); `all.html` is used as a fallback and cross-check to backfill items missed by both the sidebar and directory scan.
- **Item kinds**: supports `struct` / `enum` / `union` / `trait` / `trait alias` / `fn` / `type` / `constant` / `static` / `macro` / `primitive` / `derive` / `proc-macro` as well as members (methods / associated items / variants / fields / required trait methods).
- **Signature & doc extraction**: extracts the `pre.rust.item-decl` signature, top-level docs, each section (`examples` / `panics` / `implementations` / `trait-implementations`, …), and members.
- **Members as items**: methods and variants become independently searchable items (ids like `crate::Type::method`), so they can be read individually after a precise search while still being inlined into the parent item for full context.
- **Link rewriting**: resolves rustdoc relative links into item markdown links / source references / external links (e.g. links to std are preserved verbatim); in-page anchors are kept.

### 2. Markdown rendering & denoising

- Strips rustdoc UI elements and noisy blocks, removes `§` anchor noise, and keeps `where` clauses.
- Code blocks are handled by the renderer: it takes the `pre.rust` text directly and wraps it in ```rust fences, avoiding wrong language inference and leftover HTML entities.
- Three link styles: `Relative` (default), `PlainPath` (most token-efficient), `KeepOriginal` (debugging).

### 3. Index & search

- **Flat `index.json`**: each item's id / kind / path / one-line summary / file location / source location — the first thing an agent reads.
- **Match modes**: substring, prefix, fuzzy (subsequence); supports **multi-word AND** (summing per-word scores).
- **Filtering & ranking**: filter by crate and kind; ranking scores exact name > prefix > path hit > summary hit, and summary hits carry a snippet.
- **Pagination**: `limit` / `offset`, returning the pre-pagination `total`.

### 4. Incremental, caching & performance

- **Fingerprint check**: `file_count` / `max_mtime` / `size_sum` / `crates.js` hash + rustdoc version + schema version — any change triggers a rebuild.
- **Incremental export**: `--incremental` compares per file by `src_mtime` and rebuilds only what changed.
- **Two-tier cache**: loads an existing `index.json` in milliseconds at startup; parses bodies only on `get_item`, using `Arc<DocItem>` + LRU (512 by default).
- **Parallel builds**: `rayon` parallelizes parsing and writing; CPU-heavy tasks run in `spawn_blocking` so the async executor is never blocked.

### 5. Multi-project & cross-project shared index store

- **Multi-project in one process**: a single `mcp-docs-server` process serves many projects; tools / resources take an optional `project` argument (defaulting to the default project). Except for the default project, other projects are lazily built on first access, and all builds share one lock and run serially.
- **Cross-project shared store**: when several projects depend on the same crate (same version + rustdoc version + granularity), its index summaries and markdown are parsed and rendered once; on a hit they are materialized into the project directory via **hard links** (same volume) / **copy** (cross volume). It lives in the platform cache directory by default, overridable via `MCP_DOCS_STORE`, and disabled with `MCP_DOCS_NO_STORE`.

### 6. Command-line tool `mcp-docs`

| Command | Description |
|---|---|
| `mcp-docs tree` | Print the item tree (debugging) |
| `mcp-docs show <id>` | Parse and print a single item |
| `mcp-docs export [--incremental] [--crate NAME] [--granularity member\|item]` | Export markdown + `index.json` + `meta.json` |
| `mcp-docs search <query> [--limit] [--offset] [--mode] [--crate] [--kind]` | Search items |

Global options: `--doc-dir` (default `target/doc`), `--out` (default `target/doc-search`), `--store` (shared index store, default platform cache directory).

### 7. MCP server

Configuration is passed entirely via **environment variables** (no command-line arguments).

Single project:

```bash
MCP_DOCS_DIR=target/doc MCP_DOCS_OUT=target/doc-search cargo run -p mcp-docs-server
```

Multiple projects (sharing one dependency index store):

```bash
MCP_DOCS_PROJECTS="core=/repo/a/target/doc;svc=/repo/b/target/doc" \
  MCP_DOCS_STORE=~/.cache/mcp-docs \
  cargo run -p mcp-docs-server
```

Register with an MCP client (`mcp.json` / `claude_desktop_config.json`, etc.): put all parameters in `env`, with `command` pointing at the built binary (`target/release/mcp-docs-server`).

Single project:

```json
{
  "mcpServers": {
    "mcp-docs": {
      "command": "/abs/path/to/mcp-docs-server",
      "env": {
        "MCP_DOCS_DIR": "/repo/target/doc",
        "MCP_DOCS_OUT": "/repo/target/doc-search"
      }
    }
  }
}
```

Multiple projects:

```json
{
  "mcpServers": {
    "mcp-docs": {
      "command": "/abs/path/to/mcp-docs-server",
      "env": {
        "MCP_DOCS_PROJECTS": "core=/repo/a/target/doc;svc=/repo/b/target/doc",
        "MCP_DOCS_STORE": "/home/me/.cache/mcp-docs",
        "MCP_DOCS_DEFAULT_PROJECT": "core"
      }
    }
  }
}
```

Without an installed binary, run directly via `"command": "cargo", "args": ["run", "-p", "mcp-docs-server"]` (recompiles on each start, slower).

Environment variables:

| Variable | Meaning |
|---|---|
| `MCP_DOCS_DIR` | Single-project rustdoc output dir (mapped to a project named `default`) |
| `MCP_DOCS_OUT` | Single-project output dir |
| `MCP_DOCS_PROJECTS` | Semicolon-separated `NAME=DOC_DIR` list |
| `MCP_DOCS_PROJECTS_FILE` | JSON manifest (`{ default?, store?, projects: [{name, doc_dir, out_dir?}] }`) |
| `MCP_DOCS_STORE` | Shared index store root (default platform cache dir) |
| `MCP_DOCS_NO_STORE` | Disable the shared store when set (`1`/`true`/`yes`/`on`) |
| `MCP_DOCS_DEFAULT_PROJECT` | Default project name |

Register this server in your MCP client (via its `env` field) to use it. Except for the default project, other projects are built in the background only **on first access**. The design principle is **search first, then read**: search tools return only lightweight summaries + pointers, and only read tools return bodies.

**Tools (18)**

| Tool | Purpose |
|---|---|
| `list_projects` | List the projects this server serves and their readiness |
| `list_crates` | List the crates indexed for a project |
| `list_items` | List item summaries (filter by crate / module / kind, supports `offset` pagination) |
| `search_items` | Search items, returning lightweight summaries and scores (supports `offset` pagination) |
| `get_item` | Read an item's full markdown |
| `get_item_source` | Look up an item's source location |
| `get_examples` | Extract rust code examples from an item's docs |
| `get_item_section` | Return the markdown of one section (e.g. `examples` / `panics`) |
| `batch_get_items` | Read multiple items at once (up to 20 ids) |
| `get_source_text` | Read source text by location (falls back to path and line numbers on failure) |
| `get_item_json` | Return an item's structured JSON |
| `index_status` | Return a project's index status (ready / building / schema / version / counts / stale) |
| `module_tree` | Return a crate's module tree (with per-level item counts) |
| `get_related_items` | Return an item's parent, siblings, and child members |
| `get_trait_implementors` | List the types that implement a trait |
| `find_by_signature` | Search by signature substring (e.g. `-> Result<`) |
| `search_docs` | Full-text search in exported markdown bodies (requires `mcp-docs export` first) |
| `rebuild_index` | Rebuild the index (incremental by default) |

In multi-project mode, all of the above tools accept an optional `project` argument (defaulting to the default project).

**Resources**: `rustdoc://crates`, `rustdoc://{crate}` (supports `?offset=&limit=` pagination), `rustdoc://{crate}/{item}`; all accept `?project=NAME`. In the item segment, `/` denotes the module separator `::` (e.g. `rustdoc://tokio/task/spawn`), while `.` still separates kind and name (e.g. `rustdoc://tokio/task.Spawn`).

**Prompts**: `explain_api` / `usage_example` (argument `id`, guiding the model to read the docs before answering).

**Protocol capabilities**: `search_items` / `get_item_json` / `index_status` / `module_tree` return structured content with an `outputSchema` (while keeping text); after a background build or `rebuild_index`, the server sends `notifications/resources/list_changed`.

**Non-blocking cold start**: `initialize` returns immediately; the first index build happens in the background, and tools / resources wait until ready before returning (an existing index is served immediately while a background refresh runs).

Typical flow: `search_items("spawn", crate="tokio")` → `get_item("tokio::spawn")` → only that small slice is returned.

### 8. Skill (optional)

`skills/mcp-docs/` is an Agent Skill: it tells the model when and how to call the tools above (search first then read, argument meanings, common pitfalls). Copy the whole directory into your MCP client's skills directory so the agent uses the server correctly, e.g. Claude Code / Codebuddy's `~/.claude/skills/`:

```bash
cp -r skills/mcp-docs ~/.claude/skills/
```

You can also unzip the packaged `skills/dist/mcp-docs.zip` into the same location. Follow whichever skills-directory convention your client uses.

## Quick start

```bash
# 1. Generate docs in your project (including dependencies)
cargo doc

# 2. Export markdown + index (defaults to target/doc-search)
cargo run -p mcp-docs-cli -- export

# 3. Search
cargo run -p mcp-docs-cli -- search "spawn" --crate tokio
```

## Project structure

| Crate | Description |
|---|---|
| `crates/mcp-docs-core` | Core library: discovery, parsing, rendering, indexing, search, cache (no async) |
| `crates/mcp-docs-cli` | Command-line tool `mcp-docs` |
| `crates/mcp-docs-server` | MCP server `mcp-docs-server` (stdio) |

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

See [docs/conventions.md](docs/conventions.md) for conventions.

## Documentation

Requirements, design, roadmap, and development conventions live in [`docs/`](docs/) (written in Simplified Chinese):

- [Requirements](docs/requirements.md)
- [Design](docs/design.md)
- [Roadmap & progress](docs/roadmap.md)
- [Conventions](docs/conventions.md)
- [Lessons learned](docs/lessons.md)
- [Issue tracking](docs/issues.md)
- [Glossary & code map](docs/glossary.md)

## License

MIT OR Apache-2.0
