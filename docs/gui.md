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
`--finder TEXT` (the finder open with TEXT typed), `--xray` (the view's
X-ray on), `--scene NAME`, `--explode AMOUNT`, `--size WxH`,
and for the camera `--turn DEGREES` and `--pitch DEGREES` (as for
`--screenshot`), `--frame-selection` (framed on the selection, as F does)
and `--zoom FACTOR` (that many times closer once framed):

```
tools/window-shot close.png examples/clothing_cabinet --select 'door_r.stile[latch]' --zoom 3.5 --turn 15
```

Screenshot only: `--checking` holds a second check running (the pill's
spinner), `--building` presses the build button and holds its check.

Editing has two (see [Editing in the 3D view](#editing-in-the-3d-view)):
`--edit-script FILE` runs editing steps once the design is drawn, as the
pointer and the card would, before the picture is taken, and
`--new-project` opens the New Project dialog. A step a line, `#` starts a
comment, a step that can't be done is said on standard error with its
line and the rest are skipped:

| Step | What it stands for |
|---|---|
| `hover <region>` | the pointer resting in a space: it is lit |
| `select region <name>` | stands for a click in a space: the space is found by name and the edit started from it, its menu checked, without going through the scene's pick |
| `select face <part>.<face>` | stands for the "Add something onto its … face" button in Details for the part: the face is found by name, the button is not pressed |
| `select between <part> <part>` | stands for a click in the gap between two facing parts: the pair is found by name, without the scene's pick |
| `click face <part>.<face>` | a click on a part's face: the part selected, Details offering that face |
| `choose <entry>` | a menu entry, by its label or `group.label` |
| `bind region\|face\|between ...` | stands for the next click, filling the place the card asks for: found by name, without the scene's pick |
| `point face <part>.<face>` | the next click made on the model at the middle of that face and picked as the pointer's is (so a wall of a space is reached through the space); it fails if the click would pick something else |
| `set <name> = <value>` | a setting typed on the card (or chosen from its short list or its search) |
| `default <name>` | the default chosen from the setting's short list (`EditCommand::UseDefault`) |
| `choices <name>` | the setting's short list opened on the card (its ▾ pressed): the default, the values the project already uses, the value it holds now, "Search all …"; its options checked first, the card scrolled to it |
| `search <name> [<query>]` | "Search…" chosen from the setting's short list, the query typed: the search over every option, each checked first |
| `options <name>` | the same search with nothing typed: every option of the setting, each with its fit |
| `put back` | the card's button that puts back the setting a refusal blames |
| `name <name>` | the name retyped on the card |
| `fix <problem> <fix>` | a fix's button, both counted from 1 |
| `add`, `cancel` | the card's buttons (a change's card too) |
| `pick <name>` | a click on a part, piece of hardware or assembly: selected, Details showing its changes |
| `repeat open` | "Repeat…" pressed: the count and the direction open (on the chosen candidate's card, else on the selected item's) |
| `repeat <count> [up\|across\|deep]` | the count stepper: on the chosen candidate's card, else on the selected item's; "Repeat…" is pressed first when it is folded |
| `drag count <count>` | the count set on the repeat stepper ("Repeat…" pressed first when it is folded) |
| `drag boundary <n> <length>` | the n-th boundary (from 1) of the split the selection fills moved until the piece before it is that long, as its row in the Sizes card would |
| `mirror` | "And the other side" switched on |
| `hold <a> to <b>` | "Hold a distance…" from `a`, then a click on `b` (each a part or a space by name) |
| `hold min\|max [<length>] "<reason>"` | the hold card's fields (without a length, the distance as it is now) |
| `hold try` | the hold card's "Try it" |
| `remove` | Delete on the selection |
| `include dependants` | the removal's dialog: remove them too |
| `rename <name>` | the selected item's name retyped among its changes in Details |
| `finder <text>` | Ctrl+K with the text typed (the finder stays open) |
| `do <text>` | Ctrl+K, the text, Enter: the first verb the finder lists is run |
| `turn <degrees> [<pitch>]` | the view orbited between two clicks: turned about the upright by that many degrees, and tipped to that height above the horizontal (negative looks from below), so a face out of sight can be pointed at |
| `save` | Ctrl+S: every open file with unsaved changes saved (an Add or a change made while its file is open in the editor stays unsaved there, as typing would, until saved) |

```
tools/window-shot refused.png crates/stave-gtk/fixtures/door_hits_wall --edit-script tools/window-edits/door_refused.edit
```

Add writes, so point `--edit-script` that adds at a copy of the project
(`tools/gui-screenshots` copies its projects to a temporary folder).

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
| `stave-app` | `part_joints` | The Parts panel's joint chips per part and per folded assembly, the selected part's joined-to rows, the headings' notes, which assemblies fold, and what a narrow panel shows (`Density`). |
| `stave-app` | `inspect` | The parts tree (`parts_tree`, `PartsTree::filtered`) and the inspector's account of a selection (`inspect`): its swatch, its `Field`s (label, value, and the `NameLink`s a value names), its `Connection`s by `ConnectionSection` (Joined to: joints, the hardware on it, what a hinge hangs it on, each partner a `NameLink`; Cuts; the way into focus mode), the `FailingCheck`s about it, and the declaration it is written in, highlighted (`snippet`, `highlight`); `place_links` reads an edit's place values (`bay[doors]`, `[side_l, side_r]`) into links. A `NameLink`'s `LinkTarget` selects (`Select(Selection)`) or lights a space (`Space(RegionId)`). |
| `stave-app` | `problems` | The marks (`marks(checked)`: a `Mark` — error, warning or involved — per part or piece of hardware, per assembly and per file, for the dots), the problems grouped by part (`tree(checked, Filter)`), every problem in full as data (`reports(checked)`, `report(checked, index)`: a `Report` with severity, code, message, its `ReportPlace`s — `Role::Primary` or `Secondary`, `file:line:column`, the place to open, the line split before / underlined / after, the label's words — its notes and help, from the same diagnostic and the same `stave_lang::place_label` the `stave check` renderer uses, never by parsing its text), and a problem's `details` (eyebrow, plain title, the `stave check` text, `Fix`es, the `MotionCard`). |
| `stave-app` | `finder` | The finder (`search(Sources, query)`): parts and assemblies, files and names (through `stave-ide`), lines of the project's own text and problems, grouped by `Kind`, each `Hit` with its `Goal` (select it, or open a place in the editor). |
| `stave-app` | `focus`, `view` | Focus mode (its insert text with `steps`, `link` and `location`; `receives`), problems as rows, posing at a contact. |
| `stave-app` | `spaces` | Which space an assembly fills, read from the model (`filled_spaces`: the region its component's first `Region` param is bound to), and the boundaries of a split with where a dragged one settles (`split_boundaries`, `SplitBoundary::settle`), for the window's How many and a script's boundary moves. |
| `stave-gtk` | `ui/mod.rs`, `ui/update/` | The window: a relm4 component (`Window`) whose model owns the `Session` and every leaf's `Controller`; it dispatches commands, redraws on events, runs checks on a worker thread, hands the panels to the workbench, lays the problem pins over the 3D view and keeps what only the 3D view needs (camera, renderer, the looks, focus mode's scrub, the "Open it" slider). `mod.rs` holds the model, `init` (`build_widgets` makes the widgets and the narrow-window breakpoints), the main menu (`main_menu`) and how a window is asked to open (`Options`). Its behaviour is split by group under `update/`, each with its own input messages, its handlers and a `wire` function that turns its widgets' signals, its actions and its leaves' outputs into those messages: `selection.rs` (what is selected and focused, focus mode, posed problems, the motion sliders, the finder's hits), `project.rs` (the folder, checks, the editor's files, saving, closing, the command line's wishes and screenshots), `editing.rs` (editing in the 3D view, handled in `edit_view/`) and `rendering.rs` (the camera, drawing, the pins, pulling apart, X-ray and exporting). |
| `stave-gtk` | `ui/frame_lists.rs` | What each frame of the 3D view draws selected and how each node looks, kept between frames and worked out again only when the selection, the hover, the result, X-ray, focus mode or the posed problem changes, so an orbit or a slider drag builds no lists. |
| `stave-gtk` | `ui/relm4_shim.rs` | `Bridge`, which owns a relm4 component's `Controller` behind a leaf's older callback-style API while the window moves to relm4 (docs/relm4-migration.md). |
| `stave-gtk` | `ui/header.rs`, `tree.rs`, `inspector.rs`, `chrome.rs` | The title bar (with the problems chip and the finder's field); the Parts panel (joint chips); the Details panel's pages (the Inspector for a selection, focus mode); the motion bar and the Cut list's placeholder. |
| `stave-gtk` | `ui/sections.rs` | The Inspector's and the Edit panel's building blocks, after Unity's inspector: the header row, collapsible sections, label \| value rows (22 px, a 104 px grey label column), name links (`LinkHandlers`: a click selects, the pointer lights it in 3D; wired by `edit_view/links.rs`), the Cancel (Esc) / Apply (Enter) footer, and their stylesheet. |
| `stave-gtk` | `ui/workbench.rs`, `panels.rs` | The docked window (libpanel): the dock, its panels (Parts, Files, Details, Edit, Code, Problems), the status bar with a toggle per edge, and the remembered `Layout` (`~/.config/stave/layout`; `--layout FILE` reads another and writes nothing; screenshots use the default); the Problems and Files lists. |
| `stave-gtk` | `ui/dock.rs`, `finder.rs` | The scene menu's floating box and the moving assemblies' chips on the 3D view; the finder dialog (Ctrl+K). |
| `stave-gtk` | `ui/view_tools.rs`, `view_layout.rs`, `slider.rs` | The camera column (fit, front, turn, X-ray, Pull apart and Pull apart's pop-out slider), the shared-wood key and focus mode's From above / In 3D switch; where each floating control goes on the 3D view so none covers another (`view_layout::place`, tested without GTK); the design's slider (track, ring handle, a hatched blocked range; at least 24 px tall; mirrored right to left in drawing, pointer and arrow keys; moved a twentieth a notch by the scroll wheel; its value in words for screen readers, "2.6°, blocked" past the blocked point, and the blocked range, "blocked from 0.6°", as its description; a stronger track and a dark edge on the handle in high contrast). |
| `stave-gtk` | `ui/focus_stage.rs`, `focus_panel.rs` | Focus mode in the 3D view's area (the step player, the caption, three small views, the swing card) and a problem's motion card; focus mode's side panel. |
| `stave-gtk` | `ui/files.rs`, `editor.rs` | The Code panel: the open files as tabs (each with its dot), a placeholder while none is open, breadcrumbs, the uses panel and the footer; the editor (a GtkSourceView 5 with its gutter, tags, lenses, peek, hover card and search bar; the only file that knows it is a GtkSourceView). |
| `stave-gtk` | `ui/edit_panel.rs`, `option_search.rs` | The Edit panel's content (the card in progress, or how to start one) and the search over a setting's options (below). |
| `stave-gtk` | `ui/edit_menu.rs`, `edit_card.rs`, `edit_demo.rs` | The editing menu for a selection and the card for a chosen candidate (below), drawn from `stave_app::edit`'s contract values; `edit_demo` shows them from the contract's fixture (`--edit-fixture`). |
| `stave-gtk` | `ui/edit_view.rs`, `edit_writer.rs`, `edit_script.rs` | Editing on the real session: what the pointer lights and a click starts, the menu and the card drawn from `Session::edit` after each `EditEvent` and what is done on them sent as `EditCommand`s, the ghost; Add's writer through the editor's buffers; `--edit-script`. |
| `stave-gtk` | `ui/edit_view/` | Beside `edit_view.rs`: `changes.rs` (a change's card and the removal's dialog), `item_tools.rs` (a selected item's changes, in the Details panel), `repeat_handles.rs` (the count stepper on a candidate's card; a boundary moved by a script), `hold.rs` (Hold this), `verbs.rs` (the finder's verbs), `editor_link.rs` (the card's Pending section and the old lines marked in the editor; editing off while the text doesn't read), `change_steps.rs` (their edit-script steps), `pointing.rs` (what a click binds), `links.rs` (what a link in the Inspector or the Edit panel selects and lights), `grips.rs`, `sizes.rs`, `size_slider.rs`, `grip_panel.rs`, `grip_text.rs` (the sizes of the selection: their listing and changes, the Edit panel's Sizes card and its size editor, the editor's slider, the change card's size section for a param made solved or plain, their words). |
| `stave-gtk` | `ui/new_project.rs` | The New Project dialog, writing with `stave_app::change::new_project_file`. |
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
  "1 error", "2 warnings", "1 error, 2 warnings"; `Activity`: `Idle` or `Checking`, keeping the last tone and
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

## Starting Stave

Stave is one application, `io.github.OceanEilonwy.Stave` (`ui/app.rs`),
with a window per project folder. `stave-gtk FOLDER...` opens each folder
in a window (a `.stave` file opens its folder); started again while it
runs, or asked by Files to open a folder, it hands the folders to the
Stave already running (`HANDLES_OPEN`), which opens a window for each or
brings forward the one already showing it. The command line's other
window options apply to the first window of a Stave that was not
running. `--window-screenshot` always runs on its own (`NON_UNIQUE`) and
needs one folder.

Started with no folder (from the app grid or the dock), Stave shows the
**start page** (`ui/start_page.rs`): an `AdwStatusPage` with Open
Folder… and New Project… and, under them, the recent projects
(`gtk::RecentManager`, the folders Stave opened that are still there,
newest first). Opening a project opens its window and closes the start
page. A folder (or a `.stave` file) dropped from Files onto the start
page or a project window opens it; in a project window it replaces the
project there, after asking about unsaved changes, as Open Project… does.

Ctrl+W closes the window (`window.close`, asking about unsaved changes)
and Ctrl+Q closes every window (`app.quit`), each asking. The shortcuts
dialog (`adw::ShortcutsDialog`) lists every key: the actions' keys come
from the one table `app::ACCELERATORS` that also binds them, and the few
keys widgets handle themselves (Delete in the 3D view, Undo, Redo and
Escape in the editor) from `app::OTHER_KEYS`. Each action's `KeyScope`
says what binds its keys: the application (`set_accels_for_action`), the
window after the focused widget and never while typing in a text field
(F frames, Escape leaves focus or clears the selection), or the text
editor (`editor.follow` F12, `editor.peek` Alt+F12, `editor.uses`
Shift+F12, `editor.show-info` Ctrl+I, also in the text's right-click
menu), or the 3D view while it has the keyboard (`view.turn-left` and
`view.turn-right` on Left and Right, `view.look-higher` and
`view.look-lower` on Up and Down, about 15° a press; `view.zoom-in` on
plus, equals or keypad plus and `view.zoom-out` on minus or keypad
minus, a scroll step each).

To a screen reader the 3D view is a picture named "3D view of <project
title>", its description what is selected and what the last check found
("door_l selected. 1 error."), kept up to date as either changes. It is
outlined in the accent colour while it has the keyboard
(`staveviewport:focus-visible`).

Stave remembers the window's size, whether it was maximised and the
style chosen in the menu, in `~/.config/stave/window` (a GLib key file,
`ui/remember.rs`; a key file rather than GSettings so that `cargo run`
needs no installed schema), and the panel layout in
`~/.config/stave/layout`. `--size` and `--theme` override them for one
run; screenshots neither read nor write them. A failure to write either
file is said once, in a toast. The first window is 1200x800.

The desktop file, the AppStream metainfo and the icons are in `data/`;
`tools/install-desktop` installs them, with the binary, under
`~/.local` (it does not build).

## The window

```
┌ Project Edit View Help  Clothing cabinet ──────────── [Design|Cut list] ──────────── 🔍 · ✓ 0 problems · [hammer] ┐
├ Parts ─────────────┬ ▣ Model │ doors.stave ● ─────────────────────────────────┬ Details │ Edit ──────────────────┤
│ CARCASS            │ [As built ▾]       (a short line)           [fit]        │ PART · IN door_l              │
│  base     oak_19   │                                             [front]      │ stile[hinge]                  │
│ ASSEMBLIES         │                        3D view              [turn]       │                               │
│  door_l  turns 110°│                     (problem pins)          [X-ray]      │ Stock  oak_19 …               │
├ Files ─────────────┤                          [▶ together ──o── apart][Pull]  │                               │
│ cabinet.stave      │                                                          │                               │
│ doors.stave        │ [Move: door_l drawer[top] …]    [door_l closed ──o── 110°]│                               │
├────────────────────┴──────────────────────────────────────────────────────────┴───────────────────────────────┤
│ Problems 0 │ Output │ Linked to the text · door_l.stile[hinge], line 9            [left][bottom][right] Panels ▴ │
└───────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Every part of the window is a panel, docked with libpanel (`Workbench`):
the 3D view (its tab reads "Model"), Parts, Files, Details, Edit, Code,
Problems and Output.
Nothing but the problem pins, the editing menu at a click and the small
labels that belong on the model (the ghost's verdict, the repeat
nothing that can be dragged) sits over the model; the sizes a selection
can change, the boundaries between pieces included, are in the Edit
panel. Each panel is a
`panel::Widget` in a `panel::Frame` (tabs along the top,
`panel::FrameTabBar`); drag a tab to an edge, into the centre (a
`panel::Grid`, which splits into columns and rows, so Code can sit beside
the 3D view) or into another frame, or pick "Move to the Right" (left,
bottom, top, middle) from its tab's menu; close it with its × (every
panel but the 3D view, whose tab has no ×); show an edge again from the
status bar's toggles. A first launch shows only the 3D view, Parts on the
left and Problems along the bottom (`Layout::default`), so a beginner
meets their project rather than a dock. Every other panel opens from the
status bar's Panels menu (Code also from "Edit the Text" and Ctrl+E; Edit
when an edit starts), into its home on the compact board (H2,
`Layout::board`): Parts in a frame above Files on the left; the Model in
the centre with Code as the next tab of the same frame (so the open files
sit beside the Model); Details with Edit as tabs on the right; Problems
and Output at the bottom. Choosing a part reveals its line in the Code
panel only while that panel is open. The edges start 272 px (left), 340 px
(right) and 220 px (bottom) wide; a window narrower than 1000 sp narrows
the sides to 220 and 260. A layout file older than the Edit panel puts it
beside Details wherever that file put Details, and a panel a file doesn't
mention (Output, in a board B file) goes where the compact board puts it, so a
saved board B still opens as board B. The layout is remembered per user
in `~/.config/stave/layout` (saved on every drop and when the window
closes; a broken file means the default) and never in a `.stave` file:

```
view centre 0 0 0        # panel, side, column, frame, tab
code centre 0 0 1
closed output            # a closed panel
show bottom false        # an edge shown or hidden
tool camera right        # where a tool group on the 3D view is docked
```

"Reset Layout" (View menu, and the status bar's Panels menu) puts the
first launch's three panels back, the tool groups in their usual spots.

- **3D View** is the view with its controls floating on it (part of the
  panel, so they go wherever the view is docked): the scene menu at the
  top left; a column of icon buttons at the top right (fit, front view,
  turn to the back, then X-ray and Pull apart, which stay pressed while
  on), Pull apart's play button and "together … apart" slider popping out
  to the left of its button while it is on; a short line ("Focused on one
  joint · Esc to go back") and the shared-wood key along the top; the
  moving assemblies' chips at the bottom left and the selection's "Open
  it" slider at the bottom right; a joint's step player, a posed
  problem's swing card and the chip for a problem off screen above the
  foot, in the middle; focus mode's From above / In 3D switch at the top
  right; and the problem pins. There is no strip under the view.
  `view_layout::place` puts each control where it goes for the view's
  size and what is showing (through the overlay's `get-child-position`),
  so none covers another: the chips go above the slider when they can't
  sit beside it, a bottom control keeps clear of the camera column, the
  pop-out goes under the column when there's no room beside it, and the
  line drops below the scene menu when it can't sit beside it. A control
  that is hidden takes no room and can't catch the pointer meant for the
  view. The four tool groups (scene menu, camera column, chips, motion
  slider) can be docked elsewhere by pressing and holding one, then
  dragging it; see [The 3D view's controls](#the-3d-views-controls).
- **Headers** are a slim strip of flush tabs, 26 px tall (`TabStrip`, our
  own `panel::FrameHeader`): a tab as wide as its title, the shown one
  with an accent line and its ×, and the shown panel's menu (▾: "Move to
  the Right", ...) at the end. Dragging a tab docks its panel elsewhere.
  The 3D view's tab reads "Model" with a cube. A panel can show its own
  tabs in place of its one (`tab_strip::set_page_tabs`): the Code panel
  shows one tab per open file (its problem dot, its name in monospace and
  a yellow ● while it has unsaved changes), so on the compact board the
  strip reads "Model · doors.stave ● · carcass.stave". Clicking a file's
  tab shows the Code panel on that file; clicking Model shows the 3D view.
  With no file open the Code panel keeps its own "Code" tab.
  For the keyboard and screen readers the strip is a tab list of tabs
  (the shown one selected); showing another panel restyles the tabs in
  place, so the focused tab keeps the keyboard (a panel's own tabs
  changing remakes them, and the tab at the same place takes the keyboard
  back); each tab is one Tab stop (the × is for the pointer, labelled
  "Close <panel>"; the ▾ menu closes a panel); Ctrl+Page Up / Ctrl+Page
  Down show the previous or next panel while the keyboard is in the frame
  (after the focused widget, so the editor keeps its own).
- **Panels** (the status bar's button, and the main menu) shows any closed
  panel again, back where it was, and resets the layout.

- **Code panel** holds the open files (their tabs are in the strip of the
  frame holding it, beside the Model by default), so a file always
  opens wherever the Code panel is docked. Opening a file (from Files, the
  finder, a link or Ctrl+click) shows that file's tab. Choosing a part,
  assembly or piece of hardware anywhere opens the file that places it,
  at that line, as a tab beside the Model without covering it (the panel
  comes forward only when it doesn't share a frame with the 3D view; the
  keyboard stays where it is) (board B2: dragged to the right
  edge, files open on the right). The text scrolls that line into view
  only once the text has its size (`Editor::go_to` waits for it on the
  frame clock): a file opened into a panel that was showing "No file
  open", or behind another tab, has none until the next frame, and a
  scroll worked out before then lands anywhere. With no file open it
  shows "No file open" and stays where it is. Ctrl+E ("Show the Code Panel") brings it
  back where it last was if it was closed, and puts the keyboard in the
  text.
- **Problems panel** is where a problem is read: every problem of the
  latest check in full, as `stave check` prints it
  (`problems::reports`): `error[require]` in the severity's colour, the
  message, then each place — its `file:line:column` as a link that opens
  it in the Code panel (`Command::GoToSource`), with the label's words
  ("the kind's rule") — over its line of source in a small dark code
  block, coloured like the editor, without its indent, the part it is
  about tinted and underlined (wavy for where it is, straight for a place
  that explains it), then each `note:` and the `help:` line (green).
  Choosing a row selects the problem (posed in the 3D view). A new
  check keeps the rows of problems that are unchanged at the start and
  end of the list and replaces only the run between, so the list does
  not jump while someone types. While the 3D
  view shows an older design (`showing_older()`), a line over the list
  says so: "The 3D view shows the last version without errors until
  these are fixed." There is no banner over the window.
- **Files panel** lists the project's `.stave` files; choosing one opens
  it in the Code panel.
- **Details panel** is the old card: the selection's account and focus
  mode's controls (focus mode brings it to the front). A selected
  problem isn't explained here; the Problems panel shows it.
- **Edit panel** holds the edit in progress: the card for a chosen menu
  entry, the card for a change of the design, or the hold card (see
  [Editing in the 3D view](#editing-in-the-3d-view)). It is a panel like
  the others: a tab (beside Details by default), dragged to any area,
  resized by the splitters, closed and brought back, its place remembered
  with the rest (`edit start 0 1 1`). When a card appears its tab comes to
  the front (the panel opens where it last was if it was closed), unless
  it shares a frame with the 3D View, which it would hide; the card then
  takes the keyboard. While nothing is in progress it stays where it is
  and says how to start one ("Nothing being added"); when an edit is added
  or cancelled and the panel shares a frame with Details, Details comes
  back to the front, showing what was just added.
- **Output panel** lists what each check said, one line each ("Checked 5
  files · 0 errors · 0 warnings · 26 ms"), the newest at the foot.
- **Status bar**: the bottom panels folded into it. On its left,
  "Problems N" (the count green at none, amber otherwise) and "Output"
  are tab-like buttons: each opens the bottom edge on its panel
  (`Workbench::present`) and, while that panel shows there, folds the
  edge away again; the one showing carries an accent line. Then "Linked
  to the text · door_l, line 9" for the selection; on the right a toggle
  per edge and the Panels menu.

- **Title bar** (`Header`, 34 px): a menu bar (`gtk::PopoverMenuBar`)
  with Project (New Project…, Open Project…, Save, Check Again, Build
  Plan, Export 3D Model…), Edit (Find…, Find in the Text, Format the
  Text, Next / Previous Problem), View (Design, Cut List, Back, Forward,
  Frame Everything, the panels, Reset Layout, the style) and Help (About
  Stave), all from `main_menu`; then the project's name in small grey
  words (`Session::title()`, its folder in the tooltip). The Design | Cut
  list switcher sits in the middle. On the right: a find button (a
  magnifier; "Find a part, file, line or problem (Ctrl+K)", `win.find`),
  the problems chip and the hammer. Back and forward are not in the bar:
  they are `win.back`/`win.forward` (Alt+←/→, the View menu, the mouse's
  side buttons), each offered only when it can go. The switcher is an
  `AdwToggleGroup` named by `View::word` (Ctrl+1, Ctrl+2). Until the Cut
  list is made (`header::ready`), the switcher is hidden and only
  `win.view-design` (Ctrl+1) exists. The problems chip is the status pill
  between previous and next arrows (`win.next-problem`, F8;
  `win.previous-problem`, Shift+F8; the arrows show when there is more
  than one problem); its tooltip is the status line ("Checked 5 files · 1
  error · 0 warnings · 26 ms"). The pill is red while the latest check
  has an error, amber for warnings only, green for none (`Pill::tone`;
  the class goes on the chip round the pill, `.stave-problems-chip.errors
  button.stave-pill`, a test keeps the CSS keyed so). Colour is never
  the only sign: the pill's words count errors and warnings apart and
  its icon is a circle for errors, a triangle for warnings only. Icon-only
  buttons have accessible names, not only tooltips. The hammer is `win.build` (Ctrl+B),
  disabled while building or whenever there is nothing to build
  (`buildable()`), its tooltip saying why (`build_tooltip()`). Until the
  build plan is made, the hammer is hidden and `win.build` isn't
  registered: a button that only says it does nothing teaches a beginner
  not to trust the window. The Project menu starts a new project ("New
  Project…", `win.new-project`, see [A new project](#a-new-project)),
  opens a project folder (Ctrl+O) or a recent one ("Open Recent",
  `win.open-recent`), saves (Ctrl+S), checks again (Ctrl+R) and exports
  the view as glTF ("Export 3D Model…", `win.export-view`, with
  `stave_app::export_view::save_glb`, the same file `stave export`
  writes); the View menu frames everything, brings back closed panels and
  sets the style (follow system, light, dark: `app.style`, remembered);
  Help has "Keyboard Shortcuts" (`app.shortcuts`, Ctrl+?) and "About
  Stave" (`app.about`).
- **Parts panel, "joint chips"** (`TreeSidebar`, top left; rows from
  `inspect::parts_tree`, joints from `part_joints::PartJoints`): a flat
  list grouped by assembly instead of a deep tree. Each group has a slim
  uppercase heading (CARCASS, DOOR_L) with a grey note on the right: how
  it moves after a turn or slide icon ("110° on side_l": the parts or
  spaces outside the assembly its moving hardware names) or else what it
  holds ("12 parts · 6 joints"). Rows that are the same part (one stock
  and size under one name, mirror pairs) share a row (`rail[bottom,
  top]`, `foot[4]`, `side_l, side_r`) until one of them is selected, when
  the group shows its members one by one. Each part row (24 px) has its
  swatch, its name, small **joint-kind chips** (an icon per kind; the
  count shows when there is more than one; the tooltip names the kind and
  the count: "dowel: 8 pins", "dado: 2 joints", "euro hinge × 2") and its
  problem dot. A count is of what the part has: the pieces a joint lays
  out (side_l's two dowel joints of four pins: `dowel 8`), else the pairs
  it is in (`dado 2`), else hardware fitted to it; a shared row adds its
  parts up. The icon comes from what the joint does to the wood (pins,
  a tenon, a tenon in a groove, a channel, a slot, a hole, dovetails, a
  shape, a plain butt), never from the kind's name, and from the base for
  hardware (hinge, slide, pull), so a project's own kinds get one too.
  A mirror image or one of a set (`door_r`, "mirror of door_l";
  `drawer[bottom, top]`) folds into one row with its chips (a set's
  chips count each pair once); activating it opens it, and an assembly's
  heading (with its small chevron) folds it again. Hardware and a
  joint's pieces (pins) have no rows of their own. The **selected part
  expands** into "joined to" rows: an icon, the kind in words ("cope and
  stick", "groove, no glue", "euro hinge ×2, back face", "turns on") and
  the partner names as links (monospace light blue on a faint blue tint)
  that select the partner when clicked and outline it in the 3D view
  while hovered (`TreeSidebar::connect_hover`, drawn with the selection's
  outline). A part that hardware elsewhere names lists it too (side_l:
  "euro hinge ×2 → door_l.hinges[lower, upper]"). The partners' own rows
  get a 2 px light-blue left edge and a faint blue tint. Activating a
  joined-to row opens focus mode on that joint (its first pair) or piece
  of hardware; choosing a piece of hardware or pins elsewhere expands the
  part it is on and marks its row. Narrow panels show less
  (`part_joints::Density`, from a width watcher): below 220 px the counts
  leave the chips (they stay in the tooltips), the heading notes drop
  (the motion icon stays), folded rows drop their words and joined-to rows
  keep their icon and partners; below 160 px each row's chips fold into
  one badge (a joint glyph and how many kinds, the kinds in its tooltip)
  and names ellipsize. Every row carries its `Mark`
  (`problems::marks`): a red dot for an error, an amber one for a
  warning, a red ring for a part a problem only involves; a folded row
  carries the worst inside it. F9 shows the Parts panel.
  For the keyboard and screen readers the list is a tree (its role
  `Tree`, each row a `TreeItem` with its level: headings and folded rows
  1, part rows 2, joined-to rows 3; a heading expanded, a folded row not;
  a row's dim text and its mark in words as its description). The arrow
  keys only move through the rows: a click or Enter chooses one (selects
  and frames it, opens focus mode, opens a library's text), so walking
  the list moves neither the camera nor the editor. Right (Left, right to
  left) or + opens a folded row and, on an open heading, goes to its
  first row; Left or − folds a heading and, on a part row, goes to its
  heading (on a joined-to row, to its part); the keyboard stays on the
  assembly's row. Choosing a part inside an assembly the user folded,
  elsewhere (the 3D view, the finder), opens it again. A new check, new
  marks or a new selection update the rows in place: rows that read the
  same are kept and only the run that differs is replaced, so the scroll
  position and the row with the keyboard survive a check, and a check
  that changed nothing touches no row. The panel alone at
  300, 200 and 150 px:
  `STAVE_PARTS_SHOTS=dir cargo test -p stave-gtk parts_panel_pictures --
  --ignored` on a display.
- **Choosing in a list frames it**: a part, a piece of hardware or an
  assembly chosen in Parts, the finder or a lens's "show in 3D" is framed
  at once (no flying), with room round it (`FRAME_ROOM`), and the 3D View
  comes to the front (brought back if closed). Clicking in the
  3D view selects without moving the camera, and only outlines the
  choice: the rest stays solid (fading is for X-ray and posed problems).
  A new check's result never moves the camera. The mouse's Back and
  Forward buttons go back and forward.
- **Details** (`Inspector`, in the Details panel), drawn after Unity's
  inspector (`ui/sections.rs`):

  ```text
  [▪] [stile[hinge]               ]  doors.stave:9 ●
  ▾ Part                                   in door_l
    Stock          oak_19 · white oak, 19 mm
    Moves          with door_l, turns 110°
  ▾ Joined to                                     4
    cope_and_stick (rail[bottom]) (rail[top])
    groove         (panel)
    euro_hinge ×2  (hinges[lower]) (hinges[upper])  back face
    hinged on      (side_l)
  ▾ Cuts                                          1
    2 cup bores    35 mm across, 12.8 mm deep
    Checks                               0 problems
  ▾ Defined in                        doors.stave:9
    assembly door_l : casework.FramePanelDoor { …
  ```

  A header row: the selection's swatch, its name in a monospace field,
  and the file:line it is written at as a link (`Command::GoToSource`),
  with a yellow ● after it while that file has unsaved changes. Then
  collapsible sections (a chevron and a bold 12 px title on a faint band,
  a grey note on the right): what it is (the eyebrow's kind as the title,
  "in door_l" as the note; the doc comment; stock, finished size, grain,
  moves), **Joined to** (its joints, the hardware mounted on it by kind
  and face, and for hinges the part they hang it on), **Cuts** (the cuts
  hardware makes in it), **Checks** (each check that fails about it, a
  click selects the problem; otherwise only the count, "0 problems" —
  never a vague "all hold"), **Defined in** (the snippet of the
  declaration in the project's own files; a part of a library component
  is shown by its assembly's declaration). Rows are label | value, about
  22 px tall, the label in a 104 px grey column, the value in a dark
  rounded field. A joint's or a cut's name opens focus mode on it
  (`Command::Focus`). **Every value that names another part, piece of
  hardware, assembly or space is a link** (monospace, #99c1f1 on a faint
  blue tint, rounded; from `stave_app::inspect::NameLink`): a click
  selects it (a space is lit instead: it can't be selected); while the
  pointer is on it, it is outlined in the 3D view as a selection is (a
  space lit as the pointer in it lights it). The Edit panel's cards use
  the same rows and links.
  Every code block (this snippet, a problem's `stave check` text, a
  library's import line) is one style, `.stave-code`: #1e1e24 in both
  styles, and in the dark style #121216 with a thin light edge so it
  stands apart from the dark panels (the window carries `stave-dark`
  then); the syntax colours are tested at 4.5:1 or more on both.
- **3D view**: click a part to select it (`Command::Select`); a click
  in a space or in the gap between two facing parts opens the editing
  menu for it instead (see [Editing in the 3D view](#editing-in-the-3d-view)).
  F frames, Escape drops an edit in progress, else leaves focus or
  clears the selection; Delete removes the selected item, naming first
  what would go with it (see [Changing what is there](#changing-what-is-there)). The backdrop follows the style
  (#e8e5df light, #242428 dark, chosen when each frame is drawn and
  redrawn when the style changes). With something chosen the rest is
  drawn `Look::Dim` (still wood, half faded); with X-ray on, `Look::XRay`.
- **Narrow windows**: below 1000 sp the side edges narrow (220 px left,
  260 px right); below 860 sp the project's name goes; below 760 sp the
  switcher's words; below 600 sp the menu bar folds into one menu button
  (the same four menus) and the switcher compacts.

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
- **The floating controls** (`Dock`): the scene menu in its own box at
  the top left; the selection's motion bar ("door_l closed ——— 110°", the
  "Open it" slider) at the bottom right; and while no slider shows, a
  chip per moving assembly ("Move: door_l · drawer[top] …", ringed in its
  mark's colour) at the bottom left, which selects it. The scene menu,
  the chips, X-ray and Pull apart step aside in focus mode and while a
  problem is posed (its motion card takes the place).
- **X-ray and Pull apart** (`ViewToolbar`, at the foot of the camera
  column): X-ray is an icon toggle; so is "Pull apart": on, it sets
  `Command::SetExplode(1)` and pops out a play button and the "together …
  apart" slider (`Slider`, 12 px handle, thin ring) to the left of its
  button, joined to the column; the slider sends `SetExplode`; play runs
  the amount from 0 to 1 in 3 s on the frame clock. `Event::ExplodeChanged` reposes (staged: assemblies
  first, then parts) and redraws the toolbar; the toggle stays on at 0
  until it is switched off. A line at the top of the view says "Pulled
  apart · 60%" while nothing is selected.
- **Pulled apart, the card stays as it is**: a selected part shows its
  usual account. The mockups' "How it comes apart" panel (Exploded.dc.html,
  V2C_PullApart) was removed: its step list added little to the picture.
- **Camera buttons** (top right, `CameraToolbar`, one column with X-ray
  and Pull apart under a line): fit is `win.frame` (the selection, or
  everything); front looks from the front, a little above, framing
  everything; turn swings the view half way round.
- **Docking the tool groups** (`floating::Floats`, `view_layout::Spots`):
  the scene menu, the camera column, the chips and the motion slider are
  small rounded groups floating on the view. Press and hold one (a long
  press, `gtk::GestureLongPress`), then drag it (`gtk::GestureDrag`): it
  follows the pointer, and the spots it may land in show as dashed
  outlines in the accent colour, the nearest one brighter. Let go and it
  docks there. There are four spots, one group each: the top left, bottom
  left and bottom right corners, where a group lies flat (a row), and the
  right edge, where it stands upright (a column; the motion slider's
  scale turns upright too). A spot is free while the group in it isn't
  showing (the chips step aside while the motion slider shows, so the
  chips' corner is free then); dropping a group there swaps the two, so
  two groups never land on top of each other (`Spots::dock`,
  `Spots::free_for`, `view_layout::nearest`, all unit-tested). Pull
  apart's slider stays beside the camera group: out to the left of its
  foot on the right edge, under it in the top corner, above it in a
  bottom corner. There are no handle icons; the press and hold is the way
  in, and each group's tooltip says "Press and hold to move it". Where
  each group sits is kept with the panel layout (`tool camera
  bottom-left`), never in a `.stave` file; Reset Layout puts them back.

## Problems

- **Pins** (V2C_Home): each problem with a contact is pinned on the model
  where it happens (the middle of its first shared-wood box, or of the
  parts it is about, through `Orbit::project`): a red "!" (amber for a
  warning) with its words, at most `PIN_LIMIT`. A problem whose place is
  off screen is named on a chip at the view's bottom edge ("Off screen:
  error · …"), which selects it. A posed problem shows only its own pin;
  focus mode and an older design show none.
- **The dots**: the tree's rows and the rail's tabs carry their marks
  (`problems::marks`), round for an error, square for a warning; the
  Move chips are ringed for a moving assembly with a problem and carry
  the same dot, and say the problem to a screen reader.
- **Stepping**: the chip's pill and arrows, F8 and Shift+F8 select the
  next or previous problem (`Command::NextProblem`), wrapping round.
- **Reading a problem**: in the Problems panel (see "Problems panel"
  above), in full. Nothing there edits the text for you: a problem of
  the design has no quick fix yet (an edit's card applies its refusal's
  fixes, see [Editing in the 3D view](#editing-in-the-3d-view)). Details
  doesn't repeat it.
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
right (V2C_Joint, V2C_Hinge), and the scene menu, the Move chips, X-ray and Pull apart step aside.

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

## Editing in the 3D view

[Editing](editing.md) runs on the session: `ui/edit_view.rs` turns what
the pointer and the widgets do into `Command::Edit(EditCommand)` and
draws `Session::edit` again after each `Event::Edit(EditEvent)`. It
decides nothing; what can go where, the verdict and the text written are
the session's.

- **Lit under the pointer.** Moving the pointer over empty space inside a
  space (a region: an opening, a bay, the envelope) lights it: an accent
  box on top (`Frame::lit`), and the parts between the eye and it drawn
  see-through (`SceneGraph::covering`, `Look::XRay`), so the pointer
  works behind a door or a case's front. The scene decides what is under
  the pointer (`pick_target`): a part in front of a space wins; a part
  that is the space's wall, or beyond it, doesn't.
- **A click starts an edit.** In a space: the menu headed "In this
  space" opens beside the click (`EditCommand::Start(Picked::Region)`),
  the space stays lit while the edit is open, and any selection is let
  go. In a gap between two facing parts that is in no space
  (`SceneGraph::facing`): the menu "Between these parts". On a part: it
  is selected as before, and the Details panel gains, under its
  account, "Add something onto its right face…", which opens the menu "On this face" for the face that
  was clicked. A click on nothing, or on a part, drops the edit in
  progress. Focus mode edits nothing.
- **The menu** lists the session's candidates as their applicability
  arrives: the check runs a step at a time on the main loop
  (`Session::check_edit_menu` with a flag a timer raises, the step twice
  as long each time one finds nothing new), and the menu is drawn after
  each step, entries not checked yet with a spinner ("Checking whether
  it fits here…"), entries that fit, entries that need a choice ("Then
  point at or choose: hinge side, mount", still choosable), and greyed
  ones with the checker's reason. Choosing one sends `Choose(index)`,
  the index looked up by the entry's declaration when the command is
  sent, not when it was clicked: a step of the check can sort the menu in
  between (greyed entries go last in their group).
- **The card** takes the menu's place in the **Edit panel** (see [The
  window](#the-window)); the menu stays a pop-up at the click. Its name,
  its settings, its fix buttons, the put-back button, Add and Cancel all
  send the session's commands (`Rename`, `Set`, `UseDefault`, `ApplyFix`,
  `PutBack`, `Add`, `Cancel`); what is typed is read with
  `editing::parse_value` and `parse_name`, and what can't be read is said
  in a toast. It is compact: the title with where it comes from on one
  line ("Case · casework · in this space"), the hint on one line (in full
  in its tooltip), then one row each for the name and the places it fills,
  the verdict, a small "SETTINGS" caption and **one row per setting**: its
  name on the left (cut short with "…" when long, in full in its
  tooltip), its value on the right in an entry with a ▾ beside it. There
  is no line under a setting: its type and where its value comes from
  ("Stock · your choice", "Length · the default, 20mm", with a `within`
  range) are the row's tooltip and the entry's accessible description. A
  setting at its default shows the value the default stands for
  (`default_value`, else the default as written: `dowel(count: 4)`) dimmed
  in the empty entry; one with no value yet says "Choose…" and its name is
  in the warning colour. A value longer than the entry beside the name can
  show (`overlay(left: 18mm, right: 18mm, …)`) is also said in full,
  wrapped, under its row. The body scrolls; Add and Cancel stay at the
  foot of the panel, always in sight, "Repeat…" on their line. A long
  verdict's words (a refusal's problems, warnings) scroll in a box of
  their own, at most 120 pixels tall, so they never push the buttons out
  of reach; the blame, the verdict's line and the fixes' buttons stay
  outside it.
  A place param still unbound shows the contract's
  prompt (`PlaceParam::prompt`, "Point at the side the hinges are screwed
  to."), and the next click on the model binds it (`Bind`), picked for
  what the param holds (`edit_view/pointing.rs`): a space wins over its
  walls only when a `Region` is wanted; for an `Anchor` or a `Part` the
  click is on the part the ray meets, even through a space (the hinge
  side is a wall of the bay: its inside face is clicked through the
  bay); for `Parts` alone, the two parts facing across the gap. While a
  part is wanted the pointer lights no space. The fixes added are listed ("FIXES ADDED · Use left: 17mm").
  A command that tries the candidate again first shows Checking (the
  spinner, the ghost grey) for a moment so it is drawn, then runs. What
  a widget reports is always sent after its own signal has finished.
- **A setting's short list.** The ▾ beside a setting opens, in the card
  under its row (a popover would close each time a check's answer redraws
  it, and can't be drawn into a screenshot), a short list from
  `stave_app::choices::short_list`: the default (marked "default"), the
  values the project's own text already gives settings like it, the most
  recent first ("used here"), then the value it holds now if neither
  lists it ("now"); a tick marks the value held now, and a refused one is
  greyed with the "can't" sign and its reason in its tooltip, and never
  reports. The default sends `UseDefault` (the setting goes back to its
  default, so nothing is written for it); another sends `Set`. Last comes
  "Search all 18…".
- **The search.** "Search…" opens a dialog in the middle of the window,
  over a dimmed backdrop (part of the window, as the removal's dialog is,
  since the window's screenshot doesn't draw an `AdwDialog` opened by an
  edit script): a field and every option of the setting that matches
  what is typed (`stave_app::choices::search`: by name, a prefix of it, a
  part of it, or the first sentence of its doc comment), grouped as the
  menu groups its entries (the project's own, imported modules, then
  "Standard · joinery"), the best matches first. Each shows its name, its
  description and its fit as the card words it (a spinner and "Checking
  whether it fits…", a tick and "Fits", "Checked once you choose …", or
  greyed with a "can't" sign, "Can't be used here" and the reason
  `OptionRefusal::line` gives: the checker's first error, or for a kind
  that can never be used for this component its plain sentence, "Shelf
  gives its joint a depth, and glued_edge has no depth"). ↑ and ↓ move, Enter chooses (a refused option can't be
  chosen: the foot says so instead), Escape or × closes, and the keyboard
  goes back to the card.
- **The options, checked.** Once a candidate is chosen, and after each
  change to it, its settings' options are checked on a worker thread
  (`Session::options_job`, the setting being searched or whose short list
  is open first; cancelled and started again when the candidate changes),
  each answer handed to `Session::option_fit_found` as it comes and drawn
  in place (`EditCard::update_options`, and the open search), so an open
  list stays open and the keyboard stays where it was. Each option's
  hint (its declaration's first sentence) is its accessible description.
  A value can still be typed in the entry.
- **Blame and the way back.** When a refusal is blamed on a setting
  (`Chosen::blame`), the verdict box starts with it, in bold ("It stopped
  fitting when joinery became chigiri."), and the button "Put joinery
  back to dowel(count: 4), the default" (`PutBack`), above "It doesn't
  fit: 2 problems" and the checker's own words.
- **The ghost.** Once the chosen candidate has been tried and its
  scratch copy solved, the 3D view draws the candidate's scene
  (`stave_scene::Ghosts` over the base design, at rest): what it adds
  or changes see-through and tinted by the verdict (grey checking, green
  fits, red refused; `Frame::tints`, `GhostVerdict::tint`), with a
  label beside it ("Checking…", "✓ It fits", "✕ It doesn't fit") so the
  verdict never rests on colour alone, and what covers the space it goes
  in see-through. Problem pins step aside while an edit is open.
- **Add** writes through the editor's buffers when the file has a tab,
  or the editor is open (which opens the file's tab and shows it): the
  text shows the change unsaved and one undo takes the whole change
  back (`edit_writer::BufferWriter`, one user action). Otherwise it
  writes to disk, and nothing in the window takes that back: there is
  no undo of a commit yet (Remove, or the text, are the ways back). The
  session then shows the committed design and the
  window selects the new item by its name
  (`link::selection_named`), without moving the camera (as a click
  on the model selects). **Cancel** and **Escape** send
  `Cancel`: nothing was written.
- Rejections (`EditEvent::Rejected`) are said in a toast, in the
  session's words.

Grips and the changes of the design below are in the window too.

### Sizes

[Grips](editing.md#grips), the sizes a selection can change, are rows
of the Edit panel's **Sizes** card (`ui/edit_view/sizes.rs`), drawn
from what the session lists (`EditCommand::ListGrips`,
`Session::grips`) and changed with the session's drag commands
(`BeginDrag`, `DragTo`, `PickParam`, `MakeSolved`, `MakePlain`, `Add`,
`Cancel`) by `ui/edit_view/grips.rs`. **Nothing on the model can be
dragged**: no handles, no dimension lines, no labels, no pills. The
window decides nothing: what a size moves, what stops it, the verdict
and the text written are the session's; what a slider position or a
step is worth, where a value snaps and how far it goes are
`stave_app::grip_values` (`slider_range`, `along`, `nudged`,
`drag_target`, `typed_check`).

```text
Edit panel:
  ▾ Sizes                                   divider     what they are of
    height                         500mm  ›             a row: click to change it
    ┌ height                       was 500mm  ⋮ ┐
    │ [ 540mm      ]  +40mm                      │      the size editor, in the row's place
    │ ////|━━━━━━━━━━━|━━━━━━━●────────────|//// │      slider: stops, the old value's tick
    │ 180mm                              900mm   │
    │ [−10] [−1] [+1] [+10] [back to 500mm]      │
    │ What changes                               │
    │   divider.height  500 → 540 mm             │
    │ It fits                                    │      the verdict, in words and tint
    │                         [Cancel] [Apply]   │
    └────────────────────────────────────────────┘
    depth                      🔒   9mm             nothing it follows can move
    drawer_bay[bottom] | drawer_bay[top]  250mm  ›  a boundary between pieces
  ▸ Part sizes                                    6
    Click a size to change it. Nothing is written until Apply.
```

- **What has sizes.** Selecting an assembly (in the tree, by its chip
  in the dock, with the finder, or by clicking one of its parts on the
  model) lists the sizes of its top-level assembly, then, when it fills
  a piece of a split, the boundaries between the split's pieces
  (`grips::grips` appends `split_grips` of the space it fills). A click
  in the envelope lists the envelope's three sizes beside the menu "In
  this space"; a click in one piece of a split (`drawer_bay[bottom]`)
  lists the boundaries between its pieces. Selecting something else, or
  letting the space's menu go, takes them away. When the card appears
  the Edit panel comes forward (opening where it last was), as for any
  card, unless it shares a frame with the 3D view.
- **Which rows.** Every size the session lists has a row, numbered by
  its place in the listing (as `stave edit` and the edit scripts number
  them): an assembly's params, the envelope's sizes, a split's
  boundaries, and the selected part's length and width. The other parts'
  lengths and widths, which mostly follow those params, stay in the list
  when there are four or fewer (a one-part assembly's) and are folded
  under **Part sizes** when there are more (a drawer's sides, front,
  back and bottom). A name the card's head already says is shortened
  (`apron_front.setback` under `apron_front` reads `setback`); the full
  name and its value are the row's tooltip. A row says "follows
  overall_width, legs.inset", "solved" or "locked: follows
  oak_45_sq.width (a size of the stock)" under its name, never by colour
  alone; a size nothing can move shows a padlock and can't be opened.
- **The size editor.** Clicking a row (or Enter on it) opens the editor
  in its place; one is open at a time, and opening another drops the
  first's change. It holds: the name, "was 500mm" and the ⋮ menu; the
  value in a large field, with how far it moved ("+40mm") in the
  change's tone; a **slider** (`size_slider.rs`) showing the size's
  range with an eighth more each side, the ends past its stops hatched
  red, a tick at the old value and a round thumb at the value; buttons
  that step it **−10, −1, +1, +10** (millimetres for a length, degrees,
  pieces) and **back to 500mm**; **What changes** (the size's own change,
  then what the session says else changes: "Changes overall_width 500mm
  to 520mm"); the sizes to pick from, as radios, when it follows several
  plain params ("Which size changes?"); the verdict in words, in a band
  tinted blue while it is checked, green when it fits, red when it
  doesn't; and **Cancel** (Esc) and **Apply** (Enter, only when it fits).
  The change card is not shown for a size's change: the editor is it.
- **Moving the value.** The slider is pressed or dragged (it never moves
  under the pointer), a step button pressed, the arrow keys used on the
  slider (Ctrl: ten), or a value typed and Enter pressed. The value
  shows at once, snapped to the size's steps and held inside its range
  (`grip_values::drag_target`); "Checking the design…" shows with a
  spinner; then the change is tried (`BeginDrag`, `DragTo`): at once for
  a step or a typed value, and for the slider only once it rests for
  120 ms or is let go, so a check (about 20 ms in a release build) never
  holds up the thumb. The ghost on the model shows the design as it
  would be, tinted by the verdict. Held at a stop, the verdict names the
  rule ("Stops at 180mm, because at 179.5mm
  'drawer[bottom].box_is_useful' fails: the drawer box is too shallow;
  make the opening taller. It fits").
- **A typed value.** What is written is always the number typed: a
  value outside the size's range (its stops once found; before, its
  `within`, or how far a size reaches) or off the project's resolution
  is refused before it is tried, under the field ("Can't be 2900mm: this
  grip moves it between 780mm and 2780mm", "Can't be 912.3mm: it goes in
  steps of 0.5mm; the nearest is 912.5mm"). Enter again on the value
  just tried applies it.
- **Writing it.** Nothing is written until Apply (or Enter) on a change
  that fits; a value Apply waits for while it is checked is written once
  the check says it fits. After the commit the sizes are listed again
  from the new design and the row is marked "written" for a moment.
  Cancel, Escape or "back to …" drop the change.
- **Stops.** Once listed, every movable size's stops are searched on a
  worker thread (`Session::grip_stops_job`, cancelled when the sizes
  change), each answer handed to `Session::grip_stops_found` as it
  comes; a size's range, and its slider's hatched ends, narrow to its
  two last good values.
- **Letting the solver move it.** The editor's ⋮ menu opens "Let the
  solver move this", for a param of the project (the size's own, or each
  one a size follows; a component's param can't change kind). Turning it
  on asks for the range the solver keeps it within (a fifth either side
  of the value, to retype) and **Make it solved** sends `MakeSolved`;
  turning it off on a solved param sends `MakePlain`. That change shows
  on the change card, with its size section, verdict and Apply.

The edit script steps for sizes (`ui/edit_view/grip_script.rs`) do the
same things without a pointer, sizes numbered from 1 as `stave edit`
numbers them:

| Step | What it stands for |
|---|---|
| `grips assembly <name>` | the assembly selected: its sizes in the Sizes card |
| `grips envelope`, `grips split <piece>` | the envelope's sizes, or a split's boundaries (as a click in the space lists them) |
| `stops` | every size's stops found now |
| `drag <grip> <value>` | the size's editor opened and its value moved there, checked, not written |
| `release` | Apply: written when it fits |
| `type <grip> <value>` | the editor opened, the value typed, Enter pressed (and Apply, when it fits) |
| `pick <param>` | a size from the radio list a follows grip offers in the Edit panel |
| `menu <grip>` | the editor's ⋮ menu opened (a popover: not in a screenshot) |
| `solver <grip> <low> <high>`, `plain <grip>` | the solver's switch turned on with that range, or off |

Two words are shared with the changes' steps (`edit_view/change_steps.rs`):
`pick` is a size's while a follows grip's list waits for a pick, and
otherwise selects an item by name; `drag count …` and `drag boundary …`
set a repeat's count and move a boundary, any other `drag` a size's.

Pictures (`docs/screenshots/window/`), each from an edit script in
`tools/window-edits/` on a copy of `examples/clothing_cabinet_from_box`
or `examples/side_table_from_box`, recorded by `tools/gui-screenshots`:

| Picture | Script | Shows |
|---|---|---|
| `grip_divider.png` | `grip_divider.edit` (`--select divider`) | the divider's Sizes card: its height, its depth, its panel's sizes |
| `grip_drag.png` | `grip_drag.edit` (`--select divider`) | the height's editor at 700mm: +200mm, the slider, what changes, "It fits", the green ghost |
| `grip_stop.png` | `grip_stop.edit` (`--select divider`) | moved down past its stop: held at 180mm, the verdict naming the rule |
| `grip_typed_refused.png` | `grip_typed_refused.edit` (`--select divider`) | 120mm typed: refused, why under the field |
| `grip_follows.png` | `grip_follows.edit` (`--select top`) | the table top's length dragged: "Changes overall_width 500mm to 560mm", everything that follows it in the ghost |
| `grip_follows_pick.png` | `grip_follows_pick.edit` (`--select apron_front`) | the apron's length, which follows two plain params: the radio list to pick from in its editor |
| `grip_envelope.png` | `grip_envelope.edit` | a click in the envelope: the menu, and the three sizes in the Sizes card |
| `grip_solver.png` | `grip_solver.edit` | the height made solved within 1500mm..2000mm, waiting for Apply |

```
tools/window-shot grip_drag.png <copy of examples/clothing_cabinet_from_box> --select divider \
    --edit-script tools/window-edits/grip_drag.edit
```

`release` and `type` write when the value fits: point them at a copy.

**Not in the window yet.** Sizes of a nested assembly on their own
(only the top-level assembly's are listed), and of a whole group of
parts, are not offered. The ⋮ menu is a popover, which the screenshot
tool can't draw. The solver's switch is offered for the project's own
params only.

### Changing what is there

The changes of the design that don't come from the menu
(`docs/editing.md`, "Changes of the design") each have a gesture, a
control on a card, and a verb of the finder; all three call the same
method of the window (`repeat_selected`, `mirror_selected`,
`hold_from_selected`, `remove_selected`, `rename_selected`), which sends
the session's command. Each change is tried at once and shown on **the
change's card** (`edit_view/changes.rs`, in the Edit panel, in the
card's place), drawn as the Inspector is: a header row (the item's name
in a monospace field, the kind of change on the right), then sections —
**Change** (what it does, "Remove back", and what it found: the mirror's
names, the rule a hold writes and its reason, what a removal takes, the
files a rename rewrites), the grip's **Size** for a drag (see
[Sizes](#sizes)), **Verdict** (an icon and words, "It fits" or "It
doesn't fit: 1 problem" with the checker's problems and a button per
fix — never colour alone) and **Pending** (the lines Apply would write,
see [The text while editing in 3D](#the-text-while-editing-in-3d)) —
and a footer with **Cancel (Esc)** and **Apply (Enter)**. Every commit
button reads Apply, whatever the change (`changes::Commit`): a removal's
keeps a destructive look, and its Change section says what will be
removed. The edit card for a chosen candidate has the same footer and
sections (its name in the header's field, Placed in with each space and
part as a link, Fixes added, Verdict, Settings, Pending); the hold card
too, its button "Try it" since it checks the rule before the change's
card offers Apply. Its ghost is drawn as an edit's is, from
`Change::ghost`; a removal is drawn on the design as it is, what would go
see-through and red, since the candidate has nothing there to draw.
Nothing is written before Apply; Cancel (or Escape) leaves every file as
it was. Once written, the window selects the item by its new name.

- **A selected item's changes** (`edit_view/item_tools.rs`): with a
  part, a piece of hardware or an assembly selected, the Details panel
  ends, under the selection's account, with "CHANGE drawer": the item of the project it belongs to (its outermost assembly,
  or the part itself, without its index: `drawer[top].front` changes
  `drawer`) with its **name**, a `gtk::EditableLabel` (retyped, it sends
  `RenameItem`); **How many**, a stepper with which way the pieces split
  (stacked up, side by side, front to back; the way shows only for a
  first repeat), which sends `RepeatItem`, shown when the item fills a
  space of the design; **And the other side**, a switch. All three come
  from `Session::item_offer` (worked out once per item and result,
  since it reads the project; the window decides none of it): the
  pieces and whether the stepper shows, and the `MirrorGesture`. With
  `Image`, the switch reads **And the other side · adds apron_back**
  and on, it sends `Mirror`; the change's card shows the same switch,
  on, which cancels when turned off. With `Split`, a handed single
  piece that fills a whole space (a door filling the space above a
  divider, hinged on the left side), it reads **And the other side ·
  splits its space across into a mirrored pair** and on, it sends
  `MirrorSplit { axis: across }` (the space split in two, `door_left`
  and `door_right = mirror(...)`; see
  [editing](editing.md#repeat-and-mirror)). With neither (a top, a
  case, a shelf, a divider, or an item that already has its mirror
  image) the switch is hidden. A mirrored pair split up or deep has no
  gesture yet. **Hold a distance…** and **Remove…**.
- **Repeat** (`edit_view/repeat_handles.rs`): the card of a candidate
  that fills a space (a component with its `Region` bound) has the same
  stepper above its buttons, sending `Repeat` (one piece is no repeat).
  **Everywhere, the repeat controls are folded** behind a small
  "Repeat…" (on the line of a card's Cancel and Apply; in Details, in the
  stepper's row) and open to the count and the direction only when it is
  pressed, or when the count is already more than one: one rule for every
  component and item, `stave_app::item_offer::repeat_fold` (a case
  repeated in the envelope is valid, two cases side by side, but rarely
  wanted, so it takes no room until asked for). On the card the open row
  says "×" in place of "How many" (in its tooltip), so it fits the panel.
  Nothing on the model is dragged for a repeat: the ghost shows every
  copy once it is tried. Which space an item fills is read from the
  model, not from the picture: the region its component's first `Region`
  param is bound to (`stave_app::spaces::filled_spaces`, following `bay`,
  `drawer_bay[same]`, `case.inside`); the same says whether How many shows
  in Details. Whether a repeat fits is the session's to say.
- **Shares**: with an item selected whose space is one piece of a split
  (`drawer_bay[bottom]`), the boundaries between neighbours are rows of
  the Sizes card ("drawer_bay[bottom] | drawer_bay[top] 250mm"), changed
  in the size editor like any size. The boundaries come from the session
  (`grips::split_grips`, and `stave_app::spaces::split_boundaries` for a
  script): the region array, which pieces in label order, their sizes
  and the axis; a boundary moved settles where `SplitBoundary::settle`
  says (an even division of the two pieces, the whole split's even size,
  or a round size when near one, else a step of the project's
  `resolution:`).
- **Keeping clear of the panels**: a problem's pin is moved, by the least distance, off any panel
  floating over the 3D view that they would cover (the view's own
  controls: the scene menu, the camera column and Pull apart's slider,
  the moving assemblies' chips, the motion slider, the line and the key
  at the top, the cards and the chip at the foot; and the editing's
  menu; the editing's cards are in the Edit panel, off the view; no
  verdict pill stands on the model: the ghost's tint and the card say it),
  and a pin with no room is listed in the off-screen chip.
- **Hold this** (`edit_view/hold.rs`): "Hold a distance…" among a
  part's changes in Details (or "Hold a distance from this space…" at the foot of a space's
  menu) asks, in the line at the top of the view, for the second thing;
  the next click on a part or a space is it. **The hold card** (in the Edit panel) then says
  "divider.panel to case.top · It is 462mm now" (`Session::measure`, or
  why it can't be measured), and asks At least or At most, the distance
  (starting at the distance now) and why (the rule's message); "Try it"
  sends `Hold`, and the change's card shows the rule. An assembly is not
  measured: its card says to pick one of its parts.
- **Remove**: Delete (or "Remove…") on a selected item sends `Remove`.
  When other things use it, **a dialog** in the middle of the view, over
  a dimmed backdrop, names each ("These use divider, and can't stay
  without it: • drawer_bay • drawer") and offers Cancel or "Remove
  divider and all 2" (`IncludeDependants`); the change's card lists them
  too. Either way nothing goes until Apply on the card (red, for a
  removal), after the red ghost shows what goes. (The dialog is part of the window rather than
  an `AdwDialog`, which the window's screenshot didn't draw when it was
  opened by an edit script.)
- **Rename**: the item's name in Details, retyped (`RenameItem`).

### The finder's verbs

The finder (Ctrl+K) lists, before what it finds, under "Do · Enter does
it", what a verb typed in it would do (`edit_view/verbs.rs`, through
`Finder::connect_actions` and `connect_action`); Enter runs the first.
"add shelf" (or "add hanging rail", "add casework.drawer") with a space's
menu open lists each entry whose label holds the words, and runs the
same `Choose` a click on it sends; with a part selected and no menu, it
starts the edit for the face last clicked on it (else for the part) and
chooses the first entry that matches, or says nothing called that goes
there. "remove" (or "delete"), "rename <name>" (bare: the name field on
the card), "repeat 3 across" (or "make 3"), "mirror" (or "other side")
and "hold" do what the card's controls do. With nothing selected, it
says to select something first.

### The text while editing in 3D

While an edit or a change is in progress (`edit_view/editor_link.rs`),
the lines its patch would write show in the **Pending** section of the
card in the Edit panel (a collapsible section like any other, last
before the footer), from the last try (`Tried::changed` against the text
the editor holds), as a small diff:

```text
▾ Pending
  Apply would write this to small_chest.stave
  @ line 39
  + assembly shelf : casework.Shelf {
  −   panel_stock: oak_19
  +   panel_stock: maple_12
```

Each place starts with its `@ line N`; each inserted line is marked "+"
(green), each removed one "−" (orange), and a line whose word or value
changed is a "−" line and a "+" line with the word that changes in bold
(the old one struck through); a line whose only change is its spaces
(the formatter lining up a run of declarations again) shows "≈" and says
so. The lines are compared by a line diff (`stave_lang::changed_line_runs`,
the one patches use), so a change in several places shows each under its
own `@ line 13`, `@ line 31`, and the note says "This change would change
small_chest.stave in 2 places"; the lines between the places are neither
shown nor marked. Nothing is put into the Code panel any more; when the
editor is open it goes to the first place, and in the text itself only
the old lines each place replaces are struck through. A rename alone,
tried without a patch, shows each line of the files it rewrites with the
name changed as a word. They go when the edit is cancelled or applied.

**While the text doesn't read** (an error of layer 1, a syntax error, in
one of the project's files, as the editor's analysis has it), editing in
the 3D view is off: a click in a space or a gap, the item's changes,
Hold, a size's change, Delete, the finder's verbs and an edit script's
steps start nothing but a notice saying why; the "Add something onto
its face" button, the item's tools and the Sizes card are not offered;
and a line low in the view, just above the dock, wrapped and centred,
says why ("Editing in 3D is off while
small_chest.stave doesn't read: expected end of line, found 'space'. Fix
the text first."); it stands there, not at the top, so it covers neither
the parts list nor a card.

### A new project

"New Project…" in the main menu asks for a name, the three overall
sizes (width across, height up, depth front to back), the wood (its
name, species, how it is bought, its thickness and the longest board
you can get), the libraries to import (the standard modules outside the
prelude, `std:casework` and `std:frames`, ticked) and the tools you have
(the standard library's tool kinds, read from its text; none ticked
means every tool), and a folder. Create writes the project with
`stave_app::change::new_project_file` and the editor's one writer
(never over a file that is there), opens it, and lights the envelope
(`naming::ENVELOPE_REGION`) as the first space to click, with "Click
inside the box to add the first piece" at the top of the view. What is
missing is said under the fields.

A project with spaces but no parts yet (however it was opened) is drawn,
not shown as "No parts yet": the camera frames its spaces (a scene with
no nodes has no bounds of its own), every space is lit until an edit
starts, the same line asks for the first click, the model tree says "No
parts yet. Click inside the box to add the first piece." rather than
"Nothing to show yet", and a click in the box opens the menu "In this
space" (`new_project_opened.png`, `new_project_menu.png`).

### Pictures

`tools/gui-screenshots` records, with the edit scripts in
`tools/window-edits/`: `edit_space_lit.png` (the pointer in a new
project's case, lit, the case's front see-through), `edit_space_menu.png`
(the menu on that space), `edit_face_button.png` (Details for a part
offering its face), `edit_door_refused.png` (a door that swings into the
wall, refused, its fix offered, the ghost red),
`edit_door_fits.png` and `edit_door_fits_dark.png` (the fix applied,
fitting, the ghost green), `edit_door_added.png` (the design after Add,
the door selected), `edit_door_added_editor.png` (the same with the
editor open: the change unsaved in the text), `new_project.png` (the
dialog), `new_project_opened.png` (a project it made, opened: the
envelope drawn and lit, waiting for the first click) and
`new_project_menu.png` (the menu a click in the envelope opens).
`edit_setting_options.png` and `edit_setting_options_dark.png` (the
owner's case, `case_test.script`: Case chosen for a new project's
envelope, `setting_options.edit` opening the search over its joinery
with nothing typed: every kind, those that fit first, the refused ones
greyed with the checker's reason), and `edit_setting_blame.png` and
`edit_setting_blame_dark.png` (`setting_blame.edit`: chigiri typed, the
card saying first that it stopped fitting when the joinery changed, the
put-back button, the checker's words below). The door
scripts point at the hinge side with `point`, as the pointer would. The
door is `crates/stave-gtk/fixtures/door_hits_wall`; the small chest is
made by `stave edit --script tools/window-edits/small_chest.script`.

The changes, on the small chest with a divider and two drawers
(`chest_drawers.script`) and the side table as far as its front apron
(`table_apron.script`): `edit_item_card.png` (a drawer's card with its
changes, its sizes with the boundary between the drawer bays),
`edit_repeat_card.png` (a drawer chosen for a space, the count set to
× 3, every copy in the ghost), `edit_repeat_item.png` (the
drawers made three from Details), `edit_shares_dark.png` (the
boundary moved to 180mm), `edit_mirror_offer.png` and `edit_mirror.png`
(the apron's "And the other side", then on), `edit_hold_card.png` and
`edit_hold.png` (the hold card, then the rule tried),
`edit_remove_dialog.png` and `edit_remove.png` (the dialog naming what
goes, then what goes in red), `edit_rename_editor.png` (a rename pending
in the text), `edit_finder_verbs.png` and `edit_finder_add.png` (the
finder's "repeat 3", and "add shelf" opening Shelf's card),
`edit_pending_editor.png` (a shelf's lines pending in the editor),
`edit_pending_two_places.png` (the drawers made three with the editor
open: two places pending, the bays' line and the drawers' line, and the
envelope's line lined up again) and `edit_text_broken.png` (a brace
typed: editing in 3D off, and why).

The Edit panel, on the owner's case (`case_test.script`) and on a
project whose case is joined with screws (`recent_choices.script`):
`edit_panel_case.png` and `edit_panel_case_dark.png` (`card_panel.edit`:
Case chosen for the envelope, its card a tab beside Details, one compact
row a setting, the defaults dimmed in their entries, "Repeat…" folded
beside Cancel and Add), `edit_panel_right.png` (the same with the Edit
panel on the right edge, which is wider: the arrangement read from
`tools/layouts/edit_right.layout` as a saved one would be, since the
screenshot tool can't drag a tab), `edit_repeat_open.png`
(`repeat_open.edit`: "Repeat…" pressed, the count and the direction
open), `edit_short_list.png` and `edit_short_list_dark.png` (`short_list.edit`:
a shelf's joinery's short list: dado, the default, which fits; screw,
which the case uses, refused here and greyed; "Search all 18…") and
`edit_option_search.png` and `edit_option_search_dark.png`
(`option_search.edit`: the shelf's joinery searched for "edge": groove
fits, the other four greyed with their reason; glued_edge's is "Shelf
gives its joint a depth, and glued_edge has no depth").

### The acceptance run

`tools/window-edits/acceptance/run` builds the clothing cabinet and the
side table from empty projects in the real window, one `tools/window-shot`
run a stage, each stage twice (its card, then with Add and `save`), and
records `docs/screenshots/window/accept_cabinet_*.png` and
`accept_table_*.png` (`accept_new_project_dialog.png` is the New Project
dialog). It ends by comparing what the window wrote with the pinned
`tests/edit-scripts/clothing_cabinet.stave` and `side_table.stave`. Each
stage's steps are in `tools/window-edits/acceptance/`. The project
itself is made by `stave edit --script` with the dialog's writer: the
dialog's fields have no script step, and it takes one wood (the cabinet
needs six, the table two). Where a face is out of sight the view is
turned first (`turn`, or `--turn`/`--pitch` for a whole stage), as a
person would orbit it; one face, the front apron's left end (the cleat's
`start`), is inside the leg's mortise and cannot be clicked from any
side, so that one is filled with `bind`, which stands for the click
without making it. See [STATUS](STATUS.md#graphical-editing) for what
the run found.

What the run exercised, and what it did not: only `point face` (21
steps) goes through the scene's real pick, as a click would. The 16
steps that open a menu (6 `select region`, 6 `select between`, 4
`select face`) and the 3 `bind between` steps find their space, pair or
face by name and send the command the click would send; `select face`
does not press the Details button. So the run proves the session, the
menu, the card, the writer and picking faces, but **a click in a space
and a click in the gap between two parts were not exercised through the
real window's pick**; those picks are tested below the window
(`crates/stave-scene/tests/picking.rs`), not in it.

### The widgets

The menu and the card take contract values and report what the user
does; they decide nothing.

- **The menu** (`EditMenu`, `.stave-edit-menu` on `.stave-card-panel`):
  `show(Heading, &[MenuItem])`, where a `MenuItem` is a `&Candidate` and
  `Checked::Done` or `Checked::Pending` (an entry still
  `Applicability::NotChecked` shows as being checked too). The heading is `Heading::text` ("In this
  space"); entries are grouped by `MenuGroup`, the project's own first,
  then imported modules by name, then the standard ones ("Standard ·
  joinery"), and within a group the ones that fit or need a choice
  (saying what, in the warning colour), then those being
  checked (a spinner and "Checking whether it fits here…"), then the
  greyed ones (the checker's messages in the error colour), each in name
  order. Each entry is a real button with its label and hint; one that
  can't be chosen stays focusable so its reason can be read, is marked
  disabled for assistive technology and never reports.
  `connect_chosen(Fn(Declaration))` reports the entry chosen.
- **The card** (`EditCard`, `.stave-edit-card`, the Edit panel's
  content while an edit is in progress): `show(&CardModel, &dyn Words)`.
  `CardModel` is the heading, the `&Candidate`, the generated name, the
  `Option<&Verdict>` (`None` while the candidate isn't complete) and the
  labels of the fixes added. `Words` is the caller's: an expression as
  written, an assembly's name and what a bound place param holds, which
  the contract carries as ids and parsed expressions. From the top: the
  title and where it comes from, the hint on one line; the name in a
  `gtk::EditableLabel` and the place params (each unbound one says its
  `PlaceParam::prompt`, or "Click a part on the model for hinge side"
  without one; a bound one what fills it), one compact row each; the
  fixes added; the verdict (Checking with a spinner, "It fits" in green
  with any warnings, or "It doesn't fit: 1 problem" in red with each
  problem as `stave check` words it, `error[overlap]: …`, its notes and
  `help:` line, and a button per fix labelled `Fix::label`), in a box of
  its own that scrolls past 120 pixels (the blame, the verdict's line
  and the fixes' buttons outside it, always shown); then the settings,
  one row each:
  those that need a choice first, then those with a value, then those at
  their defaults (the default's value dimmed in the entry), each with the
  ▾ that opens its short list (`stave_app::choices::short_list`) under
  it. `setting_about` is a row's tooltip ("JointKind · the default,
  dowel(count: 4)"). At the foot, always in sight: `repeat_slot` (the
  open count and direction) and, on the buttons' line,
  `repeat_button_slot` ("Repeat…"), where the window puts its
  `RepeatFoldRow`; then Cancel and Add. Add is enabled only on Fits with
  nothing to point at or choose (`can_add`), and its tooltip says why not
  (`add_blocked_reason`). `CardModel::blame` puts the blame and the
  put-back button first in the verdict. `update_options(&Candidate)`
  redraws only the short lists; `open_options(param)` opens one.
  `connect_action(Fn(CardAction))` reports `Rename(text)`,
  `Choose { param, text }` (Enter in a setting's entry, the text as
  typed, or an entry of its short list or of its search), `UseDefault {
  param }` (the short list's default), `Search { param }` ("Search…",
  which the window opens), `Fix { problem, fix }` (indexes in the
  refusal), `PutBack`, `Add` and `Cancel`. Enter adds when Add is
  enabled; Escape cancels. The card is focusable and takes the keyboard
  when it is mapped (its tab raised) and on each `show` (unless the
  keyboard is already inside it), so these keys reach it before the
  window's own Escape.
- **The search** (`OptionSearch`, over the window's panels):
  `present(setting, query)`, `connect_search(Fn(&str) -> Vec<SearchGroup>)`
  (the window answers with `stave_app::choices::search` on the session's
  setting), `connect_chosen(Fn(setting, text))`, `refresh()` (after each
  option's fit is found) and `close()`.
- **The Edit panel** (`EditPanel`): the three cards (a candidate's, a
  change's, the hold card), one shown at a time by its owner, or "Nothing
  being added" and how to start one; `update()` says which.

The window's shell around them (see [The window](#the-window)) is drawn
by a few widgets that also decide nothing:

- **The title bar** (`Header::new(name, &menus)`): the menu bar built
  from `main_menu()` (four submenus), the project's name, the switcher,
  the find button, the problems chip and the hammer; `show_title`,
  `show_view`, `show_pill`, `show_build`.
- **The tab strips** (`TabStrip`): `tab_strip::set_page_tabs(id, Fn() ->
  Vec<PageTab>)` lets a panel show its own tabs (the Code panel's open
  files, from `FilesPage::page_tabs`), and `page_tabs_changed()` makes
  every strip again.
- **The status bar** (in `Workbench`): `show_problem_count(n)`; its
  Problems and Output buttons call `present` on their panel.
- **The floating tool groups** (`floating::Floats::new(figure, controls,
  spots, orient, on_dock)`): places every control with
  `view_layout::place`, lets a group be held and dragged, calls `orient`
  to lay a group flat or upright and `on_dock` when one lands
  (`Workbench::dock_tool` remembers it); `refresh()` after a layout is
  applied.
- **The Output panel** (`OutputPanel`): `add_line(text)`.

To see them, `--edit-fixture menu|greyed|refused|fits|choose|point`
shows one from `clothing_cabinet_fixture()` (the menu over the design
near the left door, the card in the Edit panel) and prints what is done
on it:

```
tools/window-shot menu.png examples/clothing_cabinet --edit-fixture menu
tools/window-shot refused.png examples/clothing_cabinet --edit-fixture refused
tools/window-shot fits.png examples/clothing_cabinet --edit-fixture fits --theme dark
```

The pictures are `docs/screenshots/window/edit_*.png`. The greyed
entry's reason in `--edit-fixture greyed` is illustrative: the fixture
has no greyed candidate. Their own few colours (all libadwaita's named
colours) are a stylesheet in `edit_card.rs`, installed once per display.

## Slots for the next lanes

- **Grips**: built, as the Edit panel's Sizes card; see "Sizes" under
  "Editing in the 3D view".

- **The editor**: built; see "The editor" below. "Show in text" in a
  problem's details and the joint kind's link in focus mode send
  `GoToSource` (the kind's link may name a `std/...` file, opened read
  only).
- **Cut list**: the `cut-list` page is a "Cut list is coming" status page,
  out of reach (switcher hidden, no Ctrl+2) until it is made. No page of
  the window sends the reader to a command.

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
                         │ [Move chips]  [slider]     │+│ Ctrl+click follows a name …  Line 9 │
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
  underlines from the fast tier and the last check (drawn by the
  text view itself in libadwaita's error and warning colours for the
  style, wavy and 1.6 px thick for an error, dotted for a warning, with
  a bar of the same colour in the gutter beside each line that has one,
  whole for an error and dashed for a warning; every place a check's problem points
  at is underlined, so a standard file open in a tab shows the kind's
  rule a requirement comes from; the check's underlines need only the
  text the check read, not the names worked out), a lens above each part, assembly, piece of hardware,
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
  Resting the pointer on a word, or Ctrl+I at the cursor, shows its
  card (and gives the text its words as an accessible description while
  it shows): the problem there with
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
  with the page's overlay; the underlines go on only while the text is
  still the text checked, compared with the text the result itself
  carries, so a file changed on disk and reloaded is underlined too). Saving writes the file; the watcher's check then
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
"Preview" (only an edit's card applies fixes); the model tree shows
what isn't involved in a posed problem as it is, dimming it in the view
rather than hiding it; the finder opens as a dialog in the middle of the
window; the panels are docked (the canvas's "Stave docked layouts") rather
than floating over the view.
