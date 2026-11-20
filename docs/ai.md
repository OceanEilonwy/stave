# Using Stave with an AI app

Stave works with the AI apps you may already use: **Claude Desktop**,
**Claude Code** and **Codex**. Ask in plain words ("open my dresser",
"widen the drawer to 500mm", "add a shoe rack to the dresser") and the app
writes the change as a patch in the `.stave` language (grammar,
"Patches"), Stave checks it exactly as `stave check` does, and you see the
result in 3D before anything is kept. Every change is recorded and can be
undone.

The app's own model does the thinking. Stave sends nothing to any AI
provider: it talks to the app over the app's own channel on your computer
(standard input and output) and, for the preview, a page served on your
computer only. Nothing is ever pushed anywhere unless you ask.

The design is in
[superpowers/specs/2026-11-20-ai-integration-design.md](superpowers/specs/2026-11-20-ai-integration-design.md);
the window's side is in [gui.md](gui.md) ("Starting Stave" and
"Versions"), conflict markers in [grammar.md](grammar.md) ("Conflicts").

## Setting it up

One command sets Stave up in every AI app it finds on this computer:

```
stave mcp install
```

Each app gets a line saying what was done, or why it couldn't be. To set
up only some apps, name them:

```
stave mcp install --claude-desktop
stave mcp install --claude-code
stave mcp install --codex
```

What each one does:

| App | What `stave mcp install` changes |
|---|---|
| Claude Desktop | adds `mcpServers.stave` to `~/.config/Claude/claude_desktop_config.json`, keeping everything else; start Claude Desktop again to use it |
| Claude Code | runs `claude mcp add --scope user stave -- <stave> mcp` (only when `claude` is on your `PATH`) |
| Codex | adds `[mcp_servers.stave]` to `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`), keeping your other tables and comments |

`<stave>` is the full path of the `stave` program you ran. Installing twice
changes nothing. To take Stave out again (with no flag, from every app
found):

```
stave mcp uninstall
stave mcp uninstall --codex
```

