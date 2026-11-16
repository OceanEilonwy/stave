# Build push: first working v1 slice

Goal for this push: a user can run, on the clothing cabinet example,

```
stave check examples/clothing_cabinet          # compiler-style diagnostics, exit 0 when clean
stave build examples/clothing_cabinet -o build # writes build/clothing_cabinet.pdf (+ cut list CSV)
stave view  examples/clothing_cabinet          # GTK window with the cabinet in 3D
```

and get a correct, useful result. The full v1 plan (12 phases) is in the
project proposal; this push builds one end-to-end path through all of them,
properly typed and tested, with the simplifications listed below.

## Stages of the pipeline (types are the contract)

```
.stave files ──stave-lang──▶ syntax tree (ast)
             ──stave-modules─▶ module set (files per folder, imports, std, prelude)
             ──stave-model──▶ resolved model (ids, types checked, components expanded)
             ──stave-solve──▶ SolvedDesign  (exact sizes + poses for every part/hardware,
                                            resolved cuts per part, requires checked)
             ──stave-geom───▶ meshes per part, cuts applied step by step, collisions
             ──stave-check──▶ diagnostics (all layers available in this push)
             ──stave-workplan▶ WorkPlan (recipe per cut, ordered steps, tool settings)
             ──stave-scene──▶ SceneGraph (render-ready meshes, materials, camera) and
                              vector line drawings for the PDF
             ──stave-output─▶ PDF (Typst) + CSV
             ──stave-app────▶ headless application: open project, run pipeline, commands
             ──stave-cli / stave-gtk
```

`SolvedDesign` (in `stave-model`, module `solved`) is the central contract for
everything downstream of `stave-solve`. It holds only resolved, typed data: ids,
`Length`s, poses (exact positions and an `Orientation`), stock, grain, shape,
the cuts each part receives (operation + part-local measurements), joints with
their roles, hardware with envelopes and poses, regions, scenes. It never holds
names that still need resolving. `stave-model` also provides a hand-built
fixture, `fixtures::clothing_cabinet()`, returning the cabinet's
`SolvedDesign`, so downstream crates can be built and tested before the
front end exists. When the real front end lands, an integration test checks that
solving `examples/clothing_cabinet` matches the fixture's part list and sizes.

## Simplifications for this push (do these, and record any others in the crate docs)

- **Modules:** local folders and `std:` (embedded from `std/`) only. Remote
  sources (`github:`, `https:`) parse, and resolving them is a clear
  "not supported yet" diagnostic. No lockfile yet.
- **Geometry:** parts are boards (boxes) or turned profiles. Axis-aligned cut
  operations (`notch`, `slot`, `pocket`, `tenon`, `drill`) must be exact and
  correct. Angled operations (`dovetail_*`, `outline`) may produce a
  "not supported yet" warning and leave the part uncut in the mesh, without
  blocking the build. Collision checks use boxes and envelopes.
- **Motion sweeps:** hinge and slide range checks may be simplified to the two
  end positions (closed, fully open) in this push.
- **z3:** `require` checks and range checks for linear expressions. Smart-edit
  inverse solving is out of scope for this push.
- **Work plan:** recipe choice by project `tools:`, phases, the hard ordering
  rules (mark before cut, reference faces survive, `cut_first`, `needs`), and
  grouping by tool setting. The scoring may be a deterministic greedy order.
- **PDF:** cover, cut list (rounded to `resolution:`), parts and hardware lists,
  a tool-setting sheet, one work card per part (steps with an isometric line
  drawing of the part and a dimensioned flat view of the face being cut, marks
  and hatched waste), and an assembly order section. No 1:1 templates yet.
- **Viewer:** a libadwaita window rendering the cabinet with wgpu (offscreen
  render to a texture shown in a GTK widget is fine), orbit/zoom/pan with the
  mouse, species colours, click to select a part and show its name, size and
  stock. Scenes (closed / all open) via a dropdown. No editing yet.
- **Japanese samples:** must parse and type-check; their angled cuts may be
  "not supported yet" warnings in this push.

## Lanes and ownership (who may edit what)

| Lane | Owns | Notes |
|---|---|---|
| Spec | `docs/grammar.md`, `crates/stave-lang/src/{ast,placement,units,lib}.rs`, `examples/`, `std/` | Grammar draft 3 from `docs/reviews/2026-11-16-language-review-2.md` |
| Contracts | workspace `Cargo.toml`, crate skeletons, `crates/stave-model` (contract types + fixture) | Runs after Spec |
| Parser | `crates/stave-lang` (lexer, parser, diagnostics, formatter) | Lossless syntax tree (every byte kept; `ast` is a typed view over it), recovery per grammar "Unclosed brackets" and "Tooling and editors", string interpolation parts. Must not change `ast.rs` types without updating the spec |
| Geometry | `crates/stave-geom`, `crates/stave-scene` | Builds from the fixture |
| Output | `crates/stave-output` | Builds from the fixture plus a hand-built `WorkPlan` |
| Viewer | `crates/stave-gtk` | Builds from the fixture through `stave-scene` |
| Front end | `crates/stave-modules`, `crates/stave-model` (resolver), `crates/stave-solve`, `crates/stave-schema` | Runs after Parser |
| Integration | `crates/stave-check`, `crates/stave-workplan`, `crates/stave-app`, `crates/stave-cli`, wiring | Runs after Front end |

