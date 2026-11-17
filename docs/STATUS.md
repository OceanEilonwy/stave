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
4. **Fit and geometry**: every part is cut into a solid (its cut box minus
   its cuts: exact boxes for axis-aligned cuts, exact cylinders for holes,
   round tenons and round sections, the chidori's angled slots). Wood
   passing through wood beyond the 0.05mm contact tolerance is `overlap`,
   with what goes where and by how much ("the tenon on rail goes 3mm past
   the bottom of its mortise in leg"); a joint's parts that don't touch are
   `no-contact`; a spacing that runs past `to` is `overlap`; a slide's gap
   is `slide-gap`. Hardware envelopes count, except against their own
   parts. A clash between an assembly (a door, a drawer) and something
   outside it is one error at the line that placed the assembly, in the
   assembly's own terms ("make its top overlay 10mm smaller").
5. **Motion**: every scene's poses are collision checked; every door and
   drawer is swept through its whole travel from rest and from every scene
   (slides exactly, hinges in closed form) and reported at the first angle
   or distance it hits something ("door.panel hits side_l when door is
   opened past 90.1deg"); a door that binds as soon as it opens is told the
   gap that clears it.
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
  Parts move out along the reverse of their joints' last insert step.
- **Offscreen**: `stave-gtk <folder> --screenshot out.png` with
  `--scene NAME`, `--turn DEGREES`, `--explode AMOUNT` and
  `--diagnostic N` (the Nth problem, from 0, in `stave check` order).
  See `docs/screenshots/viewer/`.

Not checked here: the interactive window itself (window screenshots time
out in the build environment); everything it draws is checked offscreen.

## Not done yet

- **Build plan and cut list**: `stave build`, the work plan (recipes,
  order of operations, tool settings), the PDF and the CSV cut list
  (stave-workplan, stave-output) are not written yet; neither is layer 7's
  buildability check (`order` for cuts, `no-recipe`).
- **Ranges**: `require` and the layer 3 and 4 checks hold only at the
  current values; checking over a param's whole `within` range (z3,
  `require-range`, `range`) is not built.
- **Geometry**: dovetails and `outline` cuts are drawn uncut and their
  joints' fit isn't checked (`not-supported` warnings). The overlap checks
  leave the chidori's angled slots uncut and its lock's round section
  without its flats (more wood than there is); assembly sequences use both.
  Turned and rotated steps are checked by dense sampling (1mm, 0.4mm of
  travel), not exactly.
- **Motion**: an assembly inside another moving assembly is not swept; the
  rule that an assembly's hinges share one pivot (`motion`) is not checked.
- **Scenes**: `section:` is not drawn.
- **Hardware models**: `.glb` models are not read (`model-mismatch`).
- **Modules**: remote sources (`github:`, `https:`) and the lockfile.
- **Editors and AI**: the language server, smart edits and patches.