In the Stave window, **Help → Use with an AI App…** (also on the start
page's menu) does the same as `stave mcp install` and says in a dialog
what was done in each app.

Each app starts the server itself, as `stave mcp`. You never need to run
it by hand; running it in a terminal just waits for an app to talk to it
on standard input:

```
stave mcp
stave mcp --help
```

The exit code of `install` and `uninstall` is 0 when every app named (or
found) was done, and 1 when one failed or none was found.

## What to ask

Talk about the project in woodworking words. Some things that work:

- "What Stave projects do I have?" and "Open my dresser." The app lists
  the projects you opened lately in the Stave window, or asks for the
  folder.
- "Make a new bedside table, 450 wide, 550 high, 400 deep, in oak, in
  `~/Projects/bedside`." A new project is made in an empty folder, step by
  step the way you would build it in the window.
- "What's wrong with it?" (the problems, as `stave check` prints them,
  with their fixes) and "Describe the left door" (what the window's
  inspector says).
- "Widen the drawer to 500mm", "Make the shelves from walnut", "Add a shoe
  rack under the dresser." The app tries the change first and tells you
  whether it fits and what it would break.
- "Show me." A 3D preview of the suggestion (see "Previews").
- "Apply it", "Discard it", "Undo that."
- "There's a clash in shelf.stave; keep mine." (see "Conflicts").

The app reads the language guide and the standard modules through Stave
(`grammar`, `std_lookup`), so it writes valid `.stave` text.

## How a change gets in

Nothing an AI app does reaches your project without checking clean.

1. **Try.** The app tries a patch on a scratch copy (`try_patch`). Stave
   says whether it fits, the problems it would cause and the files it
   would change. Nothing is written.
2. **Suggest.** A patch that fits, with the design then checking with no
   errors, is recorded as a **suggestion** (`suggest`), on the app's own
   line of changes. Your project folder does not change. More patches can
   be added to the same suggestion.
3. **Look.** You see it in the app's preview, or in the Stave window
   (Versions panel → Show, or `show_in_stave`), as a ghost over the
   design.
4. **Apply.** Applying (`apply_suggestion`, or Apply in the window) tries
   the suggestion's patches again on the project as it is now, so it still
   works if you changed something meanwhile. Only when every patch fits
   and the design checks with no errors does it become the project's
   latest version. If something a patch names is no longer there, the
   suggestion's own files are merged into the project instead, under the
   same check; changes that clash are kept as a conflict on the
   suggestion, to be settled before it can be applied.

Whether the app may apply by itself is your choice: see "Merge policy".

## Versions and suggestions

Versions are every saved state of a project, kept inside the project
folder in `.stave/history/`. The folder itself stays plain files.

- **Turning versions on.** The first time an AI app opens a project whose
  versions are off, it asks you before turning them on; the window asks
  "Turn on versions for this project?" on the first save. Suggestions need
  versions.
- **Saving in the window** records your version and merges it into the
  project's main line. A save is never refused for problems in the design.
- **Everyone works on a line of their own.** You work in the project
  folder; each AI app works on its own line, with no folder of its own, so
  it never touches the files you have open. The main line moves only by
  merges, one at a time. When an app applies a suggestion, the window
  catches up: your clean files are updated at once, and unsaved edits are
  kept until you save, when the two meet. When the window isn't running
  there are no unsaved edits, so the app updates the project's files
  itself (and again when it undoes the apply).
- **Undo.** In the window, Undo takes back the last change by anyone. An
  AI app's `undo` takes back only that app's own last change (a
  suggestion, an apply or a discard), and is refused when you or another
  app have since changed the project on top of it.
- **Discard** throws a suggestion away; undo brings it back.
- **A folder that is already a git repository** (its root) keeps its
  `.git` and history: Stave's versions use it, and the project's main line
  is the branch the folder is on, so your git tools keep working.
- **A folder inside a bigger repository** (the way `examples/` sits inside
  Stave's own) can't have versions. Stave says so and offers to copy the
  project to a folder of its own.
- Versions are on your computer only. Nothing is pushed unless you ask.

Under the hood the versions are a jj (Jujutsu) repository, kept in
`.stave/history/repo` and loaded by Stave through `jj-lib`, with Stave
itself standing in for jj's working copy; there is no `.jj` in the project
folder. The window never names jj, git or the protocol.

## Merge policy

Whether an AI app may apply its own suggestions is set per project in
`.stave/agents.toml` in the project folder, recorded with the project. The
file holds one setting:

```toml
merge_policy = "ask"   # "off", "ask" or "auto"
```

- `off`: apps can suggest; only you apply, from the window. Apps can't
  make new projects or settle conflicts in your files either.
- `ask` (the default): applying is marked as changing the project, and
  the AI app asks you before it does. If you have set the app to approve
  changes on its own, Stave doesn't ask again.
- `auto`: an app may apply any suggestion that checks clean.

A project without the file uses `~/.config/stave/agents.toml` (or
`$XDG_CONFIG_HOME/stave/agents.toml`), in the same format; without that,
`ask`. A missing, unreadable or wrong setting counts as not set. Only you
change the project's settings: a suggestion that would change any file in
`.stave/` is refused, so an app can't raise its own policy. The checks
under "How a change gets in" hold in every mode.

## Previews

The preview shows the design in 3D: rotate and zoom, click a part to see
what it is and what it is joined to, open doors and drawers, pull the
design apart, and see a suggestion as a ghost over the design as it is
now. It never edits.

- **`preview`**: in Claude Desktop, the 3D page shows in the
  conversation. In every app, the answer also carries a link to the same
  page served on your computer only (`http://127.0.0.1:<port>/?token=…`),
  for Claude Code and Codex, which don't show pages. The link works only
  with its token and only from this computer. When the app's own page
  isn't allowed to run the viewer, the page says so: open the link in a
  web browser.
- **`show_in_stave`**: asks the running Stave window to open the project
  and show the suggestion as a ghost, with its card; nothing is written
  unless you press Add ([gui.md](gui.md), "Versions"). When the window
  isn't running, the app says so: open Stave and ask again.
- **`render_view`**: a picture of the design, drawn offscreen, for an app
  that shows images but not pages.

The page needs WebGPU; there is no fallback. Without it, the page says so.
The page is built from `crates/stave-web` by `tools/build-viewer.sh`
(it needs the `wasm32-unknown-unknown` target and the matching
`wasm-bindgen` CLI, and says how to get them). A `stave` built before the
viewer shows a page saying "Viewer not built: run tools/build-viewer.sh".

## Conflicts

When two changes touch the same lines (your saved work and an app's
suggestion, or your own work outside Stave), the file keeps both between
**conflict marker lines** until someone chooses (grammar, "Conflicts"):

```
<<<<<<< conflict 1 of 1
+++++++ your saved work
part shelf : pine { ... }
------- base
part shelf : oak { ... }
+++++++ Claude Desktop: make the shelf from pine
part shelf : walnut { ... }
>>>>>>> conflict 1 of 1 ends
```

A file with conflicts still opens and saves byte for byte, `stave fmt`
leaves it as it is, and `stave check` reports `error[conflict]` and checks
the design once with each side chosen. They are expected to be rare.

- **In the window** each side is tinted, with "Keep yours", "Keep theirs"
  and (when both sides are whole items with different names) "Keep both"
  above the markers. Deleting the marker lines by hand and saving settles
  it too.
- **Through the AI app**: ask it to settle the clash. It lists each
  conflict with where each side came from and what each changes
  (`list_conflicts`), tries a resolution on a scratch copy
  (`try_resolution`: keep one side, keep both, or new text), then writes
  it (`resolve_conflict`). In your files the result must check with no
  errors, and it is recorded as a version by that app. A clash in a
  suggestion is settled in the suggestion, which can then be applied.

## The tools

What an AI app can call. Each answers in plain text that stands on its
own, plus structured content.

| Tool | What it does | Changes anything |
|---|---|---|
| `list_projects` | projects opened lately in the Stave window, then folders the app shares (its roots) that hold a project | no |
| `open_project(path, start_versions?)` | opens a project for this session: parts, problems, versions, suggestions | turns versions on, when asked |
| `new_project(folder, steps)` | makes a new project in an empty folder | yes (refused under `off`) |
| `check` | the problems, with fixes | no |
| `describe(path?)` | the parts tree, or one part as the inspector tells it | no |
| `grammar(topic?)` | a section of the language guide | no |
| `std_lookup(kind)` | a kind of the standard modules, with its documentation | no |
| `try_patch(patch, suggestion?)` | tries patches on a scratch copy | no |
| `fixes_for(problem)` | a problem's fixes, as patches | no |
| `suggest(patch, title, onto?)` | records patches that check clean as a suggestion | the app's own line only |
| `list_suggestions`, `list_versions(limit?)` | the suggestions waiting; the versions, newest first | no |
| `apply_suggestion(id)` | puts a suggestion into the project, under the merge policy | yes |
| `discard_suggestion(id)` | throws a suggestion away | the app's own line only |
| `undo` | takes back this app's own last change | yes |
| `list_conflicts(suggestion?)` | the clashes, each side and where it came from | no |
| `try_resolution(file, conflict, resolution, suggestion?)` | tries settling one clash | no |
| `resolve_conflict(file, conflict, resolution, suggestion?)` | settles one clash and records it | yes (refused under `off`) |
| `preview(suggestion?)` | the 3D page, and its link on this computer | no |
| `show_in_stave(suggestion?)` | shows it in the running Stave window | no |
| `render_view(camera?)` | a picture of the design | no |

There is no cut list tool yet: Stave has no cut list.

## When something goes wrong

- **"No AI app was found."** Install Claude Desktop, Claude Code or Codex,
  or name the app with its flag.
- **The app doesn't list Stave's tools.** Start the app again after
  `stave mcp install`. If you moved or rebuilt `stave` elsewhere, run
  `stave mcp install` again so the app runs the new path.
- **"Versions are off."** Say yes when the app asks to turn them on, or
  turn them on from the window's Versions panel.
- **Apply is refused.** The project's merge policy is `off`, the design
  doesn't check clean with the suggestion, or the suggestion still holds a
  clash. The answer says which.
