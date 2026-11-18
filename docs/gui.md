# The desktop window

How `stave view` (the `stave-gtk` binary) is put together, the contract
its views share, and where the views still to come slot in. The design is
the canvas's "V2 · Spatial" page in `target/canvas/project/` (V2C_Home,
V2C_Spatial, V2C_EditorOpen with option A's rail of open files,
V2C_Problem, V2C_Finder, V2C_PullApart, V2C_Joint, V2C_Hinge, V2C_Dark);
libadwaita and the GNOME HIG settle what it leaves open. The first
version's three panes (Main, Problems, FilesMode) are kept on the
canvas's first page for reference.

Since 2026-11-18 the window is **docked** rather than floating over the 3D
view: the canvas "Stave docked layouts" (boards B and B2) and
`docs/superpowers/specs/2026-11-18-docked-window-design.md`. Where this
page still describes the V2 pages, the panels' contents are the same; only
where they sit has changed (see "The window").

## Seeing the real window

`tools/window-shot` writes a PNG of the real GTK window, with no screen:

```
cargo build -p stave-gtk
tools/window-shot main.png examples/clothing_cabinet --theme light --select 'door_l.stile[hinge]'
tools/window-shot dark.png examples/clothing_cabinet --theme dark
tools/window-shot busy.png tests/broken/door_hits_side --checking
tools/window-shot narrow.png examples/clothing_cabinet --size 760x900
tools/window-shot apart.png examples/clothing_cabinet --explode 1
tools/window-shot problem.png tests/broken/door_hits_side --diagnostic 0
tools/window-shot finder.png examples/clothing_cabinet --finder hinge
tools/window-shot joint.png examples/stool --focus rung_sockets --step 0.6
tools/window-shot hinge.png examples/clothing_cabinet --focus 'door_l.hinges[upper]' --plan
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
system|light|dark`, `--view design|cut-list` (`parts` and `files` name the
design too; `files` opens the editor), `--editor` (the editor open),
`--select NAME` (a part, piece of hardware or assembly), `--focus NAME` (a
joint or a piece of hardware) with `--plan` (from above), `--in-3d` and
`--step AMOUNT` (0 to 1), `--diagnostic N` (select the Nth problem),
`--finder TEXT` (the finder open with TEXT typed), `--xray` (the dock's
X-ray on), `--scene NAME`, `--explode AMOUNT`, `--size WxH`,
and for the camera `--turn DEGREES` and `--pitch DEGREES` (as for
`--screenshot`), `--frame-selection` (framed on the selection, as F does)
and `--zoom FACTOR` (that many times closer once framed):

```
tools/window-shot close.png examples/clothing_cabinet --select 'door_r.stile[latch]' --zoom 3.5 --turn 15
```

Screenshot only: `--checking` holds a second check running (the pill's
spinner), `--building` presses the build button and holds its check.

The editor has its own (`--file` opens the editor): `--file FILE[:LINE]`
opens a file at a line (relative to the folder, or `std/joinery/dado.stave`),
`--cursor WORD` puts the cursor on the first `WORD` on that line, and once
the names are worked out `--peek` peeks at its definition, `--hover` shows
its card, `--uses` lists its uses, `--follow` follows it (F12), `--type
TEXT` types at the cursor a key at a time, `--undo` then undoes once,
and `--search TEXT` finds TEXT from the cursor (Ctrl+F):

```
tools/window-shot peek.png examples/clothing_cabinet --view files --file doors.stave:9 --cursor FramePanelDoor --peek
tools/window-shot typed.png examples/clothing_cabinet --view files --file doors.stave:11 --cursor side_l --type q
tools/window-shot search.png examples/clothing_cabinet --view files --file doors.stave:9 --cursor door_l --search side
tools/window-shot linked.png examples/clothing_cabinet --view files --select 'door_l.stile[hinge]'
```

