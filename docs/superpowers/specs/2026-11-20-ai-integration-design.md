# AI integration: Stave as an MCP server

Design drafted 2026-11-20, from a design conversation with the user. Status:
awaiting approval (revised the same day after a builder check).

## Why

`CLAUDE.md` principle 7 is "Bring your own AI: patches use the same
grammar". The pieces are already there: a patch is written in the `.stave`
grammar (`docs/grammar.md`, "Patches"), `stave edit --patch` tries one on a
scratch copy and the checker judges it, every diagnostic carries fixes as
patches, and one writer commits. What is missing is a way for the AI tools
people already use (Claude Desktop, Claude Code, Codex) to drive that loop,
to show the result, and to let a person and an agent work on the same
project at the same time without trampling each other.

The user wants a beginner to be able to say "open my dresser", "make a new
bedside table, 450 wide", "widen the drawer" or "add a shoe rack to the
dresser" in the AI app they already use, see the result in 3D, and keep or
discard it, with every change recorded and undoable.

## Goals

1. **Integration with Claude Desktop, Claude Code and Codex**, including a
   3D preview: rotate, zoom, click a part to see what it is, open doors and
   drawers (the scene's poses), pull the design apart (exploded view), and
   see an agent's suggestion as a ghost over the current design. The
   preview never edits.
2. **Instructions in plain English work**: open a project, make a new
   project from a description, change a design ("widen the drawer"), add
   to it ("add a shoe rack to the dresser"). The AI turns each into a patch
   or a script; Stave checks it.
3. **Installing is one command**: `stave mcp install` sets Stave up in
   Claude Desktop, Claude Code and Codex.
4. **Composable crates**, following the boundaries in `CLAUDE.md`, with the
   renderer moved out of `stave-gtk` so the desktop, the server and the
   browser share it.
5. **A person and agents work at the same time** on one project, with a
   simple revision system built on jj (Jujutsu): each agent works on its
   own line of changes and applies it to the project's main line.

## Non-goals

- An assistant panel inside the Stave window (out of scope).
- Editing in the preview: no grips, drags or menus there. Changes come only
  from instructions (through patches) or from the Stave window.
- macOS and Windows: Stave is Linux only for now, and so is this.
- Choosing or hosting a model: the host app's model does the work. Stave
  sends nothing to any AI provider itself.
- Remote modules (`github:`, `https:`) and the lockfile: not done yet in
  Stave, not part of this.

## Anti-goals (a failure even if it works)

- An agent's change reaches the main line without checking clean.
- Anything is lost: a person's unsaved edits, a revision, a suggestion.
- A beginner has to learn jj, git, MCP or the terminal to use any of it.
- AI-specific behaviour in the core crates, or a kind named there (the
  standing rule in `CLAUDE.md` still holds).
- Output that differs run to run (determinism still holds).

## Constraints

- Everything in `CLAUDE.md`: Rust standards, no panics in library code,
  exact values, determinism, crate dependency order, "no kind in the core",
  user-facing text in plain woodworking English, `cargo fmt`,
  `cargo clippy --all-targets -- -D warnings` and `cargo test` green.
- The spec comes first: language changes go into `docs/grammar.md` before
  code; behaviour of the window into `docs/gui.md`; editing into
  `docs/editing.md`.
- WebGPU and WASM are confirmed by the user to work in Claude Desktop.
- Claude Code and Codex do not show MCP App pages; every tool's text result
  must be useful on its own.
- The window is moving to relm4 (`docs/relm4-migration.md`); new window code
  follows that contract.

## Approaches considered

**Where the AI runs.** (a) An assistant built into Stave calling a model
API; (b) Stave as an MCP server that the user's own AI app drives; (c) both.
Chosen: (b). It fits "bring your own AI", needs no keys or network code in
Stave, works in three hosts at once, and keeps the core deterministic. The
user ruled (a) out of scope.

**Preview in hosts that can't show pages (Claude Code, Codex).** (A) The
server serves the same WASM viewer on localhost and gives the AI a link;
(B) a tool asks the running Stave window to show the suggestion; (C) a PNG
only. Chosen: **A and B**. A is the same viewer everywhere; B fits how the
user already works. C is a fallback only (see `render_view`).

**Revisions.** (a) Plain git with auto-commits; (b) jj; (c) Stave's own
history. Chosen: **jj**, embedded through `jj-lib`, because each agent can have
its own line of changes, the operation log gives undo for every
step, conflicts are recorded instead of blocking, it is a Rust library (no
binary to install) and it stores to a git repository the user can push.

**Applying an agent's change to main.** (a) jj rebase and text merge;
(b) re-try the agent's patch against main as it is now, through the
existing scratch pipeline, falling back to (a). Chosen: **(b)**: a patch
says what it means (`set drawer.width = 500mm`), so it still applies when
main moved on, where a text merge would conflict.

**May the agent apply to main itself?** (A) never; (B) only when asked,
behind the host's approval prompt; (C) a per-project setting
`off`/`ask`/`auto`. Chosen: **(C)**, default `ask`.

**Who writes to main.** (a) The window is the only writer while it runs,
and agents send their applies to it over D-Bus; (b) every participant,
the window included, works in a fork, and main moves only by merges.
Chosen (by the user): **(b)**. It needs no writer API between processes,
only a lock around moving main; the rare clash becomes conflict markers
the user or their AI resolves.

**How the viewer gets the design.** (a) The server solves and sends a
serialised scene; (b) the viewer receives the project's text and runs the
pipeline itself in WASM. Chosen (my call): **(b)**. Checked on 2026-11-20:
`stave-lang` through `stave-check` and `stave-scene` build for
`wasm32-unknown-unknown` unchanged; only `stave-smt` (z3, smart edits) does
not, and the viewer doesn't need it. No new data format, one source of
truth, and opening, exploding and ghosts are the code the window uses.

## The design

### Crates

New crates and where they sit in the dependency order:

| Crate | Depends on | Holds |
|---|---|---|
| `stave-render` | `stave-geom`, `stave-scene`, wgpu | The renderer moved out of `stave-gtk/src/render` (`mod.rs`, `meshes.rs`, `shader.wgsl`, `profile.rs`): draws a scene to a wgpu texture or surface. No GTK. Async device setup (no `pollster` inside); `web-time` in place of `std::time::Instant`. |
| `stave-history` | `jj-lib` (pinned), `stave-lang` (for conflict text only if needed) | The revision system: open or start a project's repository in `.stave/history/`, snapshot the project's files as a revision and write a revision's files back (Stave is the working copy), list revisions, a line of changes per agent, apply, discard, undo (operation log), conflicts. No jj type in its public API. |
| `stave-web` | `stave-lang` … `stave-scene`, `stave-render`, `wasm-bindgen` | The browser viewer, built as WASM. |
| `stave-mcp` | `stave-app`, `stave-history`, `stave-render` | The MCP server: tools, the MCP App page, the localhost viewer server, the "show in Stave" call, the merge policy. |

Changes to existing crates:

- `stave-lang`: conflict markers (below).
- `stave-app`: conflicts as problems with fixes; a save merges the
  window's fork into main through `stave-history`; suggestions from agents as candidates the
  window can show; the merge (re-try a suggestion's patch on main).
- `stave-gtk`: uses `stave-render` (keeps `dmabuf.rs` and `readback.rs` as
  its way to the screen); shows suggestions, revisions and conflicts;
  answers "show in Stave" over D-Bus; catches up with main when someone
  else merges.
- `stave-cli`: `stave mcp` (runs the server over stdio) and
  `stave mcp install`.
- `CLAUDE.md`: the dependency order gains the four crates; "Only
  `stave-gtk` may depend on GTK, libadwaita or wgpu" becomes "only
  `stave-gtk` may depend on GTK or libadwaita; only `stave-render`,
  `stave-web` and `stave-gtk` may depend on wgpu".

### Revisions (`stave-history`)

**Where it lives.** The whole repository is kept in `.stave/history/` in
the project folder; the folder itself stays plain files, with no `.jj`.
jj-lib always puts a working copy's `.jj` in the working copy's root
(`workspace.rs`, `workspace_root.join(".jj")`), so Stave doesn't use a jj
working copy for the folder: it loads the repository from
`.stave/history/` and acts as the working copy itself, building a tree
from the project's files to commit and writing merged files back. The
files that count are every file in the folder except `.stave/history/`
and what `.gitignore` excludes *(my call)*. The cost: `jj` can't be run in
the folder directly, only as `jj -R .stave/history …`.

**Forks and main.** Everyone who changes a project works in their own fork
(a line of commits in the repository), and **main** (the project's main
line) moves only by merges:

- **The window's fork is the project folder** the user has open. The user
  edits there as now; **saving merges** the saved files into main. A save
  is never refused for errors: beginners save half-finished designs, so
  the checker does not gate the user's own saves.
- **Each agent's fork is a line of commits**, with no folder of its own,
  so an agent never touches the files the user has open. Checking a patch
  uses scratch space in `~/.cache/stave/workspaces/<project>/<agent>`,
  which is disposable and rebuilt from the repository when missing.
- **An agent** is one MCP connection, named by the host's `clientInfo`
  ("Claude Desktop", "Codex") plus a short id. *(My call.)*
- Each agent request becomes a **suggestion**: one or more revisions on
  its own line, starting from main's latest revision, each carrying the
  patch that made it. Suggestions live in the repository (a bookmark per
  suggestion), so they survive restarts and a wiped cache. *(My call.)*
- **Catching up.** When main moves (someone else merged), the window's
  fork is rebased onto it: at once when the window has no unsaved edits
  (the files change and the window reloads its clean buffers, as it does
  now for a file changed on disk), otherwise at the next save. A clash
  leaves conflict markers in the user's files (see "Conflict markers");
  the user resolves them in the editor or asks their AI to, with the
  conflict tools. This is expected to be rare.
- **Applying a suggestion** (an agent's merge): re-try its patches, in
  order, against main as it is now, on a scratch copy; if the design checks
  with 0 errors, write the result into main and record a revision labelled
  with the agent and the request. If a patch no longer fits, rebase the
  suggestion's line onto main with jj; a text conflict then stays on the
  suggestion, in the agent's fork, where the agent (through the conflict
  tools) or the user (opening the suggestion in the editor) resolves it
  before it can be applied. Refused when the result doesn't check clean or
  the suggestion still holds conflicts.
- **One lock** guards the step that moves main (a lock file in the
  repository's own folder, held for the merge only), so two merges at the
  same moment (the window and an agent, or two hosts) queue instead of
  racing. No other coordination between processes is needed; the window
  and the MCP server talk over D-Bus only for `show_in_stave`.
- **Undo** reverts an operation through jj's operation log. The window's
  Undo can revert any operation. An agent's `undo` reverts only that
  agent's own last operation and is refused when a later operation by
  someone else depends on it (Stave records which operations each agent
  made). *(My call.)*
- **Discard** abandons a suggestion.
- The user sees plain words only: Versions, Suggestions, Apply, Discard,
  Undo. jj is never named in the window.

**Starting versions.** The first time a project is opened for an agent (or
the user turns versions on), Stave asks once: "Turn on versions for this
project?".

- A folder that isn't a repository gets `.stave/history/` only (jj with
  its git data kept inside); pushing to GitHub still works, from Stave.
- A folder that is already the root of a git repository keeps its `.git`
  and its history: the repository in `.stave/history/` uses that `.git` as
  its store (jj's "external git" setup), and main is the branch the folder
  is on, so the user's git tools keep working.
- A folder inside a bigger repository (as `examples/` sits inside Stave's
  own) is refused in plain words, with an offer to copy the project to a
  folder of its own. Nested repositories are not made.
- Revisions are local. Nothing is pushed unless the user asks.

### Merge policy

A per-project setting, `off`, `ask` (default) or `auto`, kept beside the
project, not in a `.stave` file (*my call:* `.stave/agents.toml` in the
project folder, recorded with the project). A global default in
`~/.config/stave/agents.toml` (itself defaulting to `ask`) covers a
project without one, `new_project` included.

- `off`: agents can suggest; only the user applies, from the window.
- `ask`: the `apply_suggestion` tool is marked as changing the project
  (MCP `destructiveHint`) and the host's own approval decides. When the
  user has set their host to approve automatically, Stave does not ask
  again.
- `auto`: an agent may apply any suggestion that checks clean.

The guards in "Revisions" hold in every mode.

### Conflict markers in the language

Spec first: a "Conflicts" section in `docs/grammar.md`.

- **Lexer**: a line that starts with a run of 7 or more of one of `<`,
  `|`, `=`, `%`, `+`, `-`, `\`, `>`, followed by a space or the end of the
  line, is a `CONFLICT_MARKER` token. Only at the start of a line; no
  `.stave` line can start that way, so nothing valid changes. Any run
  length of 7 or more is accepted (jj lengthens markers when a file holds a
  run of 7).
- **Styles**: Stave's repositories use jj's `snapshot` marker style (every
  side written in full). Git's 2-sided style (`<<<<<<<`, `|||||||`,
  `=======`, `>>>>>>>`) is accepted too.
- **Parser**: a `CONFLICT` node with a node per side and the base; the file
  still round-trips byte for byte. For checking, the file is read once per
  side (each side chosen in every conflict), so each side parses, solves
  and checks.
- **Diagnostic** `error[conflict]`: names the clashing changes in plain
  words, with a help line; its fixes (as `Problem::fixes`): keep this side,
  keep the other, keep both (only when both sides are whole items whose
  names don't clash). The formatter leaves a file with conflicts as it is.
- **Where markers appear**: in the user's files when their fork catches up
  with main and clashes; in a suggestion's files when its rebase clashes;
  or from the user's own outside jj or git work.
- **Window**: each side tinted, "Keep this side" / "Keep both" beside the
  markers; deleting markers by hand and saving also resolves (jj notices on
  the next record). The 3D view can show either side as a ghost.

### The MCP server (`stave-mcp`)

Runs as `stave mcp` over stdio. Tools (each returns plain text that stands
on its own, plus structured content; the preview ones also link the page):

Projects

- `list_projects`: recent projects (from the window's own list in
  `~/.config/stave/`) and any folder the host shares.
- `open_project(path)`: opens it for this session; starts its repository if
  needed; returns a summary (parts, problems, revisions, suggestions).
- `new_project(folder, description fields)`: makes a new project in an
  empty folder from the same steps as `stave edit --script` (`new`,
  `stock`, `import`, …); refuses a folder that isn't empty. It counts as an
  apply, so the merge policy applies.

Looking

- `check`: problems as `stave check` prints them, with fixes.
- `describe(path?)`: the parts tree, or one item as the inspector tells it.
- `cut_list`.
- `grammar(topic)` and `std_lookup(kind)`: the language and the standard
  modules, read from `docs/grammar.md` and `std/`, so the AI writes valid
  patches.

Changing (always into the agent's own line, never main)

- `try_patch(patch)`: applies it on a scratch copy of the agent's line and
  returns the verdict, problems, blame and what changed. Writes nothing.
- `suggest(patch, title)`: records a patch that checks clean as (part of)
  a suggestion.
- `fixes_for(problem)`: a problem's fixes as patches.

Revisions

- `list_suggestions`, `list_versions`.
- `apply_suggestion(id)`: the merge, under the merge policy.
- `discard_suggestion(id)`, `undo`.

Conflicts

- `list_conflicts`: each conflict's file and span, the base and each side,
  where each side came from (which agent, which request, or the user), each
  side's check result, and each side as a patch against the base (from
  `stave-lang`'s `patch/diff.rs`), so the AI reads "yours sets
  `drawer.width = 450mm`; the suggestion sets `500mm`".
- `try_resolution(conflict, keep side | text)`: checks the result on a
  scratch copy; returns problems.
- `resolve_conflict(conflict, keep side | text)`: writes through the one
  writer; records a revision.

Preview

- `preview(suggestion?)`: in Claude Desktop, the MCP App page; in every
  host, a link to the localhost viewer showing the same thing.
- `show_in_stave(suggestion?)`: asks the running Stave window (D-Bus, the
  application `io.github.OceanEilonwy.Stave`) to open the project and show
  the suggestion as a ghost; says so plainly when the window isn't
  running.
- `render_view(camera?)`: a PNG drawn offscreen with `stave-render`, for a
  host that shows images but not pages.

### The viewer (`stave-web`)

One WASM build, used two ways: as the MCP App page (a `ui://` resource,
everything inlined, so the sandbox needs no outside origin) and as the
localhost page `stave-mcp` serves (bound to `127.0.0.1` only, a random
port, a per-session token in the link).

- **WebGPU is required**; there is no WebGL fallback. Without WebGPU the
  page says so plainly and links `render_view`'s pictures.
- Receives the project's files (and the suggestion's, when there is one);
  runs parse, resolve, solve and scene in WASM; draws with `stave-render`
  on WebGPU into a canvas. `std/` is embedded.
- Rotate and zoom (the scene's orbit camera); click a part to see its name
  and what it is joined to (picking); open doors, drawers and hinges (the
  scene's poses, a slider per motion as the window has); pull apart (the
  exploded view); the suggestion as a ghost over main with its verdict;
  problems tinted as in the window.
- Read only: it never calls a tool that changes anything.

### Installing

`stave mcp install` writes the server entry for each host it finds and
prints what it did:

- Claude Desktop: its MCP config file on Linux.
- Claude Code: `claude mcp add stave -- stave mcp` (user scope).
- Codex: `codex mcp add stave -- stave mcp`.

Flags pick hosts (`--claude-desktop`, `--claude-code`, `--codex`);
`stave mcp uninstall` reverses it. The window's main menu gets "Use with
an AI app…", which runs the same thing and shows the result.

## Order of work (for the plan)

Lanes that can run in parallel once the shared interfaces are written:

1. `stave-render` extraction (and `stave-gtk` moved onto it).
2. `stave-history` on `jj-lib`.
3. Conflict markers: grammar spec, lexer, parser, diagnostic, fixes.
4. `stave-mcp` tools over `stave-app` (projects, looking, changing).
5. `stave-web` viewer (after lane 1's interface is fixed).

Then: suggestions, apply and merge policy (needs 2 and 4); conflict tools
(needs 2, 3, 4); preview tools and the MCP App page (needs 4, 5);
`show_in_stave` and the window's Suggestions, Versions and conflict views;
`stave mcp install`.

## Left to the builder

- Where a suggestion's patch is kept (revision description or a file).
- The exact MCP Rust SDK; the HTTP server for the localhost viewer.
- Bundle size work for the WASM build (`wasm-opt`, size profile).
- The D-Bus interface's exact shape (a GApplication action is enough).
- How the viewer reaches code that lives in `stave-app` today (building
  the scene for a project, opening a project): `stave-app` can't build for
  WASM (z3 through `stave-smt`, `notify`, gettext), so that code moves
  below it or behind a feature.
- The lock file's exact form and where the operations each agent made
  are recorded.
