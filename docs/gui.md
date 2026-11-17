# The desktop window

How `stave view` (the `stave-gtk` binary) is put together, the contract
its views share, and where the views still to come slot in. The design is
the canvas in `target/canvas/project/` (Main, MainDark, Problems,
Exploded, JointFocus, Hinge, FilesMode); libadwaita and the GNOME HIG
settle what it leaves open.

## Seeing the real window

`tools/window-shot` writes a PNG of the real GTK window, with no screen:

```
cargo build -p stave-gtk
tools/window-shot main.png examples/clothing_cabinet --theme light --select 'door_l.stile[hinge]'
tools/window-shot dark.png examples/clothing_cabinet --theme dark
tools/window-shot busy.png tests/broken/door_hits_side --checking --problems
tools/window-shot narrow.png examples/clothing_cabinet --size 760x900
```

It starts a private Broadway display server (`gtk4-broadwayd`, part of
GTK 4: no screen, browser or root needed), runs `stave-gtk FOLDER
--window-screenshot OUT.png ...` on it and stops it. The window draws its
own widget tree into the PNG through `gtk::WidgetPaintable` and GSK's Cairo
renderer once the 3D view has drawn a frame of its final size. For
pictures that look the same on every machine it uses GNOME's default font
(Adwaita Sans 11) at 96 dpi, the Adwaita icons and no animations, and the
window is exactly `--size` (default 1440x900) whatever the display's size.
Set `STAVE_GTK` to use another binary (`CARGO_TARGET_DIR` is honoured).

Window options (they work without `--window-screenshot` too): `--theme
system|light|dark`, `--view parts|cut-list|files`, `--select NAME` (a
part, piece of hardware or assembly), `--focus NAME` (a joint or a piece
of hardware), `--diagnostic N` (select the Nth problem), `--problems` (open
the problems list), `--scene NAME`, `--explode AMOUNT`, `--size WxH`.
Screenshot only: `--checking` holds a second check running (the pill's
spinner), `--building` presses the build button and holds its check.

The Files view has its own (with `--view files`): `--file FILE[:LINE]`
opens a file at a line (relative to the folder, or `std/joinery/dado.stave`),
`--cursor WORD` puts the cursor on the first `WORD` on that line, and once
the names are worked out `--peek` peeks at its definition, `--hover` shows
its card, `--uses` lists its uses, `--follow` follows it (F12), `--type
TEXT` types at the cursor, and `--text-only` hides the 3D view:

```
tools/window-shot peek.png examples/clothing_cabinet --view files --file doors.stave:9 --cursor FramePanelDoor --peek
tools/window-shot typed.png examples/clothing_cabinet --view files --file doors.stave:11 --cursor side_l --type q
tools/window-shot linked.png examples/clothing_cabinet --view files --select 'door_l.stile[hinge]'
```