To check that "Follow system" follows the system, set the preference where
libadwaita reads it (`ADW_DEBUG_COLOR_SCHEME` stands in for the desktop's
settings portal, which otherwise answers with this desktop's own choice):

```
ADW_DEBUG_COLOR_SCHEME=prefer-light tools/window-shot light.png examples/clothing_cabinet --theme system
ADW_DEBUG_COLOR_SCHEME=prefer-dark  tools/window-shot dark.png  examples/clothing_cabinet --theme system
```

All the screenshots in `docs/screenshots/window/` and
`docs/screenshots/files/` (the editor's) are made by `tools/gui-screenshots`, which says
what each one shows; run it after a change to the window (or
`tools/gui-screenshots library` for the shots whose path contains
"library") and look at every picture that changed before committing it.

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
| `stave-app` | `problems` | The marks (`marks(checked)`: a `Mark` — error, warning or involved — per part or piece of hardware, per assembly and per file, for the dots), the problems grouped by part (`tree(checked, Filter)`), and a problem's `details` (eyebrow, plain title, the `stave check` text, `Fix`es, the `MotionCard`). |
| `stave-app` | `finder` | The finder (`search(Sources, query)`): parts and assemblies, files and names (through `stave-ide`), lines of the project's own text and problems, grouped by `Kind`, each `Hit` with its `Goal` (select it, or open a place in the editor). |
| `stave-app` | `focus`, `view` | Focus mode (its insert text with `steps`, `link` and `location`; `receives`), problems as rows, posing at a contact. |
| `stave-gtk` | `ui/mod.rs` | The window: owns the `Session`, dispatches commands, redraws on events, runs checks on a worker thread, hands the panels to the workbench, lays the problem pins over the 3D view and keeps what only the 3D view needs (camera, renderer, the looks, focus mode's scrub, the "Open it" slider). |
| `stave-gtk` | `ui/header.rs`, `tree.rs`, `inspector.rs`, `chrome.rs` | The title bar (with the problems chip and the finder's field); the Parts panel's tree; the Details panel's pages (a part, a problem's details, focus mode); the motion bar and the Cut list's placeholder. |
| `stave-gtk` | `ui/workbench.rs`, `panels.rs` | The docked window (libpanel): the dock, its panels (Parts, Files, Details, Code, Problems), the status bar with a toggle per edge, and the remembered `Layout` (`~/.config/stave/layout`; `--layout FILE` reads another and writes nothing; screenshots use the default); the Problems and Files lists. |
| `stave-gtk` | `ui/dock.rs`, `finder.rs` | The strip under the 3D view (scene menu, Pull apart, X-ray, the motion slider, the moving assemblies' chips, the camera buttons); the finder dialog (Ctrl+K). |
| `stave-gtk` | `ui/view_tools.rs`, `slider.rs` | The Pull apart toolbar (shown in the dock), camera buttons, the shared-wood key and focus mode's From above / In 3D switch; the design's slider (track, ring handle, a hatched blocked range). |
| `stave-gtk` | `ui/focus_stage.rs`, `focus_panel.rs` | Focus mode in the 3D view's area (the step player, the caption, three small views, the swing card) and a problem's motion card; focus mode's side panel. |
| `stave-gtk` | `ui/files.rs`, `editor.rs` | The Code panel: the open files as tabs (each with its dot), a placeholder while none is open, breadcrumbs, the uses panel and the footer; the editor (a GtkSourceView 5 with its gutter, tags, lenses, peek, hover card and search bar; the only file that knows it is a GtkSourceView). |
| `stave-gtk` | `ui/icons.rs`, `style.rs` | The design's icons drawn from SVG path data in the text colour; one stylesheet whose colours are all libadwaita's named colours (`@headerbar_bg_color`, `@sidebar_bg_color`, `@card_bg_color`, `@accent_bg_color`, `@success_color`, ...), so light and dark both come from it and follow the style manager the moment it changes; only the code block is dark in both. |

## The session contract

`stave_app::session::Session` is one open project. It holds no GTK types
and is tested headless (`crates/stave-app/tests/session.rs`).

**State** (read with accessors):

- `view() -> View`: `Design` or `CutList`.
- `editor() -> Editor`: `Closed` or `Open` (part of the place, so Back
  and Forward bring it back).
- `selection() -> Option<&Selection>`, where `Selection` is one of
  `Part(PartId)`, `Hardware(HardwareId)`, `Joint { joint, pair }`,
  `Member(PairMember)` (the member a joint lays out on one pair: a
  dowel's pins), `Assembly(AssemblyId)`, `Location(SourceLocation)` (a
  file path as the source map names it, and a `LineNumber` from 1) or
  `Problem(ProblemIndex)` (an index into `problems()`).
- `looks() -> Option<Vec<Look>>`: how the 3D view draws each node outside
  focus mode: when a joint's member is selected, or the selected problem
  is about one, the parts of its pair are see-through so the pins inside
  them show (`view::see_into`); otherwise `None`, every node as usual.
- `focus() -> Option<focus::Target>`: focus mode on a joint pair or a piece
  of hardware (always with the matching selection).
- `place() -> &Place`: view, selection, focus and editor together; one entry of
  the history. `can_go_back()`, `can_go_forward()` (forward only after
  going back).
- `scene()`, `scene_index()`, `scene_entry()`: the scene by name;
  `explode() -> Explode` (0 to 1, validated).
- `check_status() -> &CheckStatus`: `Idle` (never checked),
  `Checking { last: Option<Verdict> }` or `Done(Verdict)`, where a
  `Verdict` is `Checked(Summary)` (errors, warnings, files, elapsed) or
  `CantOpen(message)`. `pill()` gives the title bar's pill (`Tone`:
  `Unknown`, `Clean`, `Warnings`, `Errors`; `label`: "No problems",
  "3 problems"; `Activity`: `Idle` or `Checking`, keeping the last tone and
  label while checking); `line()` the problems chip's tooltip.
- `build() -> Build`: `Idle` or `Building`. `buildable() ->
  Result<(), empty::Reason>`: `Ok` only when the latest check has parts
  and no errors; otherwise why not (`Checking`, `CantOpen`, `Errors`,
  `Library`, `NoParts`, said in words by `Reason::nothing_to_build`:
  "Nothing to build: a library has no parts"). `build_tooltip()` is the
  hammer's tooltip: "Build plan (Ctrl+B)", "Building the plan…" or that
  reason.
- `title()`: the window's title, the folder's own declared title
  (`project "Clothing cabinet"`, `library "Japanese joinery samples"`),
  falling back to the folder's name only when none is declared or
  nothing has been read yet.
- `latest()`: the last check's result (its `problems()` are the current
  problems). `shown()`, `drawable()`: the last result that solved, which
  the tree, the inspector and the 3D view show; `showing_older()` when the
  latest didn't solve.

**Commands** (`apply(Command) -> Vec<Event>`): `ShowView(View)`,
`Select(Option<Selection>)`, `Focus(Target)`, `LeaveFocus`, `Back`,
`Forward`, `GoToSource(SourceLocation)` (the editor open, location
selected), `ShowEditor(Editor)` (open or close it, without a history
entry), `NextProblem` and `PreviousProblem` (F8, Shift+F8: the problem
after or before the selected one, wrapping round; from anything else the
first or the last), `ShowScene(name)` (with the scene's own explode
amount), `SetExplode`, `Check`, `Build`, and `Link(Option<Selection>)`:
the editor's cursor rests on a name, so the selection changes where the
window is without a history entry (the 3D view follows the text).

Every change of place (view, selection, focus, a source link) is pushed on
the back list (at most 100) and clears the forward list; selecting what is
already selected changes nothing.

**Events** (sorted, without repeats): `ViewChanged`, `SelectionChanged`,
`FocusChanged`, `HistoryChanged`, `SceneChanged`, `ExplodeChanged`,
`EditorChanged`, `CheckStatusChanged`, `BuildChanged`, `ResultChanged`,
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
`Command::Build` does nothing unless `buildable()`; it checks first and,
when that check is done, gives
`Notice::BuildPlanNotBuilt` ("The build plan (PDF) isn't built yet"): the
PDF doesn't exist yet, and the button says so honestly.

## The window

```
┌ title bar ── back · forward · open · title ── [Find a part, file, line or problem  Ctrl K] ── Design|Cut list · ‹ chip › · hammer · menu ┐
├ Parts │ Files ─────┬ 3D View ─────────────────────────────────────────────────┬ (right edge: empty, hidden) ┤
│ CARCASS            │                                                          │                             │
│  base     oak_19   │                        3D view                           │                             │
│ ASSEMBLIES         │                     (problem pins)                       │                             │
├ Details ───────────┼ View Controls ───────────────────────────────────────────┤                             │
│ PART · IN door_l   │ scene ▾ · Pull apart · X-ray · door_l ──── 110° · Move: … · [fit][front][turn]          │
│ stile[hinge]       ├ Code │ Problems ─────────────────────────────────────────┤                             │
│ Stock  oak_19 …    │ doors.stave │ carcass.stave                               │                             │
│                    │  9 assembly door_l : casework.FramePanelDoor {           │                             │
├────────────────────┴──────────────────────────────────────────────────────────┴─────────────────────────────┤
│ Linked to the text · door_l.stile[hinge], line 9                                       [left][bottom][right] │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Every part of the window is a panel, docked with libpanel (`Workbench`):
the 3D View, its View Controls, Parts, Files, Details, Code and Problems.
Nothing but the problem pins sits over the model. Each panel is a
`panel::Widget` in a `panel::Frame` (tabs along the top,
`panel::FrameTabBar`); drag a tab to an edge, into the centre (a
`panel::Grid`, which splits into columns and rows, so Code can sit beside
the 3D view) or into another frame; close it with its ×; show an edge
again from the status bar's toggles. The default layout is board B
(`Layout::default`): Parts and Files above Details on the left, the 3D
View over its View Controls in the centre, Code and Problems at the
bottom, the right edge empty and hidden. The layout is remembered per
user in `~/.config/stave/layout` (saved on every drop and when the
window closes; one line per panel, `code centre 1 0 0` for side, column,
frame and tab, and one per edge, `show end true`; a broken file means
the default) and never in a `.stave` file. "Reset Layout" in the main
menu puts board B back.

- **3D View** is the view and nothing else (the problem pins and the chip
  for a problem off screen sit on it).
- **View Controls** holds what used to float over or under the view: the
  scene menu, Pull apart, X-ray, the "Open it" slider, the Move chips, the
  camera buttons, focus mode's From above / In 3D switch, the shared-wood
  key, a short line ("Focused on one joint · Esc to go back"), a posed
  problem's swing card and a joint's step player. The buttons and sliders
  sit in an `adw::WrapBox`: in a wide, short frame they make a row and wrap
  onto more as it narrows; in a narrow side frame they stack; the panel
  scrolls when even that doesn't fit. Focus mode brings it forward.

- **Code panel** holds the open files as its own tabs, so a file always
  opens wherever the Code panel is docked (board B2: dragged to the right
  edge, files open on the right). With no file open it shows "No file
  open" and stays where it is. Ctrl+E ("Show the Code Panel") brings it
  back where it last was if it was closed, and puts the keyboard in the
  text.
- **Problems panel** lists the latest check's problems with their file and
  line; choosing one selects it (posed in the 3D view, explained in
  Details).
- **Files panel** lists the project's `.stave` files; choosing one opens
  it in the Code panel.
- **Details panel** is the old card: the selection's account, a
  problem's details, focus mode's controls (focus mode brings it to the
  front).
- **Status bar**: "Linked to the text · door_l, line 9" for the
  selection, and a toggle per edge.

- **Title bar** (`Header`): back and forward are `win.back`/`win.forward`
  (Alt+←/→). The finder's field is a button that opens the finder
  (`win.find`, Ctrl+K). The switcher is an `AdwToggleGroup` named by
  `View::word` (Ctrl+1, Ctrl+2). The problems chip is the status pill
  between previous and next arrows (`win.next-problem`, F8;
  `win.previous-problem`, Shift+F8; the arrows show when there is more
  than one problem); its tooltip is the status line ("Checked 5 files · 1
  error · 0 warnings · 26 ms"). The hammer is `win.build` (Ctrl+B),
  disabled while building or whenever there is nothing to build
  (`buildable()`), its tooltip saying why (`build_tooltip()`). The title
  is `Session::title()`. The main menu opens a project folder (Ctrl+O),
  checks again (Ctrl+R), frames everything, exports the view as glTF
  ("Export View as glTF…", `win.export-view`, with
  `stave_app::export_view::save_glb`, the same file `stave export`
  writes) and sets the style (follow system, light, dark).
- **Model tree** (`TreeSidebar`, top left): the carcass, then assemblies
  with their motion ("turns 110°", "slides out 500 mm", "mirror of
  door_l"). Rows that are the same part (one stock and size under one
  name, mirror pairs) share a row (`rail[bottom, top]`, `foot[4]`,
  `side_l, side_r`) until one of them is selected, when the group shows
  its members one by one. Each joint's members follow, one row for the
  joint grouped over its pairs with how many copies it places
  (`top_dowels's pin[2] · dowel_pin × 8`; shown one by one, each pair's
  row counts its own, `dowel_pin × 4`), in the assembly of the part that
  carries them; selecting one selects that pair's pins (the card's "JOINT
  MEMBER": which joint placed it, its pair, what it is, how many, and the
  joint's line in "Defined in the text"), drawn with the parts it sits in
  see-through. The assembly holding the selection opens. Every
  row carries its `Mark` (`problems::marks`): a red dot for an error, an
  amber one for a warning, a red ring for a part a problem only involves;
  an assembly carries the worst inside it. Under the chosen part, the
  joints it is in (`focus::targets_of`), each opening focus mode. F9
  shows the Parts panel.
- **Choosing in a list frames it**: a part, a piece of hardware or an
  assembly chosen in Parts, the finder or a lens's "show in 3D" is framed
  at once (no flying), with room round it (`FRAME_ROOM`). Clicking in the
  3D view selects without moving the camera, and only outlines the
  choice: the rest stays solid (fading is for X-ray and posed problems).
  A new check's result never moves the camera. The mouse's Back and
  Forward buttons go back and forward.
- **Details** (`Inspector`, in the Details panel):
  eyebrow ("PART · IN door_l"), title, doc comment, rows (stock, finished
  size, grain, moves), joints and cuts (each row opens focus mode:
  `Command::Focus`), and "Defined in the text": the snippet of the
  declaration in the project's own files (a part of a library component
  is shown by its assembly's declaration) with a file:line link that
  sends `Command::GoToSource`.
  Every code block (this snippet, a problem's `stave check` text, a
  library's import line) is one style, `.stave-code`: #1e1e24 in both
  styles, and in the dark style #121216 with a thin light edge so it
  stands apart from the dark panels (the window carries `stave-dark`
  then); the syntax colours are tested at 4.5:1 or more on both.
- **3D view**: click to select (`Command::Select`), F frames, Escape
  leaves focus or clears the selection. The backdrop follows the style
  (#e8e5df light, #242428 dark, chosen when each frame is drawn and
  redrawn when the style changes). With something chosen the rest is
  drawn `Look::Dim` (still wood, half faded); with X-ray on, `Look::XRay`.
- **Narrow windows**: below 1100 sp the subtitle goes and the editor
  narrows; below 860 sp the finder's field (Ctrl+K still opens it), the
  switcher's words and the open button go; below 600 sp the title and
  forward too.

## Nothing to draw

When there is nothing to draw the view's area shows `EmptyView` on the
stage colour, never a blank area: an icon, a title, what it means and,
where it helps, "Open a Project…" (`win.open`). The words come from
`stave_app::empty::of(&session)`: checking, a folder that can't be read,
a design whose errors stop it solving, a library of kinds with no parts
("Nothing to show: this folder is a library of kinds and components, with
no parts to place"), or a project without parts yet. (AdwStatusPage drew
nothing in this layout; the page is the window's own.)

The model tree and the card say what is true for the same state
(`EmptyState::sidebar`, `EmptyState::inspector`), never "Nothing selected
/ Click a part…" when there is nothing to click. For a library folder,
`empty::library` reads what it defines from the resolved model: the
tree lists its public kinds (joints, hardware, fasteners, tools),
recipes, components, stock and profiles by group, each with the first
line of its doc comment (choosing one opens its text in the editor,
`Command::GoToSource`), and the card says what a library folder is,
with the line that imports it (`import "../japanese_joinery" as
japanese_joinery`) and an example name through it.

Light and dark come from AdwStyleManager's colour scheme only: the old
GTK 3 `gtk-application-prefer-dark-theme` switch that some desktops write
into settings.ini is turned off before libadwaita starts, so runs are
free of its warning.

## The 3D view's controls

- **The selection** (`render::Renderer`): every node of the selected
  item (every item of a selected assembly) is drawn in one flat colour,
  its wood's base colour washed with a light accent, so neither the grain
  nor the darker cut faces streak it, and still lit, so its form reads.
  Round it goes an accent outline a constant few pixels wide at any zoom:
  the selected nodes are drawn into a mask (where they are seen, tested
  against the scene's depth, and where they cover at all), and a
  full-screen pass draws the accent where the mask changes. That traces
  the outside, holes seen through, parts in front and see-through parts
  alike, needs no closed or consistently wound meshes (mirrored parts),
  and draws over the part's own edges; where the selection is hidden
  behind other parts the outline is faint. Focus mode selects nothing in
  the view; a problem's parts keep their red tint and red shared wood.
- **The dock** (`Dock`, bottom middle): the scene menu, then the view
  toolbar, X-ray, the selection's motion bar ("door_l closed ——— 110°",
  the "Open it" slider), and while no slider shows a chip per moving
  assembly ("Move: door_l · drawer[top] …", ringed in its mark's colour)
  that selects it. Hidden in focus mode and while a problem is posed (its
  motion card takes the place).
- **View toolbar** (in the dock, `ViewToolbar`): "Pull apart" is a toggle;
  on, it sets `Command::SetExplode(1)` and grows a play button and the
  "together … apart" slider (`Slider`, 12 px handle, thin ring); the
  slider sends `SetExplode`; play runs the amount from 0 to 1 in 3 s on
  the frame clock. `Event::ExplodeChanged` reposes (staged: assemblies
  first, then parts) and redraws the toolbar; the toggle stays on at 0
  until it is switched off. A line at the top of the view says "Pulled
  apart · 60%" while nothing is selected.
- **Pulled apart, the card stays as it is**: a selected part shows its
  usual account. The mockups' "How it comes apart" panel (Exploded.dc.html,
  V2C_PullApart) was removed: its step list added little to the picture.
- **Camera buttons** (bottom right, `CameraToolbar`): fit is `win.frame`
  (the selection, or everything); front looks from the front, a little
  above, framing everything; turn swings the view half way round.

## Problems

- **Pins** (V2C_Home): each problem with a contact is pinned on the model
  where it happens (the middle of its first shared-wood box, or of the
  parts it is about, through `Orbit::project`): a red "!" (amber for a
  warning) with its words, at most `PIN_LIMIT`. A problem whose place is
  off screen is named on a chip at the view's bottom edge ("Off screen:
  error · …"), which selects it. A posed problem shows only its own pin;
  focus mode and an older design show none.
- **The dots**: the tree's rows and the rail's tabs carry their marks
  (`problems::marks`); the dock's chips are ringed for a moving assembly
  with a problem.
- **Stepping**: the chip's pill and arrows, F8 and Shift+F8 select the
  next or previous problem (`Command::NextProblem`), wrapping round.
- **The details** (`Inspector::show_problem(&Details)`, in the card at the
  top right): the eyebrow in the severity's colour, the plain title and
  what happened, the text exactly as `stave check .` prints it in the dark
  code block (coloured by line: code, arrows, the source line
  highlighted, carets, `help:`), "Ways to fix it" (the help line split at
  "or"), each with "Show in text" (`Command::GoToSource` to the problem's
  line), and the note that fixes are never applied without you. Nothing
  here edits the text for you: there are no quick fixes yet.
- **Posed at contact** (V2C_Problem): a problem with a contact is posed
  and tinted with its shared wood (unchanged), framed, with what it
  doesn't involve drawn `Look::Dim` and the "Shared wood at first
  contact" key at the top of the view; a motion gets the motion card
  (`ProblemCard`): "Swing door to see where it hits", the slider from 0
  to fully open with the blocked range hatched red from the contact (its
  handle ringed red past it), "closed 0°", "blocked from 0.6°", "opens to
  110°". The line at the top of the view says "Posed at contact · door
  at 2.6°".

## Focus mode

Focus mode keeps its state in the window (`FocusState`: progress,
options, from above, playing) and lays the 3D view's area out by kind
(`layout_for_focus`); its panel is the card's `focus` page, at the top
right (V2C_Joint, V2C_Hinge), and the dock steps aside.

- **A joint** (JointFocus.dc.html, V2C_Joint): the step player floats at the bottom
  of the view (`StepCard`): play (pauses while playing), "**Step 1 of 1** ·
  `push(rung[front], along: length)`", "rung[front] is 6 mm from home",
  and the apart … together scrub. The side panel: JOINT, the kind's name
  and the first sentence of its doc comment; Goes in / Receives it / the
  cut / Fit (green when it fits); SHOW with X-ray ("see into
  leg[front_left]"), Tint cut faces and Fade other parts; HOW IT GOES
  TOGETHER with the link to the kind (`std:joinery/socket`, to its line)
  and its `insert:` steps one a line.
- The side panel shows the kind's doc comment's first paragraph (two
  sentences at most, `Focus::summary`) with the rest under "More"; member
  rows are two lines at most (the whole caption is their tooltip), so the
  rows below them show without scrolling at 1440×900; the panel scrolls
  when the window is shorter.
- **Hardware** (Hinge.dc.html, V2C_Hinge): the area becomes a page
  (`.stave-stage.page`, keeping clear of the card and the rail; the tree
  steps aside):
  a line saying what the view looks at ("**Looking down** on the hinge,
  through the cabinet top · back ↑ · front ↓", or "In 3D around ..."), the
  view in a rounded frame with the From above / In 3D switch at its top
  right (`PlanSwitch`; only when focus has a view from above), the
  members' numbers drawn on it, three small views rendered offscreen at
  fixed angles (closed, 45°, fully open; the nearest is ringed, a click
  goes there), and the swing card (`SwingCard`: play, the door's name,
  the slider and "44°"). The side panel: HARDWARE, "How a cup hinge
  works" and the kind's doc comment, its documented members numbered as
  on the drawing, and Opens to / Closest to the side / Closest to
  anything / Swing check.
- Leaving (Esc, the panel's close button, Back) puts the design's layout
  back.

## Slots for the next lanes

- **The editor**: built; see "The editor" below. "Show in text" in a
  problem's details and the joint kind's link in focus mode send
  `GoToSource` (the kind's link may name a `std/...` file, opened read
  only).
- **Cut list**: the `cut-list` page is a "Cut list is coming" status page.

## The editor

V2C_EditorOpen.dc.html with option A: the text is the design, and the 3D
view beside it follows it. Pictures in `docs/screenshots/files/`
(`linked.png`, `peek.png`, `hover.png`, `uses_dark.png`, `problem.png`,
`typed_dark.png`, `typed_light.png`, `undo.png`, `search.png`,
`brackets_dark.png`, `selected_in_3d_dark.png`, `narrow.png`).

```
                         ┌ 3D view (narrower) ────────┐ ┌ doors.stave ───────────────────── › ┐
                         │                            │‹│ doors.stave › assembly door_l       │
                         │                            │ │  8                                  │
                         │    the cursor's part        │d│    2 uses · show in 3D              │
                         │    framed                   │o│  9 assembly door_l : …              │
                         │                            │o│ 10   opening: door_bay[left]        │
                         │                            │r│ [uses panel]                        │
                         │ [dock]                      │+│ Ctrl+click follows a name …  Line 9 │
                         └────────────────────────────┘ └─────────────────────────────────────┘
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

### The panel

- **The tabs** across the top of the Code panel: a tab per open file, a
  dot for the file's worst problem (`Marks::file`) and "•" while unsaved;
  the tab of the file showing is the accent. Files open from the Files
  panel, the finder (Ctrl+K) or by following a link.
- **Breadcrumbs** follow the cursor. **Footer**: the shortcuts and the
  line.
- **Editor**: a GtkSourceView 5 (`editor.rs` is the only file that knows;
  the page sees a `gtk::TextView` and `gtk::TextBuffer`s). Colours from
  the semantic tokens (one source of truth: the grammar's words through
  `stave-lang` and `stave-schema`), applied as buffer tags; line numbers
  with the cursor's in the accent colour, the cursor's line tinted with
  the design's blue bar at its left edge (a gutter renderer of ours,
  which also draws the problem bars), matching brackets boxed, Enter
  indenting two spaces more after an opening `{`, `(` or `[` and a
  closing one typed first on a line going back out (as `stave fmt` lays
  it out; a test checks the two agree on every example), Tab inserting
  spaces, smart Home/End, undo that groups typing, the
  name under the cursor and its uses marked (its declaration stronger),
  wavy underlines from the fast tier and the last check (drawn by the
  text view itself, 1.6 px thick, in libadwaita's error and warning
  colours for the style, with a bar of the same colour in the gutter
  beside each line that has one), a lens above each part, assembly, piece of hardware,
  component and kind ("2 uses" lists them; "show in 3D" selects it and
  frames it).
- **Keys**: F12 or Ctrl+click follows a name; Alt+F12 peeks at its
  definition under the line (Open goes there; Escape closes); Shift+F12
  lists its uses in the panel under the editor (a row goes there);
  Alt+←/→ walk the window's history; Ctrl+F with the keyboard in the
  text opens the search bar over it (every match highlighted, "2 of 3",
  Enter / Shift+Enter or the arrows step, Escape closes; Ctrl+F anywhere
  else opens the finder); Ctrl+S saves; Ctrl+Shift+I lays the
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
  selected), and frames it. A name the 3D view can't show leaves the
  selection alone; a name that means nothing (a typo,
  `Analysis::is_unresolved`) lets the selection go.
  Choosing in the 3D view, the model tree or the finder moves the editor
  to `link::placed_at` (a part of a library component: its assembly's
  line) without moving the selection back. A new check's result never
  moves the editor.
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

**Highlighting stays single-sourced.** The buffer's own syntax
highlighting is off and there is no `.lang` file: a GtkSourceView
language definition would be a second list of the grammar's words that
could drift from `stave-lang` and `stave-schema`, and it couldn't tell a
part's name from a recipe's, which the semantic tokens do. The style
scheme colours everything else: `stave-light` and `stave-dark`, derived
from Adwaita and Adwaita-dark (`parent-scheme`) with the design's text
colour, cursor-line tint and accent line number and a box round matched
brackets, written to `~/.cache/stave/styles/` at start (GtkSourceView
loads schemes only from folders; if that can't be written, plain Adwaita
and Adwaita-dark). They follow libadwaita's light or dark style. The
lenses, the peek and the hover card are overlay children of the text
view (made once and hidden when not used: GTK can't take an overlay back
out of a text view), placed in room that a tag's `pixels-above-lines`
makes for them; the line numbers sit at the bottom of their line's room,
level with the text.

**Differences from the mockups**: the peek shows the doc comment, the
declaration and the four lines after it (the mockup skips to a later
line); the editor's monospace font is Adwaita Mono where JetBrains Mono
isn't installed; choosing frames at once rather than flying; a problem's
card has "Show in text" for each way to fix it and no "Apply" or
"Preview" (fixes that edit the text aren't built); the model tree shows
what isn't involved in a posed problem as it is, dimming it in the view
rather than hiding it; the finder opens as a dialog in the middle of the
window; the panels are docked (the canvas's "Stave docked layouts") rather
than floating over the view.
