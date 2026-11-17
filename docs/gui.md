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
| `stave-app` | `problems` | The problems list (`tree(checked, Filter)`: the parts tree showing only parts with problems, one `ProblemRow` per `Category` with its places; involved parts; `elsewhere`) and a problem's `details` (eyebrow, plain title, the `stave check` text, `Fix`es, the `MotionCard`). |
| `stave-app` | `focus`, `view` | Focus mode (its insert text with `steps`, `link` and `location`; `receives`), problems as rows, posing at a contact. |
| `stave-gtk` | `ui/mod.rs` | The window: owns the `Session`, dispatches commands, redraws on events, runs checks on a worker thread, and keeps what only the 3D view needs (camera, renderer, focus mode's scrub, the "Open it" slider). |
| `stave-gtk` | `ui/header.rs`, `tree.rs`, `inspector.rs`, `chrome.rs` | The title bar; the parts sidebar; the inspector (with a problem's details); the status bar, motion bar and placeholder pages. |
| `stave-gtk` | `ui/problems.rs` | The Problems sidebar: filters and the problems tree. |
| `stave-gtk` | `ui/view_tools.rs`, `slider.rs` | The 3D view's toolbar (Pull apart), camera buttons, the shared-wood key and focus mode's From above / In 3D switch; the design's slider (track, ring handle, a hatched blocked range). |
| `stave-gtk` | `ui/focus_stage.rs`, `focus_panel.rs` | Focus mode in the 3D view's area (the step player, the caption, three small views, the swing card) and a problem's motion card; focus mode's side panel. |
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

Light and dark come from AdwStyleManager's colour scheme only: the old
GTK 3 `gtk-application-prefer-dark-theme` switch that some desktops write
into settings.ini is turned off before libadwaita starts, so runs are
free of its warning.

## The 3D view's controls

- **View toolbar** (top left, `ViewToolbar`): "Pull apart" is a toggle;
  on, it sets `Command::SetExplode(1)` and grows a play button and the
  "together … apart" slider (`Slider`, 12 px handle, thin ring); the
  slider sends `SetExplode`; play runs the amount from 0 to 1 in 3 s on
  the frame clock. `Event::ExplodeChanged` reposes (staged: assemblies
  first, then parts) and redraws the toolbar; the toggle stays on at 0
  until it is switched off. The status bar says "Pulled apart · 60%" while
  nothing is selected. Hidden in focus mode and while a problem is posed.
- **How it comes apart** (Exploded.dc.html): while Pull apart is on and
  nothing is selected, the inspector lists the moves of
  `stave_app::apart::steps`, read off the scene's poses at 0, ½ and 1 so
  the words say what the picture does: stage 1 the assemblies moving out
  whole ("Drawers slide forward on their slides until they clear the
  carcass", "Doors come forward off their hinges. Cups go with the door,
  plates stay on the side"), stage 2 the parts coming out of their
  joints, per kind of assembly and then the carcass ("Carcass comes
  apart: back slides out of its rabbet, divider and shelf slide out of
  their dados, foot[4] drop out of their round tenon joints"). "Save this
  view as glTF" asks where and writes the view as posed
  (`apart::save_glb`, the same file `stave export` writes).
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

- **Lane 2, the Files view**: the `files` page of the view stack
  (`chrome::FilesPage`, a placeholder) is replaced by the editor. It shows
  `Selection::Location` when `Event::ViewChanged` or
  `Event::SelectionChanged` arrives, and selects with `Command::Select` or
  `Command::GoToSource`; back and forward already walk through it. "Show
  in text" in a problem's details and the joint kind's link in focus mode
  send `GoToSource` (the kind's link may name a `std/...` file).
- **Cut list**: the `cut-list` page is a "Cut list is coming" status page.
