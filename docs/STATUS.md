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
   Hinge, slide and pull kinds name and document their members
   (`member cup` with a `///` doc comment; grammar, "Hardware members"):
   the standard hinges, slides, knob and bar pull do, and a misnamed member is
   `unknown-field` with the nearest word.
   **Boards**: a `solid` part wider across the grain than its stock's
   `max_width` is made of equal boards (in `--json`), joined by the edge
   joints its `boards:` names (or the project's, or `glued_edge`):
   `tongue_and_groove` widens each board by its tongue, `splined_edge` makes
   a spline per seam, `chigiri` lets a butterfly key into the face across
   each seam, `suitsuki_zan` slides two battens into dovetail housings
   across all the boards, `glue: false` is kept. The collision checks see
   the part's wood as its boards make it, angled cuts included. A sheet
   wider than `max_width` is an error, as a part longer than `max_length`
   is.
4. **Fit and geometry**: every part is cut into a solid (its cut box minus
   its cuts: exact boxes for axis-aligned cuts, exact cylinders for holes,
   round tenons and round sections, and exact prisms, a polygon on a face
   cut straight down, for dovetails, `outline`s and the chidori's angled
   slots; an outline that crosses itself is `type-mismatch`). Wood
   passing through wood beyond the 0.05mm contact tolerance is `overlap`,
   with what goes where and by how much ("the tenon on rail goes 3mm past
   the bottom of its mortise in leg"); a joint's parts that don't touch are
   `no-contact`; a spacing that runs past `to` is `overlap`; a slide's gap
   is `slide-gap`; a door overlaying its side by more or less than its
   cup hinges' catalogue allows is `hinge-overlay` (with the `cup_inset`
   or `plate_height` that fits). Hardware envelopes count, except against
   their own parts. A clash between an assembly (a door, a drawer) and something
   outside it is one error at the line that placed the assembly, in the
   assembly's own terms ("make its top overlay 10mm smaller").
5. **Motion**: every scene's poses are collision checked; every door and
   drawer is swept through its whole travel from rest and from every scene
   and reported at the first angle or distance it hits something
   ("door_r.panel hits door_l.panel when door_r is opened past 71.5deg");
   a door that binds as soon as it opens is told the gap that clears it.
   Slides are exact; butt hinges turn about their pin in closed form; cup
   hinges are four-bar linkages (an arm, two links, the boss in the cup:
   `docs/hardware/cup-hinge-linkage.md`), so the door turns and moves
   forward and out at once, swept by conservative advancement and
   bisection to within the contact tolerance, and the hinges' own arms and
   links are checked against the door and the side.
6. **Requires** at the current values (over whole ranges: not yet).
7. **Assembly sequences**: every joint's `insert` steps are run backwards
   from the finished joint and swept; a step that can't be undone is
   `order`. The chidori's quarter turn and diagonal slide come apart as
   `tools/joint-sim/chidori.py` proves, and fail without its flats.

**Formatting** (`stave fmt`), **dumping** the solved design (`stave dump`,
`--json`), **glTF export** (`stave export`, posed in any scene, exploded or
not), and the **viewer** (`stave view`).

## How to try it

```
cargo build --release -p stave-cli -p stave-gtk
target/release/stave check examples                      # 0 errors; 2 warnings for cuts not drawn yet
target/release/stave check tests/broken/door_hits_side   # one broken design per diagnostic
target/release/stave dump examples/clothing_cabinet
target/release/stave export examples/clothing_cabinet --gltf apart.glb --explode 1
target/release/stave view examples/clothing_cabinet
```

`tests/broken/` holds a small design for every diagnostic, each with the
exact message it must produce on its first line.

## The viewer

`stave view <folder>` opens a libadwaita window: every part drawn with
its cuts made (tenons, mortises, dados, rabbets, grooves, holes, hinge
recesses, the chidori's diagonal slots and flats), species colours, orbit,
zoom and pan, click to select a part or piece of hardware, and the
project's scenes in a dropdown. It reloads when a file changes.

- **Problems**: the side panel lists every diagnostic. Clicking one the
  checks can show poses the design where it happens (its scene, and a
  door or drawer stopped just past where it hits), tints the parts red
  and draws the shared wood as translucent red boxes. For a motion, a
  slider swings it from closed to fully open.
- **Exploded views**: any scene can be pulled apart with a slider (0 to
  1) or the "Pull apart" toggle; scenes with `explode: true` start apart.
  The first half of the range moves each assembly out whole (drawers
  forward, doors off their hinges), the second pulls parts apart along
  the reverse of their joints' last insert step.
- **Cup hinges** are drawn as their mechanism: the steel cup with its
  rim and boss pins in the door, the plate, arm and arm pins on the side,
  and the two links, which turn about their arm pins as the door opens.
  Selecting any part or piece of hardware of a door or drawer (a hinge
  included) offers an "Open it" slider that opens it, the hinges'
  mechanism moving with it; the side panel describes the linkage.
- **Sections**: a scene's `section:` cuts the view at a plane; the wood
  it cuts is drawn flat (evenly lit).
- **Every cut drawn**: holes in round and turned parts are bored too (the
  stool's sockets), every drilled mesh is watertight (no hairline cracks
  where a hole's opening crosses a facet edge), and every face a cut made (holes, mortises, dados,
  rabbets, grooves, tenon cheeks, round tenons, hinge recesses) is drawn
  darker, so the joinery reads at a glance.
- **Focus mode**: selecting a part offers "See how it goes together" for
  each joint pair it is in; selecting hardware offers "See how it works".
  Everything else fades to a ghost and the item is framed.
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

Not checked here: the interactive window itself (window screenshots time
out in the build environment); everything it draws is checked offscreen,
focus mode included, through the same `stave-app` and `stave-scene` code
the window calls.

## Not done yet

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
  Every cup hinge moves on the one built-in 110° full-overlay linkage:
  half-overlay, inset and wide-angle (155°, 165°) hinges have other
  mechanisms, not modelled (a door that needs one is `hinge-overlay`, an
  opening over 110° `out-of-range`). The hinge stops at 110° on its
  limiter (steel on steel, not modelled as a shape); nothing models the soft-close damper or the
  spring.
- **Hardware models**: `.glb` models are not read (`model-mismatch`).
- **Focus mode**: the PDF work cards that will reuse flat views don't
  exist yet. Ghosts and see-through parts are sorted back to front by
  part, not by triangle. A joint's view from above exists only when one
  of its parts turns about the up axis.
- **Modules**: remote sources (`github:`, `https:`) and the lockfile.
- **Editors and AI**: the language server, smart edits and patches.
