# Status

What works today, how to try it, and what is not done yet. The language
is specified in [grammar.md](grammar.md); [test-coverage.md](test-coverage.md)
maps every rule and diagnostic code to the tests that pin it down.

## What works

**Checking** (`stave check`), layer by layer, compiler-style, with a file,
a line and a concrete help line for every problem:

1. **Syntax**: a lossless parser with recovery; one mistake is one error.
2. **Names and types**: modules, imports, `std:` and the prelude, kinds,
   components and assemblies, units and types.
3. **Placement and sizes**: every axis set exactly once, `fit`, spacing,
   mirroring, regions and mounts, joints (pairs, `adds:`, made parts, cuts)
   and hardware (hinges, slides, pulls, their members and cuts). Exact
   throughout: lengths are whole nanometres, arithmetic is exact fractions.
   Hinge, slide and pull kinds are made of members, each with any name, a
   `///` doc comment and at least one shape (grammar, "Shapes"); a member
   without a shape is `missing-field`. Nothing in the core crates knows a
   kind or a member by name: the standing rule (`CLAUDE.md`, "No kind in
   the core") is held by `stave-check/tests/standing_rule.rs`.
   **Boards**: a `solid` part wider across the grain than its stock's
   `max_width` is made of equal boards (in `--json`), joined by the edge
   joints its `boards:` names (or the project's, or else the prelude's, `glued_edge`):
   `tongue_and_groove` widens each board by its tongue, `splined_edge` makes
   a spline per seam, `chigiri` lets a butterfly key into the face across
   each seam, `suitsuki_zan` slides two battens into dovetail housings
   across all the boards, `glue: false` is kept. The collision checks see
   the part's wood as its boards make it, angled cuts included. A sheet
   wider than `max_width` is an error, as a part longer than `max_length`
   is, unless it is spliced. **Splices**: a part's `lengths:
   spliced(by: kind, at: d, from: end)` makes it pieces joined end to end
   by an ordinary joint kind (`lap_splice`, `kanawa_tsugi` in std), each
   piece a part and each splice a joint, so the checks, focus mode and the
   pulled-apart view work on them; every piece must fit `max_length`, and a
   cut of another joint reaching into a splice is `wall`. A stock with no
   `max_length` is a warning.
4. **Fit and geometry**: every part is cut into a solid (its cut box minus
   its cuts: exact boxes for axis-aligned cuts, exact cylinders for holes,
   round tenons and round sections, and exact prisms, a polygon on a face
   cut straight down, for dovetails, `outline`s and the chidori's angled
   slots; an outline that crosses itself is `type-mismatch`). Wood
   passing through wood beyond the 0.05mm contact tolerance is `overlap`,
   with what goes where and by how much ("the tenon on rail goes 3mm past
   the bottom of its mortise in leg"). Messages call a cut by the name its
   kind gives it (`cut groove: ...`), whatever its operation; an unnamed
   cut in a kind is the warning `unnamed-cut`, two cuts of one kind with
   one name are `duplicate-name`, and every cut in `std/` and `examples/`
   is named. A joint's parts that don't touch are `no-contact`; a spacing
   that runs past `to` is `overlap`; a kind's catalogue rules are its own
   `require`s and `adjust`s (a slide's gap, how far a cup hinge's arm can
   be set on its plate). Every shape of every hardware member is a
   body and counts like wood, against its own parts too (a cut it makes is
   room for it); a kind's own members are not checked against each other.
   A part made of boards counts as its boards make it. A clash between an assembly (a door, a drawer) and something
   outside it is one error at the line that placed the assembly, in the
   assembly's own terms ("make its top overlay 10mm smaller").
5. **Motion**: every scene's poses are collision checked; every door and
   drawer is swept through its whole travel from rest and from every scene
   and reported at the first angle or distance it hits something
   ("door_r.panel hits door_l.panel when door_r is opened past 71.5deg");
   a door that binds as soon as it opens is told the gap that clears it.
   Every motion is the mechanism its kinds declare, solved by one planar
   solver: a rail travels exactly, one pin turns in closed form, and
   anything else (a cup hinge's four-bar: an arm, two links, the boss in
   the cup, `docs/hardware/cup-hinge-linkage.md`) is posed step by step, so
   the door turns and moves forward and out at once, swept by conservative
   advancement and bisection to within the contact tolerance, and every
   member the mechanism moves is checked against everything else. The
   solver proves, not samples, that no step comes within 0.1mm of a dead
   point over the whole range: between each two samples it bounds how
   fast each step's margin can change from the declared geometry and
   halves an interval it can't prove, down to 0.00005deg or 50nm
   (`stave-geom/src/planar/proof.rs`). The 110deg cup hinge is proven
   with at least 1.016mm to spare and the 165deg wide hinge with 0.190mm,
   neither needing a halving. A slot's ends are still checked at the
   samples only.
6. **Requires** at the current values (over whole ranges: not yet).
7. **Assembly sequences**: every joint's `insert` steps are run backwards
   from the finished joint and swept; a step that can't be undone is
   `order`. The chidori's quarter turn and diagonal slide come apart as
   `stave-app/tests/oracle_chidori.rs` proves, and fail without its flats.

**Formatting** (`stave fmt`), **dumping** the solved design (`stave dump`,
`--json`; under each piece of hardware whose kind has `adjust` entries the
text dump prints one line of its solved settings in the kind's own words,
`arm set -0.5mm across`, joined with `, ` when there are several; the
JSON carries them as `settings`), **glTF export** (`stave export`, posed in any scene, exploded or
not), and the **viewer** (`stave view`).

## How to try it

```
cargo build --release -p stave-cli -p stave-gtk
target/release/stave check examples                      # 0 errors, 0 warnings
target/release/stave check tests/broken/door_hits_side   # one broken design per diagnostic
target/release/stave dump examples/clothing_cabinet
target/release/stave export examples/clothing_cabinet --gltf apart.glb --explode 1
target/release/stave view examples/clothing_cabinet
```

`tests/broken/` holds a small design for every diagnostic, each with the
exact message it must produce on its first line.

## The viewer

`stave view <folder>` opens the libadwaita window of the design canvas
(the V2-C "Spatial" screens with option A's rail of open files;
[gui.md](gui.md) describes how it is built): a title bar with back and
forward (Alt+←/→), the project title, the finder's field (Ctrl+K), the
Design / Cut list switcher, the problems chip and the build button; and
the design: the 3D view filling the window, the model tree floating over
it, the chosen part's card beside it, problems pinned where they happen,
the dock at the bottom, and the rail of open files at the right edge with
the editor sliding in beside the view. Every part is drawn with its cuts made (tenons, mortises, dados,
rabbets, grooves, holes, hinge recesses, the chidori's diagonal slots and
flats), in species colours, with orbit, zoom and pan. Light and dark follow
the system or the main menu, the 3D backdrop with them.

- **Background checks**: saving any `.stave` file checks the project again
  on a worker thread once the folder is quiet (250 ms); the new result
  replaces the old one whole, and the selection and back/forward history
  follow their items by name. The pill says "No problems" (green), "N
  problems" (red; amber for warnings only) and spins while a check runs,
  keeping its last colour and words; its tooltip says what the check
  found, and clicking it (or F8, Shift+F8, its arrows) steps through the
  problems.
  A design that stops solving keeps its last good version on show under a
  banner.
- **Model tree and card**: the floating tree lists the carcass, then
  each assembly with its motion ("turns 110°", "slides out 500 mm",
  "mirror of door_l"), its parts and hardware, with stock swatches and a
  dot on every row with a problem (red, amber, or a red ring for a part a
  problem only involves); under the chosen part, the joints it is in, each
  opening focus mode. Choosing in the tree, the 3D view or the finder
  frames it at once (no flying), dims the rest of the design and opens its
  card beside it: the kind and assembly ("PART · IN door_l"), the doc
  comment, stock, finished size, grain and how it moves, its joints and
  hardware cuts (each opens focus mode), and the declaration as written,
  highlighted, with a file:line link that opens the editor there.
- **Finder** (Ctrl+K): one field over the design's parts and assemblies,
  the project's files and names, its lines and its problems, grouped,
  with a filter per kind; the arrows show a part in 3D, Enter goes there,
  Ctrl+Enter opens its text.
- **Dock**: the scene menu, Pull apart, X-ray (everything but the choice
  see-through), the chosen door's or drawer's slider, and a chip per
  moving assembly.
- **Editor** (`docs/gui.md`, "The editor"): Ctrl+E, the rail's arrow or
  any link slides it in from the right; each open file is a tab on the
  rail with a dot for its worst problem. It edits the real files (a
  GtkSourceView 5): `.stave` colours from the navigation
  layer's semantic tokens, line numbers, the cursor's line with its blue
  bar, bracket matching, two-space auto-indent, smart Home/End, undo,
  Ctrl+F search in the text, wavy underlines with a card for each problem,
  F12 / Ctrl+click to follow a name (across files, imports and the
  standard library, through the back and forward history), Alt+F12 to
  peek at a definition under the line, Shift+F12 to list every use with
  what it does there ("door_r mirrors it", "scene all_open poses it"), "2
  uses · show in 3D" above declarations, a card with the doc comment on
  hover, Ctrl+S to save (atomically), Ctrl+Shift+I to lay the file out on
  demand. Typing re-checks the design from the open buffers; unsaved
  files are marked and closing asks. The 3D view stays beside the text:
  the cursor on a part, assembly or piece of hardware selects and frames
  it there, and choosing in 3D, the tree or the finder moves the editor to
  the line that places it. See `docs/screenshots/files/`;
  `tools/gui-screenshots` regenerates every window and editor screenshot.
- **Navigation layer** (`stave-ide`, no GTK): definition, uses, hover,
  breadcrumbs, semantic tokens, lenses, search and the fast diagnostics
  at a byte offset, plus open buffers and saving; the future language
  server's core.
- **Build button**: checks, then says "The build plan (PDF) isn't built
  yet"; disabled while the design has errors.
- **Problems**: each problem with a place in the design is pinned on the
  model where it happens (its shared wood, or the parts it is about); one
  whose place is off screen is named on a chip at the view's edge. The
  tree's rows and the rail's tabs carry their dots. Choosing a pin (or
  F8, or the finder) poses the design where it happens (its scene, and a
  door or drawer stopped just past where it hits), tints the parts red,
  dims what it doesn't involve and draws the shared wood as translucent
  red boxes, with a "Shared wood at first contact" key. The card gives the
  problem's details: "ERROR ·
  MOTION", a plain title ("door hits wall as it opens"), the text exactly
  as `stave check .` prints it (highlighted), the ways to fix it from its
  help line, each with "Show in text", and that fixes are never applied
  automatically. A door or drawer that hits something gets a motion card
  ("Swing door to see where it hits") whose slider hatches the blocked
  range red from the contact angle.
- **Exploded views**: scenes with `explode: true` start apart (`--explode`
  sets any amount). The first half of the range moves each assembly out
  whole (drawers forward, doors off their hinges), the second pulls parts
  apart along the reverse of their joints' last insert step; a part held
  from both ends (a rung, an apron) goes with the parts it is held
  between, so the stool's rungs drop with its legs and the legs pull off
  their ends. The 3D
  view's dock has "Pull apart": on, it grows a play button and a
  "together … apart" slider. The
  main menu's "Export View as glTF…" saves the view as posed. The camera
  buttons fit the view, look at the front and turn to the back. A library folder, or a design that doesn't solve, says so in
  the view instead of leaving it blank, and so do the tree and the card:
  a library's tree lists the kinds, recipes and components it defines
  (each opens its text) and its card explains what a library folder is
  and the line that imports it.
- **Cup hinges** are drawn as their mechanism: the steel cup with its
  rim and boss pins in the door, the plate, arm and arm pins on the side,
  and the two links, which turn about their arm pins as the door opens.
  Selecting any part or piece of hardware of a door or drawer (a hinge
  included) offers an "Open it" slider in the dock that opens it, the
  hinges' mechanism moving with it.
- **Sections**: a scene's `section:` cuts the view at a plane; the wood
  it cuts is drawn flat (evenly lit).
- **Every cut drawn**: holes in round and turned parts are bored too (the
  stool's sockets), every drilled mesh is watertight (no hairline cracks
  where a hole's opening crosses a facet edge), and every face a cut made (holes, mortises, dados,
  rabbets, grooves, tenon cheeks, round tenons, hinge recesses) is drawn
  darker, so the joinery reads at a glance.
- **Focus mode**: selecting a part offers "See how it goes together" for
  each joint pair it is in; selecting hardware offers "See how it works".
  Everything else fades to a ghost and the item is framed. The window
  lays focus out as the canvas's JointFocus and Hinge designs: a joint's
  step player floats over the view and its side panel gives the joint,
  the toggles and its insert steps with a link to the kind
  (`std:joinery/socket`); hardware gets a page with a line saying what the
  view looks at, a From above / In 3D switch, three small views at fixed
  angles (click one to go there) and the swing card, and its side panel
  "How a cup hinge works" with its numbered members.
  - A **joint** plays its own `insert:` steps (the steps the assembly
    check runs backwards) with a play button and a scrub slider from apart
    to together; the part that receives it is drawn see-through (X-ray,
    its cut faces nearly solid) and the moving part's home is outlined,
    dashed. The panel gives the joint kind's doc comment, what goes in and
    what receives it, its cuts ("Hole: 22 mm across, 15 mm deep"), the
    fit, the step ("Step 1 of 1 · push(rung[front], along: length)"), how
    far from home ("rung[front] is 6 mm from home") and the kind's
    `insert:` as written, with its file and line.
  - A **piece of hardware** drives its door or drawer; every member
    follows. The panel gives the kind's doc comment ("How a cup hinge
    works"), its members numbered with their doc comments (and the
    numbers on the drawing), how far it opens, the closest it comes to
    the side and to anything else over its whole travel (exact for
    slides; for swings within 0.05mm: it is refined until no point moves
    more than 0.1mm between samples), the swing check's verdict, and three
    small views (0°, 45°, fully open). The path of the door's front edge
    is drawn whole, in teal: solid where the edge has been, dotted where it
    will go, with dots and tiny angle labels at closed and fully open (the
    panel says what the line is). The closest approach is written beside
    the part it comes closest to ("closest at 110°: 3.5 mm"); its
    dimension line is drawn only in the pose where it is true (within 2°
    or 2mm), from point to point exactly the gap apart, level with the
    hinge where the gap runs along it. In 3D the camera frames the
    whole motion, so the hinge and the mark stay in view all the way. A
    cup hinge is first seen from inside the cabinet, and can be seen
    **from above**: flat (orthographic), cut just above its arm, through
    the cabinet top.
  - A **drawer slide** drives its drawer and can be seen from above, cut
    through the middle of its height and turned so the drawer travels
    across the picture: the cabinet member on the side, the drawer member
    on the drawer side running out. Its closest gap is exact and marked.
  - A **pull** shows how it is fixed: its knob or bar and its screw or
    screws, numbered, on its door or drawer front seen through.
  - A joint whose part **turns upright** (the chidori's lock) opens
    looking down on the node, cut through it, so the lock's flats are
    seen lining up with the slots before the quarter turn.
  - Toggles: X-ray, tint cut faces, fade other parts.
- **Offscreen**: `stave-gtk <folder> --screenshot out.png` with
  `--scene NAME`, `--turn DEGREES`, `--pitch DEGREES`, `--explode
  AMOUNT`, `--diagnostic N` (the Nth problem, from 0, in `stave check`
  order), `--open ASSEMBLY=AMOUNT` (degrees or millimetres), `--hide NAME`,
  `--section AXIS=MM` and `--frame NAME` (close up on an item). A hinge
  from above, half open:
  `stave-gtk examples/clothing_cabinet --screenshot h.png --open door_l=45
  --frame 'door_l.hinges[lower]' --pitch 89 --turn -33.2 --section up=657`.
  `--flat` draws flat; `--no-tint` leaves cut faces untinted.
  See `docs/screenshots/viewer/` and `docs/screenshots/hinge/` (rendered
  with the refitted linkage).
- **Focus offscreen**: `--focus NAME` (a joint or a piece of hardware),
  `--pair N` (which pair of a joint, from 0), `--step AMOUNT` (0 apart to
  1 together, or 0 closed to 1 open; an `--open` of the focused door sets
  it in degrees), `--plan` (a hinge or a slide from above), `--in-3d`
  (keep a joint that opens from above in 3D), `--no-xray`, `--no-fade`;
  the panel's text is printed. For example
  `stave-gtk examples/stool --screenshot s.png --focus rung_sockets --step 0.6`
  and `stave-gtk examples/clothing_cabinet --screenshot h.png --focus
  'door_l.hinges[upper]' --plan --open door_l=45`. See
  `docs/screenshots/focus/`. Posing a focus frame takes a few
  microseconds (6.5µs for the cabinet's hinge, 2µs for the stool's
  socket, in release); opening focus 17ms for the hinge (its closest
  gaps) and 100ms for the chidori (its three steps traced).

- **The real window, without a screen**: `tools/window-shot OUT.png
  FOLDER [options]` runs the viewer on a private GTK Broadway server and
  the window draws itself into the PNG (`--theme`, `--size`, `--select`,
  `--focus` with `--plan`, `--in-3d` and `--step`, `--diagnostic`,
  `--explode`, `--view`, `--editor`, `--finder`, `--xray`, `--checking`).
  See
  `docs/screenshots/window/`.

What the window shows is worked out in `stave-app` (the session, the
tree, the inspector, focus mode, problems, editing) and tested headless;
the window's editing is driven by `--edit-script` steps (`gui.md`).

## Graphical editing

A design can be built and changed in the 3D view ([editing.md](editing.md)
is the specification, [gui.md](gui.md) "Editing in the 3D view" the
window). Every gesture makes an ordinary patch in the `.stave` grammar,
tries it on a scratch copy, shows a ghost with a verdict and writes the
text only when it checks (an error the edit would cause refuses it, with
the checker's fixes on the card).

- **Language**: `locked param` / `solved param` as a prefix; a project's
  root may be `region envelope = box(across:, up:, deep:)`; `split(...,
  shares: [...])`; component params of type `Part`, `Parts`, `Anchor`;
  patches `add`, `set`, `unset`, `remove`, `replace`, `rename`; fixes as
  patches on diagnostics.
- **Libraries**: `std:casework` (Case, Back, Divider, Shelf, HangingRail,
  Feet, Drawer, FramePanelDoor) and `std:frames` (Top, Legs, Apron,
  Stretcher, Cleat, Slats), imported, not in the prelude.
- **Window**: New Project dialog (name, sizes, one wood, libraries,
  tools); a click in a space, in the gap between two parts, or "Add
  something onto its … face" in Details opens a menu drawn from the
  components and kinds in scope, each tried (greyed with the reason when
  it doesn't fit); the card asks for what is still to point at and the
  settings, with a ghost and "It fits" / "It doesn't fit"; Add, Cancel,
  fixes; a selected item's changes (rename, how many, "And the other
  side", hold a distance, remove with what depends on it); count and
  boundary handles; grips on the model (drag, type, stops, sizes that
  follow, "Let the solver move this"); the finder's verbs.
- **Command line**: `stave edit <folder> --patch <file> [--commit]` and
  `stave edit <folder> --script <file>` (a whole build as steps).

**Accepted in the window**: the clothing cabinet and the side table were
built from empty projects by the window's scripted input (the same
commands the pointer, the card and the keys send to the real window); the
text each run wrote is byte-identical to the pinned
`tests/edit-scripts/clothing_cabinet.stave` and `side_table.stave`. A
picture before and after each Add is in
`docs/screenshots/window/accept_*.png`.

### How to try it

```
cargo build -p stave-cli -p stave-gtk
mkdir -p ~/stave-try
target/debug/stave-gtk ~/stave-try --new-project      # an empty folder, the New Project dialog open
target/debug/stave view examples/side_table_from_box  # a finished piece: click a part, its card has its changes

# The two pieces by script, from nothing (the session, no window):
target/debug/stave edit /tmp/cabinet --script tests/edit-scripts/clothing_cabinet.txt
target/debug/stave edit /tmp/table   --script tests/edit-scripts/side_table.txt

# The same two in the real window, a picture a stage (about six minutes):
tools/window-edits/acceptance/run
```

In the window: Create writes the project and lights the box; click
inside it, choose Case (or Top), pick the wood on the card, Add. Then
click in the spaces and gaps the parts leave. The dialog takes one wood:
type the other stock lines into the text (Ctrl+E opens the editor), as
`tests/edit-scripts/clothing_cabinet.stave` has them. After a change made
from a selected item (mirror, a grip, hold) press Ctrl+S: with its file
open in the editor the change stays unsaved there.

### What is not right yet

- The New Project dialog takes one wood; the cabinet needs six stocks,
  the table two. Its fields have no script step, so the acceptance made
  the projects with `stave edit --script` (the same writer).
- The front apron's end (the cleat's `start`) is inside the leg and
  cannot be clicked from any side.
- Changes made from a selected item go into the open editor buffer
  unsaved; menu Adds with no file open are written at once.
- A mirror is offered again for every item that has one (`door_left`
  offers `door_2_right`, `apron_front` offers `apron_2_back`).
- "And the other side · splits its space across into a mirrored pair"
  shows on every single piece that fills a space (the Top, the Case, the
  Divider, the Shelf), not only where a pair makes sense (a door).
- Grip labels overlap each other and are drawn over the cards; the typed
  grip's panel covers the change card's title; the Details panel's
  "CHANGE …" heading overlaps the row above it.
- Library place params prompt "Point at the under." (they have `//`
  comments, not `///`); the card shows raw param names.
