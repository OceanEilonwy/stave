# AI Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This plan is run as a workflow of lanes; each lane owns the files listed for it and nothing else.

**Goal:** Stave as an MCP server for Claude Desktop, Claude Code and Codex: a read-only 3D preview (WASM + WebGPU, the existing renderer), plain instructions turned into checked patches, jj-based forks and merges kept in `.stave/history/`, conflict markers in the language, and a per-project merge policy.

**Architecture:** New crates `stave-render` (renderer moved out of `stave-gtk`), `stave-history` (jj-lib; Stave is the working copy), `stave-link` (install into AI apps; talk to the running window over D-Bus), `stave-mcp` (the MCP server) and `stave-web` (the WASM viewer). Conflict markers go into `stave-lang`. The window gains save-merges-to-main, catching up, Versions/Suggestions and conflict views.

**Tech Stack:** Rust 2024; wgpu 30; jj-lib (pinned exact version, latest on crates.io at build time); rmcp (the official MCP Rust SDK); zbus; wasm-bindgen; tokio (stave-mcp only).

**Spec:** `docs/superpowers/specs/2026-11-20-ai-integration-design.md`. Read it before any task.

## Global Constraints

- Every rule in `CLAUDE.md`: no `unwrap`/`expect`/`panic!`/fallible indexing/lossy `as` in library code; `thiserror` in libraries, `anyhow` in binaries; newtypes and enums over raw values; doc comment on every public item; `BTreeMap`/sorted output where order is observable; full words in names; no kind named in a core crate.
- Before every commit: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (for the crates you touched, plus `cargo test -p stave-check --test standing_rule`). Set `CARGO_TARGET_DIR=target/<lane>`.
- Spec first: a language change goes into `docs/grammar.md` in the same commit; window behaviour into `docs/gui.md`; editing into `docs/editing.md`.
- User-facing text is plain woodworking English. jj, git and MCP are never named in the window.
- Linux only. WebGPU required in the viewer; no WebGL fallback.
- Nothing is ever pushed by Stave unless the user asks. No network calls except the MCP stdio channel and the localhost viewer bound to `127.0.0.1`.
- There is no cut list in Stave yet (`stave-output` doesn't exist): no `cut_list` tool.
- Commit only paths your lane owns (`git add <paths>`). Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A path outside the project** given to `open_project`, `new_project`, or in a patch (`..`, absolute, a symlink): refused with a plain message; nothing read or written outside the project folder. Test in Task D2 and B3.
2. **Two merges at the same moment** (window save while an agent applies): both land, main stays linear, neither is lost. Test in Task B3.
3. **A project folder inside a bigger git repository**: versions refused with the copy offer; no `.stave/history` made. Test in Task B1.
4. **A file with conflict markers opened, formatted, checked and saved**: it round-trips byte for byte, `stave fmt` leaves it alone, `stave check` reports `error[conflict]` and still checks each side. Test in Task C2.
5. **The localhost viewer hit without the session token, or from another origin**: refused (403); bound to `127.0.0.1` only. Test in Task J1.

---

## Phase 1: foundations (5 lanes in parallel)

### Lane A: `stave-render`

#### Task A1: move the renderer into `stave-render`

**Files:**
- Create: `crates/stave-render/{Cargo.toml,src/lib.rs,src/meshes.rs,src/profile.rs,src/shader.wgsl}` (moved from `crates/stave-gtk/src/render/`)
- Modify: `crates/stave-gtk/src/render/mod.rs` (keeps only `dmabuf.rs`, `readback.rs` and the glue), `crates/stave-gtk/Cargo.toml`, root `Cargo.toml` (member), `CLAUDE.md` (dependency order: `stave-render` after `stave-scene`; wgpu rule as in the spec)

**Interfaces:**
- Produces (`stave_render`): `Renderer` with `pub async fn headless() -> Result<Renderer, RenderError>` (offscreen, any adapter), `pub async fn for_surface(instance: wgpu::Instance, surface: wgpu::Surface<'static>, width: u32, height: u32) -> Result<Renderer, RenderError>`, `pub fn upload(&mut self, scene: &SceneGraph)`, `pub fn render(&mut self, frame: &Frame<'_>) -> Result<Image, RenderError>` (offscreen), `pub fn present(&mut self, frame: &Frame<'_>) -> Result<(), RenderError>` (surface), `pub fn resize(&mut self, width: u32, height: u32)`, `pub fn device(&self) -> &wgpu::Device`, `pub fn queue(&self) -> &wgpu::Queue`, `pub fn encode_png(image: &Image) -> Result<Vec<u8>, RenderError>`. `Frame`, `Image`, `Highlighted`, `Backdrop`, `Mask` keep their current fields. `RenderError` is a `thiserror` enum.
- `stave-gtk` keeps `pollster::block_on(Renderer::headless())` at its own call site and its dmabuf/readback path.

- [ ] Step 1: Test `renders_box_example_headless` in `crates/stave-render/tests/headless.rs`: open `examples/box` with `stave_app::open`, build `stave_app::view::scene`, `pollster::block_on(Renderer::headless())`; if no adapter, return early printing "no GPU adapter" (CI has none); else `render` a 64×64 frame and assert some pixel differs from the backdrop. (Dev-dependency on `stave-app` is fine for tests.)
- [ ] Step 2: Move the files; replace `anyhow` with `RenderError`; make device setup `async`; replace `std::time::Instant` with `web_time::Instant`; gate dmabuf-only features behind a `dmabuf` cargo feature off by default (gtk enables it).
- [ ] Step 3: `cargo check -p stave-render --target wasm32-unknown-unknown` passes (wgpu `webgpu` feature on wasm32).
- [ ] Step 4: `stave-gtk` builds and its tests pass; screenshots (`stave-gtk/src/screenshot.rs`) unchanged.
- [ ] Step 5: fmt, clippy, test; commit `refactor(render): the renderer is its own crate, stave-render`.

### Lane B: `stave-history`

**Files (whole lane):** `crates/stave-history/**`, root `Cargo.toml` (member). Depends on `jj-lib` (exact pin, `git` feature), `thiserror`, `fs4` or `fs2` (lock), `ignore` (gitignore matching).

**Interfaces produced (`stave_history`), used by Lanes G, H, I:**

```rust
pub struct ProjectFiles(BTreeMap<RelativePath, Vec<u8>>); // RelativePath: validated, no `..`, no absolute
pub struct RelativePath(PathBuf);         // fn new(&Path) -> Result<Self, PathError>
pub enum ActorKind { User, Agent }
pub struct Actor { pub kind: ActorKind, pub name: String, pub id: String }
pub struct RevisionId(String);            // jj commit id, hex
pub struct SuggestionId(String);          // bookmark name suffix
pub enum VersionsStatus { Off, On, Nested { repository_root: PathBuf } }
pub struct Version { pub id: RevisionId, pub actor: Actor, pub message: String, pub time: String }
pub struct Suggestion { pub id: SuggestionId, pub agent: Actor, pub title: String, pub patches: Vec<String>, pub base: RevisionId, pub conflicted: Vec<RelativePath> }
pub enum CatchUp { UpToDate, Write { files: ProjectFiles, removed: Vec<RelativePath>, conflicted: Vec<RelativePath> } }
pub enum MergePolicy { Off, Ask, Auto }
pub struct History { /* private */ }
impl History {
    pub fn status(folder: &Path) -> Result<VersionsStatus, HistoryError>;
    pub fn start(folder: &Path) -> Result<History, HistoryError>;   // Nested → HistoryError::Nested
    pub fn open(folder: &Path) -> Result<History, HistoryError>;
    pub fn read_folder(&self) -> Result<ProjectFiles, HistoryError>; // every file except .stave/history and .gitignore'd
    pub fn files_at(&self, revision: &RevisionId) -> Result<ProjectFiles, HistoryError>;
    pub fn main(&self) -> Result<RevisionId, HistoryError>;
    pub fn save(&mut self, files: &ProjectFiles, actor: &Actor, message: &str) -> Result<RevisionId, HistoryError>;
    pub fn catch_up(&mut self, folder_files: &ProjectFiles) -> Result<CatchUp, HistoryError>;
    pub fn suggest(&mut self, agent: &Actor, onto: Option<&SuggestionId>, files: &ProjectFiles, patch: &str, title: &str) -> Result<SuggestionId, HistoryError>;
    pub fn suggestions(&self) -> Result<Vec<Suggestion>, HistoryError>;
    pub fn suggestion_files(&self, id: &SuggestionId) -> Result<ProjectFiles, HistoryError>;
    pub fn apply(&mut self, id: &SuggestionId, actor: &Actor, base: &RevisionId, merged: &ProjectFiles) -> Result<RevisionId, HistoryError>; // caller replayed & checked on `base` (the main it read); base→merged is 3-way merged onto main if main moved
    pub fn rebase_suggestion(&mut self, id: &SuggestionId) -> Result<Vec<RelativePath>, HistoryError>; // conflicted files, empty = clean
    pub fn write_suggestion_files(&mut self, id: &SuggestionId, files: &ProjectFiles) -> Result<(), HistoryError>; // resolution
    pub fn discard(&mut self, id: &SuggestionId, actor: &Actor) -> Result<(), HistoryError>;
    pub fn undo(&mut self, actor: &Actor, scope: UndoScope) -> Result<String, HistoryError>; // describes what was undone
    pub fn versions(&self, limit: usize) -> Result<Vec<Version>, HistoryError>;
}
pub enum UndoScope { Own, Any }
pub fn merge_policy(folder: &Path, config_dir: &Path) -> MergePolicy; // .stave/agents.toml, then <config_dir>/agents.toml, else Ask
```

Actor is stored in each commit's description trailer (`Stave-Actor: agent Claude Desktop 3f2a`) and operation tags; a suggestion's patches in trailers `Stave-Patch:` (base64) one per patch. *(Builder's choice in the spec: settled here.)*

#### Task B1: start, open, status, read folder
- [ ] Tests (`tests/start.rs`, temp dirs): `start_makes_only_stave_history` (no `.jj`, no `.git` in folder root); `start_in_git_root_keeps_git_history` (init a git repo with one commit via `git` CLI in the test, start, `versions(10)` contains it); `nested_folder_is_refused` (folder inside a git repo → `VersionsStatus::Nested`, `start` errs, no `.stave/history`); `read_folder_skips_history_and_gitignored`.
- [ ] Implement with jj-lib: repository at `.stave/history/repo` (`ReadonlyRepo::init` with git backend; external git when the folder is a git root), no jj `Workspace`.
- [ ] Commit.

#### Task B2: save, main, files_at, versions, catch_up
- [ ] Tests: `save_moves_main_and_records_actor`; `catch_up_after_agent_apply_writes_files`; `catch_up_with_unsaved_clash_reports_conflict` (the conflicted file holds jj snapshot-style markers).
- [ ] Implement; conflict marker style `snapshot`.
- [ ] Commit.

#### Task B3: suggestions, apply, lock, undo, discard, policy
- [ ] Tests: `suggestion_survives_reopen`; `apply_moves_main`; `concurrent_save_and_apply_both_land` (two threads, two `History` handles on one folder; main linear, both changes present); `agent_undo_only_reverts_own` (agent undo after a later user save → error naming the user's save); `relative_path_rejects_parent_and_absolute`; `merge_policy_reads_project_then_global_then_ask`.
- [ ] Implement: lock file `.stave/history/main.lock` held only while moving main.
- [ ] Commit.

### Lane C: conflict markers in `stave-lang`

**Files:** `docs/grammar.md` (new "Conflicts" section), `crates/stave-lang/src/{lexer.rs,parser.rs,syntax.rs,format.rs,diagnostics.rs}`, new `crates/stave-lang/src/conflict.rs`, tests and corpus files under `crates/stave-lang/tests/` (`conflict_*.stave`).

**Interfaces produced (`stave_lang::conflict`), used by Lanes H and I:**

```rust
pub struct Conflict { pub range: Range<usize>, pub sides: Vec<ConflictSide>, pub base: Option<ConflictSide> }
pub struct ConflictSide { pub label: String, pub range: Range<usize> } // range of the side's text
pub fn find_conflicts(text: &str) -> Vec<Conflict>;
pub fn side_text(text: &str, side: usize) -> String;          // that side chosen in every conflict
pub enum Resolution { Keep(usize), KeepBoth, Text(String) }
pub fn resolve(text: &str, conflict: usize, resolution: &Resolution) -> Result<String, ResolveError>;
```
Syntax kinds `CONFLICT_MARKER`, `CONFLICT`, `CONFLICT_SIDE`; diagnostic code `conflict`.

#### Task C1: spec, lexer, finder
- [ ] Write "Conflicts" in `docs/grammar.md` (marker rule from the spec: line start, run of 7+ of one of `<|=%+-\>`, then space or end of line; snapshot and git styles).
- [ ] Tests: `finds_snapshot_style`, `finds_git_style_with_base`, `accepts_longer_markers`, `six_chars_is_not_a_marker`, `side_text_picks_each_side`, `resolve_keep_both_joins_sides`.
- [ ] Implement lexer token + `conflict.rs`. Commit.

#### Task C2: parser, diagnostic, fixes, formatter
- [ ] Tests (snapshot): `conflicted_file_round_trips`; `conflict_diagnostic_has_help_and_fixes` (fixes "Keep this side"/"Keep the other side", "Keep both" only for whole items with distinct names); `format_leaves_conflicted_file_alone`; `each_side_parses_cleanly` (via `side_text`).
- [ ] Implement: parser builds `CONFLICT` nodes; `parse_file` emits `error[conflict]` per conflict with help "This file holds two versions of the same lines. Keep one, keep both, or edit between the marker lines."; fixes as `stave_lang` patch text edits like existing fixes. Commit.

### Lane D: `stave-mcp` core and `stave mcp`

**Files:** `crates/stave-mcp/**`, `crates/stave-cli/src/main.rs` (`Mcp` subcommand: `stave mcp` runs the server), root `Cargo.toml`.

**Interfaces produced (`stave_mcp`), used by Lanes G, I, J:**
- `pub struct ServerOptions { pub config_dir: PathBuf, pub cache_dir: PathBuf, pub recent_projects: PathBuf }`, `ServerOptions::from_environment() -> Result<ServerOptions, McpError>` (the XDG folders), `pub async fn serve_stdio(options: ServerOptions) -> Result<(), McpError>`. *(Changed in Lane D: `recent_projects` is the desktop's `recently-used.xbel`, where the window records the projects it opens, so `list_projects` reads it rather than a list under `config_dir`. Later lanes build `ServerOptions` with `from_environment()` or the three-field literal.)*
- `pub struct StaveServer` (rmcp `ServerHandler`), state: `Option<OpenProject { folder: PathBuf }>`, `agent: Actor` from the client's `clientInfo.name` + 4 hex chars from a counter seeded by process id *(no randomness needed)*.
- Tools are registered in `src/tools/mod.rs`, one module per group: `projects.rs` (`list_projects`, `open_project`, `new_project`), `looking.rs` (`check`, `describe`, `grammar`, `std_lookup`), `changing.rs` (`try_patch`, `fixes_for`). Each returns `CallToolResult` with one text content that stands alone plus `structured_content` JSON. Later lanes add `revisions.rs`, `conflicts.rs`, `preview.rs` and register them in `mod.rs`.
- Helper `pub fn project_path(open: &OpenProject, given: &str) -> Result<PathBuf, McpError>` refuses paths escaping the folder.

#### Task D1: server skeleton and `stave mcp`
- [ ] Test `initialize_lists_tools` (in-process rmcp client over a duplex stream): tool names exactly `check, describe, fixes_for, grammar, list_projects, new_project, open_project, std_lookup, try_patch`.
- [ ] Implement; `stave mcp` in stave-cli. Commit.

#### Task D2: tools
- [ ] Tests on `examples/` copied to temp dirs: `open_project_summarises` (parts count, problems); `open_project_refuses_missing_folder`; `check_matches_stave_check_output`; `try_patch_fits_and_writes_nothing` (use `examples/clothing_cabinet` + a shoe-shelf patch like `docs/grammar.md` "Patches"); `try_patch_refused_lists_diagnostics`; `new_project_refuses_non_empty_folder`; `new_project_from_steps_checks` (uses `stave-cli`'s script steps; move `script.rs`'s step runner into `stave-app` as `stave_app::script` so both use it); `grammar_returns_section` (`grammar("Patches")` contains `patch_op`); `std_lookup_finds_kind` (a kind name read from `std/` at test time, so the test names none itself); `project_path_rejects_escape`.
- [ ] Implement over `stave_app::open_with`, `scratch::EditBase::try_patch`, `view::describe`, `problems`. `list_projects` reads the window's recent list in `~/.config/stave/` (find its file in `stave-gtk/src/ui/remember.rs`). Commit.

### Lane E: `stave-link` and the window's D-Bus action

**Files:** `crates/stave-link/**`, `crates/stave-cli/src/main.rs` (`stave mcp install|uninstall` subcommands only, in a separate `mcp_install.rs` module), `crates/stave-gtk/src/ui/app.rs` (+ a new `crates/stave-gtk/src/ui/link.rs`), `docs/gui.md` (the action and the menu item), root `Cargo.toml`.

**Interfaces produced (`stave_link`), used by Lane J:**
```rust
pub enum Host { ClaudeDesktop, ClaudeCode, Codex }
pub struct Installed { pub host: Host, pub how: String }       // what was done, in plain words
pub fn install(hosts: &[Host], stave: &Path) -> Vec<Result<Installed, LinkError>>;
pub fn uninstall(hosts: &[Host]) -> Vec<Result<Installed, LinkError>>;
pub fn found_hosts() -> Vec<Host>;
pub async fn show_in_stave(folder: &Path, patch: Option<&str>) -> Result<Shown, LinkError>; // Shown::Opened | Shown::NotRunning
```
- Claude Desktop: merge `mcpServers.stave = {command, args:["mcp"]}` into `~/.config/Claude/claude_desktop_config.json` (keep other keys). Claude Code: run `claude mcp add --scope user stave -- <stave> mcp` when `claude` is on `PATH`. Codex: add `[mcp_servers.stave]` `command`/`args` to `~/.codex/config.toml` (keep other tables). Paths overridable for tests via `HOME`.
- D-Bus: GApplication action `show-project` with parameter `(ss)` (folder, patch text or empty), invoked through `org.freedesktop.Application.ActivateAction` on `io.github.OceanEilonwy.Stave` at `/io/github/OceanEilonwy/Stave` (zbus). The window opens the folder and, with a patch, starts the existing patch candidate flow so it shows as a ghost (never committed).

#### Task E1: install and uninstall
- [ ] Tests with a temp `HOME`: `installs_claude_desktop_keeping_other_servers`, `installs_codex_keeping_other_tables`, `uninstall_removes_only_stave`, `install_twice_is_idempotent`.
- [ ] Implement; `stave mcp install [--claude-desktop] [--claude-code] [--codex]`, no flag = every found host; print each `Installed.how`. Commit.

#### Task E2: show in Stave
- [ ] Test `show_in_stave_reports_not_running` (no such bus name → `Shown::NotRunning`; skip if no session bus).
- [ ] Implement the client and the window action (relm4 message per `docs/relm4-migration.md`); main menu item "Use with an AI app…" runs `install` for found hosts and shows the result in a dialog. Commit.

---

## Phase 1 integration

Merge lanes A–E into `worktree-ai-integration`; resolve `Cargo.toml` members and `CLAUDE.md`; full `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Phase 2: features (4 lanes in parallel)

### Lane F: `stave-web` viewer

**Files:** `crates/stave-web/**`, `tools/build-viewer.sh`, root `Cargo.toml`. Move any scene-building code `stave-web` needs out of `stave-app` (which can't build for WASM) into `stave-scene` or a new module there; keep `stave_app::view::scene` as a thin call.

**Interfaces produced:** `tools/build-viewer.sh` builds `target/viewer/stave-viewer.html`: one HTML file with the WASM (base64) and JS glue inlined. Page API (JS, set by the host page): `window.staveViewer.show({ files: {path: text}, suggestion?: {path: text}, folder: "/p" })`. Exposed WASM functions: `start(canvas)`, `load(folder, files_json, suggestion_json) -> summary_json` (`folder` is the `folder` passed to `show`; `suggestion_json` is empty or `null` for none), `set_pose(motion_index, amount)`, `set_explode(amount)`, `orbit(dx, dy)`, `zoom(steps)`, `resize(width, height)`, `pick(x, y) -> json` (`{name, what, joined, suggested, replaced}`). Hosts call `window.staveViewer.show`, not these directly. The suggestion is drawn as the project's scene with its ghosts added (tinted by verdict) and what it changes or takes out faded.

#### Task F1: pipeline in WASM over `MemoryFiles`
- [ ] Test (native, `cargo test -p stave-web`): `loads_box_from_memory_files` builds a `SceneGraph` from `examples/box` read into `MemoryFiles`, same node count as `stave_app::view::scene` on disk.
- [ ] Implement; `cargo build -p stave-web --target wasm32-unknown-unknown --release`. Commit.

#### Task F2: drawing and interaction
- [ ] Implement canvas surface via `stave_render::Renderer::for_surface`; orbit drag/zoom; click → `pick`, show part name and joined parts; a slider per motion (`pose_with`); an explode slider; suggestion as ghost (`stave_scene` `Ghosts`); without `navigator.gpu`: message "This preview needs WebGPU." and no crash.
- [ ] `tools/build-viewer.sh` (installs nothing; fails with a clear message if `wasm-bindgen` CLI of the matching version is missing). Test `viewer_html_is_self_contained` (the built file has no `src="http`). Commit.

### Lane G: suggestions, apply, policy in `stave-mcp`

**Files:** `crates/stave-mcp/src/tools/revisions.rs`, registration line in `tools/mod.rs`, `crates/stave-mcp/src/agent_line.rs`; modify `changing.rs` so `try_patch` runs on the agent's line (`History::suggestion_files` or main via `MemoryFiles`).

**Tools:** `suggest(patch, title)`, `list_suggestions`, `list_versions`, `apply_suggestion(id)` (annotation `destructiveHint: true`; policy `off` → refused "Only the person using Stave can apply suggestions in this project."; read `base = History::main()` and its files, replay every patch on them via `EditBase` over `MemoryFiles`, require 0 errors, then `History::apply(id, actor, &base, &merged)`; patch no longer fits → `rebase_suggestion`, report conflicts), `discard_suggestion(id)`, `undo` (`UndoScope::Own`). `open_project` starts versions when off (the host shows the question through the tool's text: the first call returns "Versions are off for this project. Call open_project again with start_versions: true to turn them on." *(my call: the ask goes through the AI app, as the window isn't involved)*).

`new_project` (Lane D) now follows the merge policy (global default from `merge_policy(folder, config_dir)`): under `off` it is refused; otherwise it writes the folder, starts versions and records the first revision as the agent.

- [ ] Tests: `new_project_respects_policy_and_records_revision`; `suggest_then_apply_moves_main`; `apply_refused_under_off_policy`; `apply_refused_when_not_clean`; `apply_replays_after_main_moved` (user save changes another field first; the suggestion still applies); `undo_reverts_own_apply`; `open_project_nested_folder_refused`.
- [ ] Implement. Commit.

### Lane H: the window's versions

**Files:** `crates/stave-app/src/versions.rs` (new: save-merges, catch-up decisions, suggestions list for the window), `crates/stave-gtk/src/ui/versions.rs` (new relm4 panel "Versions" with Suggestions above revisions: Apply, Discard, Undo, "Show" ghosts a suggestion), save path in `crates/stave-gtk/src/ui/files.rs`, `docs/gui.md`.

- [ ] Tests (stave-app): `save_merges_into_main`; `catch_up_reloads_clean_buffers_only` (decision function given dirty set returns which files to write now); `turn_on_versions_prompt_needed` (status Off → prompt).
- [ ] Implement the window: "Turn on versions for this project?" dialog on first save after opening an Off project (Turn on / Not now, remembered per project in `~/.config/stave/`); catch-up after `notify` sees `.stave/history` change; Undo with `UndoScope::Any`; plain words only. GTK tests per `docs/relm4-migration.md`. Commit.

### Lane I: conflicts in app, window and MCP

**Files:** `crates/stave-app/src/conflicts.rs` (problems + fixes from `stave_lang::conflict`; per-side check via `side_text` + `open_with`), `crates/stave-mcp/src/tools/conflicts.rs` (+ registration line), `crates/stave-gtk/src/ui/editor.rs` conflict tint + "Keep this side"/"Keep both" actions (and, from Lane H's Versions panel, "Resolve" on a conflicted suggestion opens its files read from `History::suggestion_files` in the editor and saves back through `write_suggestion_files`), `docs/editing.md`, `docs/gui.md`.

**Tools:** `list_conflicts` (per conflict: file, line range, base and sides text, origin from the revision actors, each side's error count, each side as a patch against the base via `stave_lang` `patch/diff.rs`), `try_resolution(file, conflict, resolution)`, `resolve_conflict(file, conflict, resolution)` (main: writes the file through the one writer when the window isn't holding it dirty, then `History::save` as the agent; suggestion: `write_suggestion_files`).

- [ ] Tests: `list_conflicts_describes_sides_as_patches`; `try_resolution_reports_problems`; `resolve_keep_both_writes_and_records`; app `conflict_fixes_keep_each_side`.
- [ ] Implement. Commit.

## Phase 2 integration

As phase 1.

## Phase 3: preview (2 lanes in parallel)

### Lane J: preview tools and MCP App

**Files:** `crates/stave-mcp/src/tools/preview.rs` (+ registration), `crates/stave-mcp/src/viewer_server.rs`, `crates/stave-mcp/build.rs` or `include_str!` of the built viewer (if the built file is missing at build time, embed a page saying "Viewer not built: run tools/build-viewer.sh").

- **MCP App:** resource `ui://stave/viewer` with MIME `text/html;profile=mcp-app` (check the MCP Apps spec at `github.com/modelcontextprotocol/ext-apps` and follow it exactly); `preview` declares `_meta.ui.resourceUri = "ui://stave/viewer"`; the page receives the tool result (files, suggestion) through the Apps bridge and calls `staveViewer.show`. It calls no tool that changes anything.
- **Localhost viewer:** `tokio` + `hyper`/`axum` on `127.0.0.1:0`, token = 32 hex chars from `getrandom`; `GET /?token=…` the page; `GET /scene?token=…` the JSON; anything else 404, a wrong token 403; `Origin` other than the server's own → 403.
- Tools: `preview(suggestion?)` (text: the link; structured: files), `show_in_stave(suggestion?)` (via `stave_link::show_in_stave`; "Stave isn't running. Open it and ask again." when not running), `render_view(camera?: {yaw, pitch, distance})` (PNG image content from `Renderer::headless` + `encode_png`; text-only error when no adapter).
- [ ] Tests: `viewer_refuses_wrong_token`, `viewer_refuses_other_origin`, `viewer_binds_loopback_only`, `preview_declares_ui_resource`, `render_view_returns_png_or_says_why`.
- [ ] Implement. Commit.

### Lane K: documentation

**Files:** `docs/STATUS.md` ("Editors and AI" rewritten: what now exists), `docs/ai.md` (new: using Stave with Claude Desktop, Claude Code and Codex: `stave mcp install`, what to ask, previews, versions and suggestions, merge policy file format, conflicts), `CLAUDE.md` (final crate order, the five new crates in "Crate boundaries").

- [ ] Write; every command in `docs/ai.md` matches the real CLI (`stave mcp --help`). Commit.

## Final

Whole-branch review (correctness, `CLAUDE.md` rules, spec coverage, security of paths and the localhost server), fixes, full `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `tools/build-viewer.sh`.