To check that "Follow system" follows the system, set the preference where
libadwaita reads it (`ADW_DEBUG_COLOR_SCHEME` stands in for the desktop's
settings portal, which otherwise answers with this desktop's own choice):

```
ADW_DEBUG_COLOR_SCHEME=prefer-light tools/window-shot light.png examples/clothing_cabinet --theme system
ADW_DEBUG_COLOR_SCHEME=prefer-dark  tools/window-shot dark.png  examples/clothing_cabinet --theme system
```

## Layers

All behaviour lives in `stave-app`; `stave-gtk` only lays out widgets and
forwards what people do.

| Crate | Module | What |
|---|---|---|
| `stave-app` | `session` | The shared state and its commands and events (below). |
| `stave-ide` | (crate) | The navigation layer the editor uses and `stave-lsp` will (below): `Analysis`, `Buffers`, `Overlay`, `Document`. Depends on `stave-lang`, `stave-modules` and `stave-schema` only. |
| `stave-app` | `check` | One check as a value that crosses threads (`check::run`, `check::run_with` for open buffers, `Checked`, `Outcome`), and the folder watcher with its debounce (`Watcher`, `RECHECK_DELAY` = 250 ms). |
| `stave-app` | `link` | The text and the 3D view share one selection: `selection_for` (the part, assembly or hardware a name means), `placed_at` (the line that places a selection), `chip` ("Linked to the text · door_l, line 9"). |
| `stave-app` | `inspect` | The parts tree (`parts_tree`, `PartsTree::filtered`) and the inspector's account of a selection (`inspect`), with the declaration it is written in, highlighted (`snippet`, `highlight`). |
| `stave-app` | `focus`, `view` | Focus mode, problems as rows, posing at a contact (unchanged). |
| `stave-gtk` | `ui/mod.rs` | The window: owns the `Session`, dispatches commands, redraws on events, runs checks on a worker thread, and keeps what only the 3D view needs (camera, renderer, focus mode's scrub, the "Open it" slider). |
| `stave-gtk` | `ui/header.rs`, `tree.rs`, `inspector.rs`, `chrome.rs` | The title bar; the parts sidebar; the inspector; the status bar, motion bar, problems list and placeholder pages. |
| `stave-gtk` | `ui/files.rs`, `editor.rs` | The Files view: the file sidebar, tabs, breadcrumbs, the uses panel and the 3D view's slot; the editor (a `GtkTextView` with its gutter, tags, lenses, peek and hover card). |
| `stave-gtk` | `ui/icons.rs`, `style.rs` | The design's icons drawn from SVG path data in the text colour; one stylesheet whose colours are all libadwaita's named colours (`@headerbar_bg_color`, `@sidebar_bg_color`, `@card_bg_color`, `@accent_bg_color`, `@success_color`, ...), so light and dark both come from it and follow the style manager the moment it changes; only the code block is dark in both. |

## The session contract

`stave_app::session::Session` is one open project. It holds no GTK types
and is tested headless (`crates/stave-app/tests/session.rs`).

**State** (read with accessors):

- `view() -> View`: `Parts`, `CutList` or `Files`.
- `selection() -> Option<&Selection>`, where `Selection` is one of
  `Part(PartId)`, `Hardware(HardwareId)`, `Joint { joint, pair }`,
  `Assembly(AssemblyId)`, `Location(SourceLocation)` (a file path as the
  source map names it, and a `LineNumber` from 1) or
  `Problem(ProblemIndex)` (an index into `problems()`).
- `focus() -> Option<focus::Target>`: focus mode on a joint pair or a piece
  of hardware (always with the matching selection).
- `place() -> &Place`: view, selection and focus together; one entry of
  the history. `can_go_back()`, `can_go_forward()` (forward only after
  going back).
- `scene()`, `scene_index()`, `scene_entry()`: the scene by name;
  `explode() -> Explode` (0 to 1, validated).
- `sidebar() -> Sidebar`: `Parts` (the tree) or `Problems` (the list).
- `check_status() -> &CheckStatus`: `Idle` (never checked),
  `Checking { last: Option<Verdict> }` or `Done(Verdict)`, where a
  `Verdict` is `Checked(Summary)` (errors, warnings, files, elapsed) or
  `CantOpen(message)`. `pill()` gives the title bar's pill (`Tone`:
  `Unknown`, `Clean`, `Warnings`, `Errors`; `label`: "No problems",
  "3 problems"; `Activity`: `Idle` or `Checking`, keeping the last tone and
  label while checking); `line()` the status bar's words.
- `build() -> Build`: `Idle` or `Building`.
- `latest()`: the last check's result (its `problems()` are the current
  problems). `shown()`, `drawable()`: the last result that solved, which
  the tree, the inspector and the 3D view show; `showing_older()` when the
  latest didn't solve.

