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
tools/window-shot apart.png examples/clothing_cabinet --explode 1
tools/window-shot problem.png tests/broken/door_hits_side --problems --diagnostic 0
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
system|light|dark`, `--view parts|cut-list|files`, `--select NAME` (a
part, piece of hardware or assembly), `--focus NAME` (a joint or a piece
of hardware) with `--plan` (from above), `--in-3d` and `--step AMOUNT`
(0 to 1), `--diagnostic N` (select the Nth problem), `--problems` (open
the problems list), `--scene NAME`, `--explode AMOUNT`, `--size WxH`,
and for the camera `--turn DEGREES` and `--pitch DEGREES` (as for
`--screenshot`), `--frame-selection` (framed on the selection, as F does)
and `--zoom FACTOR` (that many times closer once framed):

```
tools/window-shot close.png examples/clothing_cabinet --select 'door_r.stile[latch]' --zoom 3.5 --turn 15
```

Screenshot only: `--checking` holds a second check running (the pill's
spinner), `--building` presses the build button and holds its check.

The Files view has its own (with `--view files`): `--file FILE[:LINE]`
opens a file at a line (relative to the folder, or `std/joinery/dado.stave`),
`--cursor WORD` puts the cursor on the first `WORD` on that line, and once
the names are worked out `--peek` peeks at its definition, `--hover` shows
its card, `--uses` lists its uses, `--follow` follows it (F12), `--type
TEXT` types at the cursor a key at a time, `--undo` then undoes once,
`--search TEXT` finds TEXT from the cursor (Ctrl+F), and `--text-only`
hides the 3D view:

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
`docs/screenshots/files/` are made by `tools/gui-screenshots`, which says
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
| `stave-app` | `problems` | The problems list (`tree(checked, Filter)`: the parts tree showing only parts with problems, one `ProblemRow` per `Category` with its places; involved parts; `elsewhere`) and a problem's `details` (eyebrow, plain title, the `stave check` text, `Fix`es, the `MotionCard`). |
| `stave-app` | `focus`, `view` | Focus mode (its insert text with `steps`, `link` and `location`; `receives`), problems as rows, posing at a contact. |
| `stave-gtk` | `ui/mod.rs` | The window: owns the `Session`, dispatches commands, redraws on events, runs checks on a worker thread, and keeps what only the 3D view needs (camera, renderer, focus mode's scrub, the "Open it" slider). |
| `stave-gtk` | `ui/header.rs`, `tree.rs`, `inspector.rs`, `chrome.rs` | The title bar; the parts sidebar; the inspector (with a problem's details); the status bar, motion bar and placeholder pages. |
| `stave-gtk` | `ui/problems.rs` | The Problems sidebar: filters and the problems tree. |
| `stave-gtk` | `ui/view_tools.rs`, `slider.rs` | The 3D view's toolbar (Pull apart), camera buttons, the shared-wood key and focus mode's From above / In 3D switch; the design's slider (track, ring handle, a hatched blocked range). |
| `stave-gtk` | `ui/focus_stage.rs`, `focus_panel.rs` | Focus mode in the 3D view's area (the step player, the caption, three small views, the swing card) and a problem's motion card; focus mode's side panel. |
| `stave-gtk` | `ui/files.rs`, `editor.rs` | The Files view: the file sidebar, tabs, breadcrumbs, the uses panel and the 3D view's slot; the editor (a GtkSourceView 5 with its gutter, tags, lenses, peek, hover card and search bar; the only file that knows it is a GtkSourceView). |
| `stave-gtk` | `ui/icons.rs`, `style.rs` | The design's icons drawn from SVG path data in the text colour; one stylesheet whose colours are all libadwaita's named colours (`@headerbar_bg_color`, `@sidebar_bg_color`, `@card_bg_color`, `@accent_bg_color`, `@success_color`, ...), so light and dark both come from it and follow the style manager the moment it changes; only the code block is dark in both. |

## The session contract

`stave_app::session::Session` is one open project. It holds no GTK types
and is tested headless (`crates/stave-app/tests/session.rs`).

**State** (read with accessors):

- `view() -> View`: `Parts`, `CutList` or `Files`.
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
`Command::Build` does nothing unless `buildable()`; it checks first and,
when that check is done, gives
`Notice::BuildPlanNotBuilt` ("The build plan (PDF) isn't built yet"): the
PDF doesn't exist yet, and the button says so honestly.

## The window

```
┌ title bar ── sidebar · back · forward · open · title ─ Parts|Cut list|Files ─ scene ▾ · pill · hammer · menu ┐
│ ┌ sidebar ──────┐ ┌ 3D view ─────────────────────────────┐ ┌ inspector ─────┐ │
│ │ Parts tree    │ │ [Pull apart ▶ together—apart]  [fit]  │ │ details, focus │ │
│ │  or problems  │ │                               [front] │ │ panel, or a    │ │
│ │  tree         │ │                               [turn]  │ │ problem's      │ │
│ │               │ │  motion bar / motion card / steps     │ │ details        │ │
│ └───────────────┘ └───────────────────────────────────────┘ └────────────────┘ │
└ status bar ── ● Checked 5 files · 0 errors · 0 warnings · 26 ms   Watching for changes   Selected: … ┘
```

- **Title bar** (`Header`): back and forward are `win.back`/`win.forward`
  (Alt+←/→); the switcher is an `AdwToggleGroup` named by `View::word`;
  the scene menu lists the scene titles; the pill is `win.problems`
  (Ctrl+E, toggles `Sidebar::Problems`); the hammer is `win.build`
  (Ctrl+B), disabled while building or whenever there is nothing to
  build (`buildable()`: a library, no parts, a folder that can't be
  opened, errors), its tooltip saying why (`build_tooltip()`). The title
  is `Session::title()`. The
  main menu opens a project folder (Ctrl+O), checks again (Ctrl+R),
  frames everything, exports the view as glTF ("Export View as glTF…",
  `win.export-view`: asks where and writes the view as posed, pulled
  apart or opened, with `stave_app::export_view::save_glb`, the same file
  `stave export` writes) and sets the style (follow system, light, dark).
- **Parts sidebar** (`TreeSidebar`): the carcass, then assemblies with
  their motion ("turns 110°", "slides out 500 mm", "mirror of door_l").
  Rows that are the same part (one stock and size under one name, mirror
  pairs) share a row (`rail[bottom, top]`, `foot[4]`, `side_l, side_r`)
  until one of them is selected, when the group shows its members one by
  one. Each joint's members follow, one row for the joint grouped over
  its pairs with how many copies it places (`top_dowels's pin[2] ·
  dowel_pin × 8`; shown one by one, each pair's row counts its own,
  `dowel_pin × 4`), in the assembly of the part that carries them;
  selecting one selects that pair's pins (the inspector's "JOINT MEMBER":
  which joint placed it, its pair, what it is, how many, and the joint's
  line in "Defined in the text"). The assembly holding the selection
  opens. Ctrl+F searches.
- **Inspector**: eyebrow ("PART · IN door_l"), title, doc comment, rows
  (stock, finished size, grain, moves), joints and cuts (each row opens
  focus mode: `Command::Focus`), and "Defined in the text": the snippet of
  the declaration in the project's own files (a part of a library
  component is shown by its assembly's declaration) with a file:line link
  that sends `Command::GoToSource`. Every code block (this snippet, a
  problem's `stave check` text, a library's import line) is one style,
  `.stave-code`: #1e1e24 in both styles, and in the dark style #121216
  with a thin light edge so it stands apart from the dark panels (the
  window carries `stave-dark` then); the syntax colours are tested at
  4.5:1 or more on both.
- **3D view**: click to select (`Command::Select`), F frames, Escape
  leaves focus or clears the selection. The backdrop follows the style
  (#e8e5df light, #242428 dark, chosen when each frame is drawn and
  redrawn when the style changes). The floating motion bar opens the
  selection's door or drawer (a problem has its own motion card).
- **Narrow windows**: below 1100 sp the parts sidebar folds away (the
  sidebar button shows it over the view); below 860 sp the inspector
  does too (it slides in while something is selected), the switcher and
  scene menu drop their words and the subtitle goes; below 600 sp the
  title, the open button and forward go too.

## Nothing to draw

When there is nothing to draw the view's area shows `EmptyView` on the
stage colour, never a blank area: an icon, a title, what it means and,
where it helps, "Open a Project…" (`win.open`). The words come from
`stave_app::empty::of(&session)`: checking, a folder that can't be read,
a design whose errors stop it solving, a library of kinds with no parts
("Nothing to show: this folder is a library of kinds and components, with
no parts to place"), or a project without parts yet. (AdwStatusPage drew
nothing in this layout; the page is the window's own.)

The parts sidebar and the inspector say what is true for the same state
(`EmptyState::sidebar`, `EmptyState::inspector`), never "Nothing selected
/ Click a part…" when there is nothing to click. For a library folder,
`empty::library` reads what it defines from the resolved model: the
sidebar lists its public kinds (joints, hardware, fasteners, tools),
recipes, components, stock and profiles by group, each with the first
line of its doc comment (choosing one opens its text in the Files view,
`Command::GoToSource`), and the inspector says what a library folder is,
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
- **View toolbar** (top left, `ViewToolbar`): "Pull apart" is a toggle;
  on, it sets `Command::SetExplode(1)` and grows a play button and the
  "together … apart" slider (`Slider`, 12 px handle, thin ring); the
  slider sends `SetExplode`; play runs the amount from 0 to 1 in 3 s on
  the frame clock. `Event::ExplodeChanged` reposes (staged: assemblies
  first, then parts) and redraws the toolbar; the toggle stays on at 0
  until it is switched off. The status bar says "Pulled apart · 60%" while
  nothing is selected. Hidden in focus mode and while a problem is posed.
- **Pulled apart, the inspector stays as it is**: with nothing selected
  it shows "Nothing selected", and a selected part shows its usual
  account. The mockup's "How it comes apart" panel (Exploded.dc.html) was
  removed: its step list added little to the picture.
- **Camera buttons** (top right, `CameraToolbar`): fit is `win.frame`
  (the selection, or everything); front looks from the front, a little
  above, framing everything; turn swings the view half way round.

## Problems

- **The sidebar** (`ProblemsSidebar`, Problems.dc.html) opens with the
  pill, Ctrl+E or `--problems`; from another view `win.problems` goes to
  the Parts view first. Its filters (All / Errors / Warnings with counts)
  and rows come from `stave_app::problems::tree(latest, filter)`; the
  carcass and then the assemblies, each part with a problem showing a
  badge (how many problems are about it, red for errors, amber for
  warnings) or "involved", and under it one row per kind: the
  category's icon (line icons in `icons.rs`: the door-swing plan symbol
  for motion, two boxes sharing a corner for overlap, two apart for no
  contact, a drawer between sides for a slide gap, two arrows for order,
  ...), the title with its count when more than one, and the places in
  monospace; the full messages are its tooltip. Choosing a row selects
  its first problem (`Selection::Problem`); the row under the part it is
  about is ringed.
- **The details** (`Inspector::show_problem(&Details)`): the eyebrow in
  the severity's colour, the plain title and what happened, the text
  exactly as `stave check .` prints it in the dark code block (coloured
  by line: code, arrows, the source line highlighted, carets, `help:`),
  "Ways to fix it" (the help line split at "or"), each with "Show in
  text" (`Command::GoToSource` to the problem's line), and the note that
  fixes are never applied without you.
- **Posed at contact**: a problem with a contact is posed and tinted with
  its shared wood (unchanged), framed, with the "Shared wood at first
  contact" key in the view toolbar's place; a motion gets the motion card
  (`ProblemCard`): "Swing door to see where it hits", the slider from 0
  to fully open with the blocked range hatched red from the contact (its
  handle ringed red past it), "closed 0°", "blocked from 0.6°", "opens to
  110°". The status bar says "Posed at contact · door at 2.6°".

## Focus mode

Focus mode keeps its state in the window (`FocusState`: progress,
options, from above, playing) and lays the 3D view's area out by kind
(`layout_for_focus`); the side panel is the inspector's `focus` page.

- **A joint** (JointFocus.dc.html): the step player floats at the bottom
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
- **Hardware** (Hinge.dc.html): the area becomes a page (`.stave-stage.page`):
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
- Leaving (Esc, the panel's close button, Back) puts the Parts view's
  layout back.

## Slots for the next lanes

- **Lane 2, the Files view**: built; see "The Files view" below. "Show
  in text" in a problem's details and the joint kind's link in focus mode
  send `GoToSource` (the kind's link may name a `std/...` file, opened read
  only).
- **Cut list**: the `cut-list` page is a "Cut list is coming" status page.

## The Files view

FilesMode.dc.html: the text is the design, and the 3D view beside it
follows it. Pictures in `docs/screenshots/files/` (`linked.png`,
`peek.png`, `hover.png`, `uses_dark.png`, `problem.png`, `typed_dark.png`, `typed_light.png`,
`undo.png`, `search.png`, `brackets_dark.png`, `selected_in_3d_dark.png`, `narrow.png`).

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
  switches to Both).
- **Keys**: F12 or Ctrl+click follows a name; Alt+F12 peeks at its
  definition under the line (Open goes there; Escape closes); Shift+F12
  lists its uses in the panel under the editor (a row goes there);
  Alt+←/→ walk the window's history; Ctrl+F with the keyboard in the
  text opens the search bar over it (every match highlighted, "2 of 3",
  Enter / Shift+Enter or the arrows step, Escape closes; Ctrl+F anywhere
  else in the view still goes to "Go to file or name"); Ctrl+S saves; Ctrl+Shift+I lays the
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
  selected). A name the 3D view can't show leaves the selection alone; a
  name that means nothing (a typo, `Analysis::is_unresolved`) lets the
  selection go, so no stale chip stays.
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

**Differences from the mockup**: the 3D view's "Pull apart" toolbar is
lane 3's slot; the peek shows the doc comment, the declaration and the
four lines after it (the mockup skips to a later line); the editor's
monospace font is Adwaita Mono where JetBrains Mono isn't installed.