- A mirrored pair split up or deep has no gesture.
- The full list, with what was decided and left out, is in
  `docs/workpacks/2026-11-18-graphical-editing.md`.

## Not done yet

- **Window**: the Cut list view is a placeholder with its slot ready
  (`docs/gui.md`). A problem's ways to fix it are its help line split at
  "or", and "Show in text" goes to the problem's own line, not to each
  fix's.
- **Build plan and cut list**: `stave build`, the work plan (recipes,
  order of operations, tool settings), the PDF and the CSV cut list
  (stave-workplan, stave-output) are not written yet; neither is layer 7's
  buildability check (`order` for cuts, `no-recipe`).
- **Boards**: boards are of equal width. The cuts other joints make on a
  part made of boards (a dado across a wide side) stay on the part; they
  are assigned to its boards when the work plan exists. On a part with
  `grain: width`, an edge joint that cuts a tongue is refused (the seam is
  across the part's length there). Board joints that `uses:` fasteners
  don't list them (`not-supported`), and holes, round tenons and round
  sections are not made on boards (`not-supported`). `suitsuki_zan` cuts
  each batten's tongue on the batten's reference face, which faces the
  panel when the panel lies flat with its reference face up; under a
  standing panel the tongue can be on the wrong face (the overlap check
  reports it). The text dump doesn't print boards yet (CLI lane).
- **Ranges**: `require` and the layer 3 and 4 checks hold only at the
  current values; checking over a param's whole `within` range (z3,
  `require-range`, `range`) is not built.