**Commands** (`apply(Command) -> Vec<Event>`): `ShowView(View)`,
`Select(Option<Selection>)`, `Focus(Target)`, `LeaveFocus`, `Back`,
`Forward`, `GoToSource(SourceLocation)` (Files view, location selected),
`ShowScene(name)` (with the scene's own explode amount), `SetExplode`,
`ShowSidebar(Sidebar)`, `Check`, `Build`, and `Link(Option<Selection>)`:
the editor's cursor rests on a name, so the selection changes where the
window is without a history entry (the 3D view follows the text).

Every change of place (view, selection, focus, a source link) is pushed on
the back list (at most 100) and clears the forward list; selecting what is
already selected changes nothing.

**Events** (sorted, without repeats): `ViewChanged`, `SelectionChanged`,
`FocusChanged`, `HistoryChanged`, `SceneChanged`, `ExplodeChanged`,
`SidebarChanged`, `CheckStatusChanged`, `BuildChanged`, `ResultChanged`,
`CheckStarted(CheckTicket)` and `Notice(Notice)`. A view redraws what an
event names; the window takes `ResultChanged` first so the others see the
new result.

**Checks.** `Command::Check` (sent by the watcher after a save, by
Ctrl+R, and once at start) returns `CheckStarted(ticket)` unless one is
running, in which case another is queued and starts when it finishes. The
window runs `check::run(folder)` on a worker thread and hands the outcome
to `finish_check(ticket, outcome)`. A stale ticket's outcome is dropped.
The new result replaces the old one whole (`Rc<Checked>`); the selection,
focus and every history entry are carried to the new ids by name (a
problem by its code, message and place), and anything gone is let go.
`Command::Build` checks first and, when that check is done, gives
`Notice::BuildPlanNotBuilt` ("The build plan (PDF) isn't built yet"): the
PDF doesn't exist yet, and the button says so honestly.

## The window

```
┌ title bar ── sidebar · back · forward · open · title ─ Parts|Cut list|Files ─ scene ▾ · pill · hammer · menu ┐
│ ┌ sidebar ──────┐ ┌ 3D view ─────────────────────────────┐ ┌ inspector ─────┐ │
│ │ Parts tree    │ │ [view-toolbar]        [camera-toolbar]│ │ details, focus │ │
│ │  or problems  │ │                                       │ │ panel, or the  │ │
│ │  list         │ │          motion bar ("Open it")       │ │ problem slot   │ │
│ └───────────────┘ └───────────────────────────────────────┘ └────────────────┘ │
└ status bar ── ● Checked 5 files · 0 errors · 0 warnings · 26 ms   Watching for changes   Selected: … ┘
```

- **Title bar** (`Header`): back and forward are `win.back`/`win.forward`
  (Alt+←/→); the switcher is an `AdwToggleGroup` named by `View::word`;
  the scene menu lists the scene titles; the pill is `win.problems`
  (Ctrl+E, toggles `Sidebar::Problems`); the hammer is `win.build`
  (Ctrl+B), disabled while building or while the design has errors. The
  main menu opens a project folder (Ctrl+O), checks again (Ctrl+R) and
  sets the style (follow system, light, dark).
- **Parts sidebar** (`TreeSidebar`): the carcass, then assemblies with
  their motion ("turns 110°", "slides out 500 mm", "mirror of door_l").
  Rows that are the same part (one stock and size under one name, mirror
  pairs) share a row (`rail[bottom, top]`, `foot[4]`, `side_l, side_r`)
  until one of them is selected, when the group shows its members one by
  one. The assembly holding the selection opens. Ctrl+F searches.
- **Inspector**: eyebrow ("PART · IN door_l"), title, doc comment, rows
  (stock, finished size, grain, moves), joints and cuts (each row opens
  focus mode: `Command::Focus`), and "Defined in the text": the snippet of
  the declaration in the project's own files (a part of a library
  component is shown by its assembly's declaration) with a file:line link
  that sends `Command::GoToSource`.
- **3D view**: click to select (`Command::Select`), F frames, Escape
  leaves focus or clears the selection. The backdrop follows the style
  (#e8e5df light, #242428 dark, chosen when each frame is drawn and
  redrawn when the style changes). The floating motion bar opens the
  selection's door or drawer, or swings a selected problem's motion.
- **Narrow windows**: below 1100 sp the parts sidebar folds away (the
  sidebar button shows it over the view); below 860 sp the inspector
  does too (it slides in while something is selected), the switcher and
  scene menu drop their words and the subtitle goes; below 600 sp the
  title, the open button and forward go too.

## Slots for the next lanes

- **Lane 2, the 3D view's toolbar**: two empty, hidden `gtk::Box`es are
  overlaid on the view, named `view-toolbar` (top left: Pull apart and its
  slider, Exploded.dc.html) and `camera-toolbar` (top right: fit, front,
  turn). The explode amount is `Session::explode` (`Command::SetExplode`,
  `Event::ExplodeChanged` reposes); fitting is the window's `win.frame`
  action.
- **Lane 2, the Files view**: built; see "The Files view" below.
- **Lane 3, problems**: the pill already toggles `Sidebar::Problems`; the
  sidebar's `problems` page (`chrome::ProblemsList`, a plain list) is
  replaced by the problems tree and filters (Problems.dc.html), selecting
  with `Selection::Problem`. A selected problem with a contact is posed
  and tinted already, and the motion bar swings it; the inspector's
  `problem` page (`Inspector::show_problem`, message and help only) is
  replaced by the problem's details and ways to fix it.
- **Cut list**: the `cut-list` page is a "Cut list is coming" status page.

## The Files view

FilesMode.dc.html: the text is the design, and the 3D view beside it
follows it. Pictures in `docs/screenshots/files/` (`linked.png`,
`peek.png`, `hover.png`, `uses_dark.png`, `problem.png`, `typed_dark.png`,
`selected_in_3d_dark.png`, `narrow.png`).

```
┌ files ───────┐ ┌ tabs ─ doors.stave • ─ carcass.stave ──────── Text|Both ┐ ┌ 3D view ─────────┐
│ Go to file…  │ │ doors.stave › assembly door_l › mount                    │ │                  │
│ PROJECT      │ │  8                                                       │ │                  │
│  cabinet…    │ │    2 uses · show in 3D                                   │ │                  │
│  casework/   │ │  9 assembly door_l : casework.FramePanelDoor {           │ │                  │
│ STANDARD LIB │ │    ┌ casework/frame_panel_door.stave · component …  Open┐│ │ ⛓ Linked to the  │
│  joinery     │ │    └ (peek)                                             ┘│ │   text · door_l, │
│  hardware    │ │ 10   opening: door_bay[left]                             │ │   line 9         │
│              │ │ [uses panel]                                             │ │                  │
│              │ │ Ctrl+click follows a name · Alt+F12 peeks · …    Line 9  │ │                  │
└──────────────┘ └──────────────────────────────────────────────────────────┘ └──────────────────┘
```

### The navigation layer (`stave-ide`)

A crate between `stave-model` and `stave-app` in the dependency order
(it needs only `stave-lang`, `stave-modules` and `stave-schema`), with no
GTK and no language-server types, so `stave-lsp` can use it unchanged.
Positions are byte offsets (`FilePosition { file, offset }`); lines are
worked out at the edge (`Location { document, span, line, column }`,
`Location::label` gives `casework/frame_panel_door.stave:4`).

- `Analysis::load(folder, &dyn FileSource)` loads the project (its own
  folders, the folders it imports and the standard library), parses every
  file's lossless tree and indexes it: every declaration is a `Symbol`
  (kind, name, span, container, doc comment, one-line signature), and every
  name written is a `Reference` to one, resolved by the grammar's
  namespaces (the enclosing component or kind, the module, this file's
  imports, the prelude; members of modules and of an assembly's
  component; labels in `[ ]`; named arguments of a kind; fields of an
  assembly as its component's params). Enum words, built-in functions and
  fixed call heads come from `stave-schema`, never items. Broken items
  index as far as they parse, so a half-typed line loses only itself.
- Queries: `symbol_at`, `definition`, `declaration`, `uses` (each `Use`
  says what it does: "declared", "door_r mirrors it", "scene all_open
  poses it", "assembly door_l · hinge_side"), `hover` (kind, signature,
  doc comment, where; built-in functions and enum words too),
  `breadcrumbs` (`doors.stave › assembly door_l › mount`), `tokens`
  (`TokenClass`: keyword, comment, doc, text, number, function, type,
  module, enum word, field, item of a kind), `lenses` ("2 uses", "show in
  3D" for parts, assemblies and hardware), `search` ("Go to file or name")
  and `diagnostics` (the fast tier: syntax, imports, module headers).
- `Buffers`: open documents' text and their text on disk; `is_dirty`,
  `save` (written to a temporary file beside it and moved over it),
  `revert`, `reload` (a change on disk reloads a clean buffer and keeps
  unsaved changes), and `overlay()`: an `Overlay` (a `FileSource`) that
  reads unsaved buffers in place of their files and the disk for the
  rest. It crosses threads, so checks read what the editor shows.
- `Document`: `File(path)` or `Std { module, file }`, read only, with URIs
  `file:///…` and `stave-std:///joinery/dado.stave`.

Incrementality: an `Analysis` is a value built again a moment (150 ms)
after typing pauses, on a worker thread: about 3 ms for the cabinet in
release (60 ms in a debug build). Caching parses per file is the next
step and changes nothing in the interface.

### The page

- **Sidebar**: PROJECT lists every `.stave` file of the folder and its
  subfolders (`casework/`, `hardware/`), a dot on unsaved ones; STANDARD
  LIBRARY lists the three modules, read only, each opening to its files.
  "Go to file or name" (Ctrl+F in this view) lists files, then names,
  the project's own first; Enter opens the first.
- **Tabs** with a dot while unsaved; Text | Both on the right. Both
  moves the window's one 3D view (`view_stack`, with its motion bar and
  the toolbar slots) beside the editor; Text and the other views move it
  back. The split starts with about 460 px for the 3D view; below 860 sp
  the editor sits above the 3D view, and below 1100 sp the file sidebar
  folds away (the title bar's sidebar button shows it).
- **Breadcrumbs** follow the cursor. **Footer**: the shortcuts and the
  line.
- **Editor**: colours from the semantic tokens (one source of truth: the
  grammar's words through `stave-lang` and `stave-schema`), line numbers
  with the cursor's in the accent colour, the cursor's line tinted, the
  name under the cursor and its uses marked (its declaration stronger),
  wavy underlines (red errors, amber warnings) from the fast tier and the
  last check, a lens above each part, assembly, piece of hardware,
  component and kind ("2 uses" lists them; "show in 3D" selects it and
  switches to Both).
- **Keys**: F12 or Ctrl+click follows a name; Alt+F12 peeks at its
  definition under the line (Open goes there; Escape closes); Shift+F12
  lists its uses in the panel under the editor (a row goes there);
  Alt+←/→ walk the window's history; Ctrl+S saves; Ctrl+Shift+I lays the
  file out with `stave fmt` (never done unasked; Undo takes it back).
  Resting the pointer on a word shows its card: the problem there with
  its help line, or what the name is, its doc comment and where it is
  declared.
- **History**: following a name or a use first records where the cursor
  was (`Command::Link` with that `Selection::Location`), then sends
  `Command::GoToSource`, so Back returns to it. The page shows any
  `Selection::Location` (a "Defined in the text" link included) by
  opening its file at its line.
- **Linked selection**: 120 ms after the cursor rests on a name the page
  tells the window, which asks `link::selection_for` and sends
  `Command::Link`: the 3D view selects the part, the piece of hardware or
  the whole assembly (every part and piece of hardware in it drawn
  selected). A name the 3D view can't show leaves the selection alone.
  Selecting in the 3D view or the parts tree moves the editor to
  `link::placed_at` (a part of a library component: its assembly's line)
  without moving the selection back, and the chip on the 3D view names
  it. A new check's result never moves the editor.
- **Editing**: the editor edits the real files. Every change updates the
  document's buffer; 150 ms later the names are worked out again, and
  700 ms later the design is checked from the buffers (`check::run_with`
  with the page's overlay; the underlines go on only if the text is still
  the text checked). Saving writes the file; the watcher's check then
  reads the same text. Tabs keep their text (and undo) when switching
  files or views; closing the window or opening another folder with
  unsaved changes asks: Save, Don't Save or Cancel. A file changed on disk
  by something else reloads when its buffer is clean and keeps unsaved
  changes otherwise (with a toast).

**GtkSourceView 5** isn't installed on this machine (only GtkSourceView 4,
the GTK 3 one; no `gtksourceview5` package), so the editor is a plain
`GtkTextView`: the lenses, the peek and the hover card are overlay
children (made once and hidden when not used: GTK can't take an overlay
back out of a text view), placed in room that a tag's
`pixels-above-lines` makes for them, and the line numbers are a gutter
widget of our own. Moving to GtkSourceView later would change only
`editor.rs`.

**Differences from the mockup**: the 3D view's "Pull apart" toolbar is
lane 3's slot; the current line has a tint but not the mockup's blue bar
at its left edge; the peek shows the doc comment, the declaration and the
four lines after it (the mockup skips to a later line); the editor's
monospace font is Adwaita Mono where JetBrains Mono isn't installed.