## Definition of done for this push

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test`
  pass for the whole workspace.
- Every file in `examples/` and `std/` parses; the cabinet checks clean; the
  broken-design corpus (`tests/broken/`) produces its expected diagnostics.
- `stave build examples/clothing_cabinet -o build` writes a PDF whose cut list
  matches the design and has a work card for every part.
- `stave view examples/clothing_cabinet` opens a window showing the cabinet.
- `docs/STATUS.md` says what works, how to try it, and what is not done yet.

# Next push: fit, collision and motion checks (layers 4 and 5), exploded view

Goal: `stave check` catches wood passing through wood, joints whose parts don't
meet, drawers without slide clearance, and doors or drawers that hit something
as they open, with messages that name the part, the place and the angle or
distance, and a concrete help line. The viewer shows the problem and draws
exploded scenes.

## Steps (each delegated, then checked before the next starts)

1. **Cut solids** (`stave-geom`). Each part's solid is its cut box minus its
   cuts. Axis-aligned operations (`notch`, `slot`, `pocket`, `tenon`, dado,
   rabbet, groove) are exact: a box minus axis-aligned boxes is a set of
   disjoint boxes in integer nanometres. `drill`, `round` and turned parts are
   cylinders or prisms. Angled operations (`dovetail_*`, `outline`) keep the
   "not supported yet" warning and are left uncut, and their parts are left
   out of overlap checks with the parts they join (a note says so). The viewer
   draws cut solids (`Extent::Cut`), so tenons, dados and mortises show.
2. **Static overlap and contact** (`stave-check`, layer 4). Broad phase: sort
   and sweep bounding boxes in a fixed order. Narrow phase: volume shared by
   two parts' solids beyond the 0.05mm contact tolerance is `error[overlap]`,
   with the place and depth ("the tenon on apron[front] goes 3mm past the
   bottom of its mortise in leg[0]"). Hardware envelopes count (a knob in a
   neighbouring door). Every joint's parts must touch (`error[no-contact]`).
   Every scene's poses are checked too. `pitch`/`gap` running past `to` lands
   here as well.
3. **Slides** (layer 4 and 5). `error[slide-gap]` is a measurement. A slide's
   motion is one-dimensional: project the moving assembly and every fixed part
   onto the slide direction; the first contact is a subtraction, exact.
4. **Hinges** (layer 5). Every hinge axis is parallel to a part axis, so the
   sweep reduces to 2D: keep the fixed parts that overlap the moving assembly
   along the axis, project onto the plane across it, and find the first
   contact angle in closed form (each moving corner's circle against each fixed
   edge, each fixed corner against each moving edge). Floats only here, in
   `stave-geom`, converted once. Each motion is checked from rest and from
   each scene's poses. Message: "door_l hits side_l when opened past 87°; the
   hinge opens to 110°", help with a concrete change.
5. **Assembly sequences**. A joint's `insert` steps, run in reverse from the
   finished joint, must be clear: `push` uses the slide sweep, `rotate` the
   hinge sweep. The chidori's quarter turn and slide-out must pass (they are
   proven by `tools/joint-sim/chidori.py`), and a broken variant must fail.
6. **Viewer**. A diagnostic carries the motion and its contact angle or
   distance. Selecting it poses the design at contact and tints the overlap
   red; a slider swings the motion. **Exploded view**: scenes with
   `explode: true` draw parts moved out along their joints' insert directions,
   and the viewer has an explode control (amount 0 to 1, animated) for any
   scene. The logic lives in `stave-scene` and `stave-app`; `stave-gtk` only
   displays it.

## Definition of done

- Broken designs for each new check (`tenon_too_long`, `shelf_misses_dado`,
  `door_hits_side`, `doors_hit_each_other`, `knob_hits_door`,
  `drawer_hits_divider`, `slide_gap_wrong`, `chidori_no_flats`, ...), each with
  one exact diagnostic; the coverage map's rows for `overlap`, `no-contact`,
  `slide-gap` and `motion` move from pending to covered.
- Every example still checks clean, or a real design mistake it reveals is
  fixed in the example and reported.
- Screenshots of cut solids, a highlighted collision and an exploded scene for
  the cabinet and at least two other examples, looked at.
- Checks stay fast: `stave check examples` in release well under a second.
