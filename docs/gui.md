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
| `stave-app` | `check` | One check as a value that crosses threads (`check::run`, `Checked`, `Outcome`), and the folder watcher with its debounce (`Watcher`, `RECHECK_DELAY` = 250 ms). |
| `stave-app` | `inspect` | The parts tree (`parts_tree`, `PartsTree::filtered`) and the inspector's account of a selection (`inspect`), with the declaration it is written in, highlighted (`snippet`, `highlight`). |
| `stave-app` | `focus`, `view` | Focus mode, problems as rows, posing at a contact (unchanged). |
| `stave-gtk` | `ui/mod.rs` | The window: owns the `Session`, dispatches commands, redraws on events, runs checks on a worker thread, and keeps what only the 3D view needs (camera, renderer, focus mode's scrub, the "Open it" slider). |
| `stave-gtk` | `ui/header.rs`, `tree.rs`, `inspector.rs`, `chrome.rs` | The title bar; the parts sidebar; the inspector; the status bar, motion bar, problems list and placeholder pages. |
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
`ShowSidebar(Sidebar)`, `Check`, `Build`.

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
- **Lane 2, the Files view**: the `files` page of the view stack
  (`chrome::FilesPage`, a placeholder) is replaced by the editor. It shows
  `Selection::Location` when `Event::ViewChanged` or
  `Event::SelectionChanged` arrives, and selects with `Command::Select` or
  `Command::GoToSource`; back and forward already walk through it.
- **Lane 3, problems**: the pill already toggles `Sidebar::Problems`; the
  sidebar's `problems` page (`chrome::ProblemsList`, a plain list) is
  replaced by the problems tree and filters (Problems.dc.html), selecting
  with `Selection::Problem`. A selected problem with a contact is posed
  and tinted already, and the motion bar swings it; the inspector's
  `problem` page (`Inspector::show_problem`, message and help only) is
  replaced by the problem's details and ways to fix it.
- **Cut list**: the `cut-list` page is a "Cut list is coming" status page.