- **Geometry**: cuts on round and turned parts other than holes and round
  tenons are drawn uncut (`not-supported`). The overlap checks keep the
  chidori's lock's round section without its flats (more wood than there
  is); assembly sequences use the flats. Where an angled cut is involved,
  overlaps and straight pushes are worked out in floats from exact inputs
  (well under a nanometre against the 0.05mm tolerance); motion sweeps and
  rods leave angled cuts in (more wood than there is). Turned and rotated
  steps are checked by dense sampling (1mm, 0.4mm of travel), not exactly.
- **Motion**: an assembly inside another moving assembly is not swept; the
  rule that an assembly's hinges share one pivot (`motion`) is not checked.
  A cup hinge moves on the mechanism its kind declares; `std` has the 110°
  full-overlay `euro_hinge` and a 165° `wide_hinge_165` with placeholder
  pins; half-overlay and inset hinges need their own kinds, not written
  yet (an opening over 110° is out of `euro_hinge`'s range). The hinge stops at 110° on its
  limiter (steel on steel, not modelled as a shape); nothing models the soft-close damper or the
  spring.
- **Hardware models**: a kind's `.glb` is read only to check it against the text
  (`model-mismatch`: named nodes to 0.01mm, and an optional `sha256:` pin); it
  is not drawn yet, and a model in a standard module or at a remote source
  is not read.
- **Focus mode**: the PDF work cards that will reuse flat views don't
  exist yet. Ghosts and see-through parts are sorted back to front by
  part, not by triangle. A joint's view from above exists only when one
  of its parts turns about the up axis.
- **Modules**: remote sources (`github:`, `https:`) and the lockfile.
- **Editors and AI**: the language server (`stave-lsp`, on `stave-ide`),
  completion, signature help, quick fixes in the text editor and inlay
  hints. (Patches, rename and smart edits exist for the 3D editor and
  `stave edit`; see "Graphical editing".) The navigation layer rebuilds its
  index on each pause in typing rather than per file (a few milliseconds).
