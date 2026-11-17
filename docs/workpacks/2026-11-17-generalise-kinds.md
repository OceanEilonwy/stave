# Work pack: generalise kinds (2026-11-17)

How to carry out the design in
`docs/reviews/2026-11-17-special-cases-audit.md` (read it first; finding
numbers such as SO10 and design letters A to K refer to it). Eleven stages.
Each one is green and committable on its own.

## Ground rules

- **Spec first.** Every stage starts with its `docs/grammar.md` change, in
  the same commit as the code that follows it. Code follows the spec.
- **Lane.** Changes are confined to the core crates (`stave-lang`,
  `stave-schema`, `stave-modules`, `stave-model`, `stave-solve`,
  `stave-geom`, `stave-check`, `stave-scene`), `std/`, `examples/`,
  `tests/`, `tools/` (deletions) and `docs/`. `crates/stave-app/src`,
  `stave-gtk` and `stave-cli` belong to other lanes: what they need is
  listed under "Hand-offs" and is not done here.
  `crates/stave-app/tests/` is where the corpus-driven tests of every crate
  already live (`spec_*.rs`, `fit.rs`, `pipeline.rs`); this pack assumes
  the core lane may add and edit test files there. Confirm that with the UI
  lane before stage 0.
- **Add beside, then remove.** `SolvedDesign` is read by three crates this
  pack may not change. So each stage adds the new contract next to the old
  one and keeps the old fields filled, from one module,
  `crates/stave-solve/src/legacy.rs`, which is the only place allowed to
  read a kind's entries by name. Stage 8 deletes the old fields and that
  module, after the hand-offs.
- **Gate before every commit**, all from the worktree:

  ```
  cargo fmt
  CARGO_TARGET_DIR=target/generalise cargo clippy --workspace --all-targets -- -D warnings
  CARGO_TARGET_DIR=target/generalise cargo test --workspace
  target/generalise/debug/stave fmt --check .
  target/generalise/debug/stave check examples     # 0 errors, 0 warnings (since stage 2a)
  ```

- **Commits.** Stage only the paths you own (`git add <paths>`, never
  `-A`). Conventional messages (`feat(lang): ...`, `refactor(solve): ...`).
  End each message with a blank line and then
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (the line this
  pack was given; if the session that commits is told to use another
  attribution line, that one wins). Never rewrite history.
- **Never weaken a test or add an exemption.** A test that changes meaning
  is rewritten and the commit message says what it meant before and what
  it means now. A dump snapshot changes only in a stage that lists the
  change below.
- CLAUDE.md applies throughout: newtypes for ids and quantities, enums for
  closed sets, a type per stage of the pipeline, validating constructors,
  exhaustive `match`, no panics in library code, floats only in
  `stave-geom` and `stave-scene`, `BTreeMap` or sorted `Vec` wherever order
  shows.

## Acceptance for the whole push

1. Every example behaves as it does now: the seven dump snapshots in
   `crates/stave-cli/tests/snapshots/` are byte-identical except for the
   changes each stage lists; `stave check examples` gives 0 errors and 0
   warnings (stage 2a cut the `kanawa_tsugi` outlines and the `ari_tsugi`
   dovetail in `examples/joint_samples`, which were the 2 `not-supported`
   warnings before it).
2. `app/focus::no_cup_hinge_puts_steel_in_wood` passes through the generic
   path (stage 3 says how).
3. Adding a 165° cup hinge needs one new kind in `std/hardware` and no
   Rust (stage 5).
4. The special-case code in "Deleted by the end" is gone, and
   `crates/stave-check/tests/standing_rule.rs` keeps it gone.

## Hand-offs to other lanes

| # | To | What | Needed by |
|---|---|---|---|
| H1 | UI (`stave-app/src/focus.rs`, `stave-gtk`) | `member_anchor` (AP4) anchors on a member's solved bounds from `Hardware::members` instead of matching `MemberName`; the panel lists a kind's members by their own names. `stave-gtk/src/screenshot.rs:575` uses node names: they become `<hardware> <member>` (SN2) | stage 3 if possible, stage 8 at the latest |
| H2 | UI (`stave-app/src/view.rs`, `focus.rs`) | Read `Hardware::members`, `roles` and `entries` instead of `HardwareData`, `fixed`, `mechanism`, `is_butt_hinge()` (AP1, AP2, AP3, AP5, AP6); headings from the base's words and the kind's doc comment | stage 8 |
| H3 | CLI (`stave-cli/src/dump.rs`) | Print hardware from `members`, `entries` and the solved mechanism (CL1). The new text should reproduce today's lines for today's kinds, so the snapshots need no change; where it cannot, the snapshot diff is reviewed with this pack's owner | stage 8 |
| H4 | CLI (later) | `stave hardware import`, `--update`, `preview` (stage 10, proposal only) | after the push |

Until H1 to H3 land, stages 1 to 7 keep the old fields filled, so those
crates compile and behave as now without being touched.

**Status.** H1, H2 and H3 landed with stage 8 (the other lanes had
finished, and stage 8 took them over). `stave-app` reads a hardware's
`members`, `roles`, `own_mechanism` and its assembly's `mechanism`; focus
numbers a kind's members with a body by their own names and doc comments,
anchors each number on that member's nodes, and heads the panel with the
base's words ("How this hinge works") over the kind's doc comment. `stave
dump` prints each member on a part or region by name, the kind's entries
in its own order, and each assembly's motion from its mechanism (a turn
about one pin and a travel along one rail in world terms as before; any
other mechanism as its plane and entries). The slide and butt-hinge lines
and the four plain dumps are unchanged; the cup hinge's and the pulls'
lines changed, as the commit that made the change lists. H4 is still a
proposal.

---

## Stage 0. Safety net: the Python oracles as Rust tests (K) — size M

**Goal.** Pin today's behaviour before anything moves, and remove the
Python.

**Spec.** None. `docs/hardware/cup-hinge-linkage.md`: replace the
references to `tools/hinge-sim/*.py` with the test names.

**Files.** New `crates/stave-app/tests/oracle_hinge.rs`,
`crates/stave-app/tests/oracle_chidori.rs`; `docs/test-coverage.md`;
`docs/STATUS.md` (item 7 names `chidori.py`); the comment at
`crates/stave-geom/src/sequence.rs:19-20`.

**Tests** (each named for what `linkage.py` checks, on small designs built
with `common::open_files`, for doors 16, 18, 19, 20 and 22mm thick and
`cup_inset` 3 to 7mm):

1. `closed_is_exact`
2. `the_links_reach_past_110_degrees_with_a_millimetre_to_spare`
3. `the_overlay_is_the_catalogues_for_every_boring_distance`
4. `the_hinge_edge_swings_out_as_the_catalogue_says` (the reveal table,
   within 0.25mm for 18 to 20mm doors, as the doc records)
5. `the_door_stands_8_6mm_into_the_opening_at_110_degrees`
6. `nothing_passes_through_wood_at_any_side_screw_setting` (settings -2,
   -1, 0, +0.5, +2mm, by `overlay(...)` values that need them)
7. `the_front_edge_moves_forward_and_out` (five angles, the numbers
   `linkage.py` prints)

`fit.py` becomes one `#[ignore]` test, `refit_the_cup_hinge`, in the same
file: the same bounds, feasibility rules and objective, a seeded
differential evolution written out in the test (about 40 lines), printing
the pins. It is a tool, run by hand; it must reproduce today's pins from
today's seed. `chidori.py`: `the_lock_turns_a_quarter_and_slides_out`,
`without_flats_it_can_not_turn`, and one test per free direction the script
reports, on `examples/japanese_joinery/chidori.stave`; drop any that
`fit::a_chidori_without_flats_cannot_come_apart` already proves.

**Delete.** `tools/hinge-sim/linkage.py`, `tools/hinge-sim/fit.py`,
`tools/joint-sim/chidori.py`, and `tools/` if empty.

**Risks.** The Python works in millimetres with numpy; the Rust tests go
through the real pipeline, so a number may differ in the last digit. Assert
with the tolerances the doc states, not equality with the script's output.
Run the scripts once more first and keep their output in the commit
message.

**Done when** the gate is green, the three scripts are gone and the doc
names the tests.

## Stage 1. The mechanism is text; `CUP_HINGE_110` moves to `std/` — size L

**Goal.** `std/hardware/hinges.stave` holds the cup hinge's pins, link
sizes, arm and knuckle, word for word the numbers of
`crates/stave-schema/src/hinges.rs`. Nothing else changes: **all seven dump
snapshots byte-identical**, every diagnostic snapshot identical.

**Spec (first).**
- "Syntactic grammar": `member = "member", IDENT, [ block(member_entry) ]`;
  `member_entry = "shape", [ IDENT ], ":", call | field`; `mechanism =
  "mechanism", block(mechanism_entry)`; `pivot`, `slot`, `rail`, `couple`,
  `drive`, `stop`. `mechanism`, `shape`, `pivot`, `slot`, `rail`, `couple`
  join the contextual words.
- New subsection "The kind's frame" beside "Faces to cut": `along`,
  `across`, `depth`; "measurements within the hardware"; `point(...)` is
  `error[syntax]` outside a kind.
- New subsections "Shapes" (B) and "Mechanisms" (C), with the version 1
  limit: planar, one freedom, and in this stage only the form "a driven
  member hung on two links from a held member"; any other is
  `error[not-supported]`.
- "Hardware members": the fixed table is marked as "being replaced" (a
  member with a body may have any name); 752-761 rewritten: a cup hinge
  moves on the mechanism its kind declares.
- "Built-in functions": `point`, `bar`, `prism`; `box` and `cylinder` take
  the frame's ranges inside a member.

**Files.** `stave-lang`: `syntax.rs`, `parser.rs`, `lower.rs`, `ast.rs`,
`format.rs`, `lib.rs`. `stave-schema`: `functions.rs`; delete `hinges.rs`.
`stave-model`: `defs.rs`, `resolve/kinds.rs`, `resolve/expr.rs`.
`stave-solve`: `hardware.rs`, new `mechanism.rs`. `stave-geom`: tests in
`linkage.rs` and `sweep.rs` that import `CUP_HINGE_110` build their linkage
from a literal instead. `std/hardware/hinges.stave`.

**In `std/hardware/hinges.stave`.** `euro_hinge` gains `member arm { shape
body: ...; shape knuckle: ... }`, `member inner_link { ... }`, `member
outer_link { ... }` and the `mechanism { }` block of the audit's draft. Its
four bare members (`plate`, `arm`, `links`, `cup`) stay as the viewer's
list until H1: the resolver keeps `KindDef::members` (the old enum list)
and adds `KindDef::bodies`. Every other entry is unchanged.

**New types.**
- `stave_lang::ast`: `MemberBody`, `ShapeEntry`, `MechanismBlock`,
  `MechanismEntry::{Pivot, Slot, Rail, Couple, Drive, Stop}`.
- `stave_model::defs`: `BodyId(u32)`, `BodyName` (a validated name),
  `BodyDef { name, with: Carrier, shapes: NonEmpty<ShapeDef> }`,
  `Carrier::{Role(RoleIndex), Body(BodyId), Mechanism}`, `ShapeDef::{Box,
  Cylinder, Prism, Bar}`, `KindPoint { along, across, depth: RExpr }`,
  `MechanismDef` with `ConstraintDef::{Pivot, Slot, Rail}`, `CouplingDef`,
  `DriveDef::{Turn, Travel}`, `StopDef`.
- `stave_solve::mechanism`: `Mechanism::new(..) -> Result<Mechanism,
  MechanismError>` holding exact points and a `NonEmpty` list of
  constraints; it can only be built with one freedom and a known step
  order. `MechanismError::{Freedom { found }, NoOrder { left: Vec<BodyId>
  }, NotSupported, DeadPoint, ..}` (`thiserror`).

**Delete.** `stave_schema::hinges` (`CupPoint`, `CupLink`, `CupMechanism`,
`CUP_HINGE_110`); in `stave-solve/src/hardware.rs` every read of it
(`837-853`, `976-988`, `1235`, `1248`, `1290-1294`) and the 110° cap at
`989-1002`: the range is the kind's param, the limit its `stop`.

**Tests.**
- `lang/parser`: members with bodies, shapes, each mechanism entry;
  round-trip through `stave fmt`.
- `app/spec_declarations`: `point_is_an_error_outside_a_kind`,
  `a_mechanism_needs_exactly_one_freedom`,
  `a_pivot_names_two_declared_members`.
- New broken design `tests/broken/mechanism_not_one_freedom`.
- The stage 0 oracle tests pass unchanged: they now read the kind.
- `a_cup_hinge_opens_at_most_110_degrees` (`fit.rs:139`) is rewritten: its
  `wide_hinge` copies `euro_hinge`'s mechanism with `opening_angle:
  125deg`; the error is `motion`, "links lock" (the pins lock near 130°
  less the margin; if 125° clears, use 135°). Old meaning: a built-in cap.
  New meaning: the declared mechanism's own limit.
- `tests/broken/misnamed_member` keeps passing (bare members are still
  legal in this stage).

**Risks.** Byte identity depends on rounding in the same place: the pins
go through `ExactLength` and are rounded where `cup_linkage` rounds today
(`to_nearest_length`). Compare the cabinet's dump before touching
anything else. A hinge kind with cup entries and no `mechanism` gets no
motion and `warning[not-supported]`, so third-party kinds fail loudly.

**Done when** the gate is green with zero snapshot changes and `rg
CUP_HINGE_110 crates` finds nothing.

## Stage 2. Roles, mounts and cuts in hardware kinds (F, G) — size L

**Goal.** A hardware kind says which parts it joins, where it sits by
default and what it cuts. The base decides none of it.

**Spec (first).** Rewrite 731-774 (one table: what any hardware kind may
contain; `fastener` and `tool` rows stay), "Hardware placement" 907-972
(the `mount { }` block, the four selectors, `open`), "Hardware" 1346-1375
(no "what each base does" list: the standard kinds are shown instead).
"Faces to cut": `crossing(p, member)`. "Roles and pairs": hardware roles,
`Region` roles. Named cuts (gap 9): `cut dado: crossing(into, piece) {
... }`. `uses:` allowed in hardware kinds.

**Files.** `stave-lang` (parser, ast: `roles`, `cut`, `mount`, `uses` in
any kind; delete `placement.rs:719-814`, L3). `stave-schema/src/kinds.rs`.
`stave-model` (`resolve/kinds.rs`, `defs.rs`, `solved.rs`: `CutPlace`).
`stave-solve` (`hardware.rs`, `build.rs`, `eval.rs`, `joints.rs`, new
`legacy.rs`). `stave-geom/src/solid.rs`. `stave-check/src/explain.rs`.
`std/hardware/*.stave`, `std/joinery/{dado,rabbet,groove,socket}.stave`
(named cuts), `examples/clothing_cabinet/hardware/handles.stave`.

**New types.** `MountDef { face: FaceRule, across: AxisRule, along:
AxisRule }`, `AxisRule::{Open, Term(MountTerm)}`, `Selector::{EdgeNearer,
EdgeFlushWith, FaceToward, FaceNearer, Crossing}`; `solved::CutPlace::Member
{ hardware, body, footprint }`; `solved::Hardware::roles: Vec<(RoleName,
RoleValue)>` and `entries: BTreeMap<EntryName, EntryValue>` (new, beside
`data`); `CutName`.

**Delete.** `MountRule`, `OpenAxes`, `BaseSpec::{mount, open_axes}`,
`SLIDE`/`HINGE`/`PULL` role and entry tables, `CUP_HINGE_ENTRIES`,
`BUTT_HINGE_ENTRIES`, `DEFAULT_SCREW_DIAMETER_NM` (SC2 to SC4, SC6, SC7);
in the resolver the cup-or-butt test and its three errors (M1); in
`stave-solve`: `HingeStyle`, `Bore`, `Recess`, `hinge_plate`, `butt_hinge`,
`HingeFrame`, the per-base arms at `453-509` and `522-526`, the pull
default at `eval.rs:751-753`, `bore_cut`, `recess_cut` (SO3 to SO7, SO9 to
SO13); `CutPlace::{Recess, Mount}` (M10); `cavity_noun`'s word search
(CH13).

**Kept filled by `legacy.rs`:** `HardwareData`, `Hardware::{envelope,
fixed}`.

**Dump changes to explain.** Hardware cuts print as `crossing` cuts instead
of `recess`/`mount` lines unless H3 lands first; list the changed lines in
the commit.

**Tests.** `app/spec_joints`: `a_cup_hinge_bores_its_cup...`,
`a_pull_bores_one_hole_or_two...`, `butt_hinge_leaves_are_housed...` pass
unchanged in what they assert (the cut's size and place). New:
`a_hardware_kind_declares_its_roles`, `mount_axes_left_open_are_set_by_place`,
`edge_flush_with_picks_the_flush_edge`, `face_nearer_works_on_any_axis` (a
drawer that pulls out sideways), `a_named_cut_is_called_by_its_name`.
Broken: `tests/broken/butt_hinge_meets_nothing` keeps its exact first line
(the kind's `require` carries the message).

**Risks.** The butt hinge's `no-contact` wording is pinned by a broken
design; keep it as the kind's `require` message. Slides gain a real
direction: check `drawer_hits_divider` and `out_of_range` still give their
exact lines.

**Done when** the gate is green and `rg '"(cup_|plate|leaf|centres|screw_)'
crates/stave-{solve,model,check,geom}/src` finds only `legacy.rs`.

## Stage 2a. Angled cuts: dovetails, outlines and angled slots — size L

**Goal.** `dovetail_socket`, `dovetail_tail`, `outline` and a `slot`
turned by an angle are cut for real, drawn, and checked like every other
cut. Today they are left uncut (`Solid::skipped`) and the parts they join
are left out of the overlap check (`stave-check/src/lib.rs:673-677`,
audit CH8). That is an exemption, so it goes before stage 3 builds the one
collision model, and before stage 7 needs it for `chigiri` and
`suitsuki_zan`. Joint kinds are already clean (audit: no core crate names
a joint kind), so this stage adds one general solid primitive and no
joint-specific code.

**One primitive: the prism.** Every one of these cuts is a flat shape
marked on a face and cut straight down from it, so each is a polygon in
the face's plane, extruded along the face's axis between two depths:

- `outline`: the marks, as written (non-convex allowed).
- `dovetail_socket`: the trapezoid, `neck` wide at the end, widening by
  `depth * tan(angle)` each side, cut through the thickness.
- `dovetail_tail`: the cut box minus the two wedge-shaped prisms either
  side of the tail.
- an angled `slot`: its rectangle turned in the face (replaces
  `AngledSlot`; the chidori's slots are then cut, which removes one of
  STATUS.md's "more wood than there is" simplifications).

**Spec (first).** "Cut operations" (grammar.md ~1275-1285): say how each
of the three is shaped, exactly enough to draw it, including which way a
dovetail's angle widens and that an `outline`'s marks must not cross
themselves (`error[type-mismatch]`, "the outline crosses itself between
mark 3 and mark 4"). Remove the "geometry can't make yet" sentence
(~1239) for these operations; `not-supported` stays for cuts on round
or turned parts. Optional (decision table): accept a slope as a ratio,
`slope: 1 in 8`, the way dovetail markers are sold, as an exact
alternative to `angle:`.

**Exactness.** Polygon vertices are exact `Local` points in nanometres.
`outline` marks are exact already. A dovetail's offset `depth *
tan(angle)` is irrational for most angles; it is computed once, from the
exact `Angle` and `Length`, and rounded to the nanometre when the prism is
built (the same rule as a hole's position, solid.rs module docs). A
`slope: 1 in n` is exact, with no rounding. Rotated slot corners round
the same way.

**Files.** `stave-geom/src/solid.rs` (the prism, the shapes above, delete
`Unsupported::{Angled, AngledSlot}` and `AngledSlot`),
`stave-geom/src/collide.rs` (block against prism, prism against prism),
`stave-geom/src/mesh_cut.rs` (cut prisms out of the mesh),
`stave-geom/src/sequence.rs` (assembly sequences test prisms instead of
`AngledSlot` in floats), `stave-geom/src/sweep.rs` (prism pieces are
convex polygons for the planar sweep), `stave-check/src/lib.rs` (delete
the can't-cut exemption), `docs/grammar.md`, `docs/STATUS.md`.

**New types.** `Polygon` (private fields, `Polygon::new(Vec<Point2>) ->
Result<Polygon, PolygonError>`: at least three points, no repeated point,
does not cross itself, positive area, wound counter-clockwise after
normalising); `ConvexPolygon` (the same, also convex, for collision
pieces); `Prism { face_axis: PartAxis, outline: Polygon, pieces:
NonEmpty<ConvexPolygon>, depth: DepthRange }` where `DepthRange::new(from,
to)` requires `from < to`; `PolygonError` (thiserror) with one variant
per rule, each mapped to a diagnostic with a help line.

**Collision.** Each prism is split once into convex pieces. The split is
exact and deterministic: ear-clipping triangulation in a fixed vertex
order, then merging neighbouring triangles while the result stays convex
(Hertel-Mehlhorn). Tests:
- Two shapes extruded along the same axis (the common case: both cut from
  the same face, or a box) reduce to 2D: convex polygon clipping in exact
  integers times the overlap of their depth ranges.
- Otherwise, a separating-axis test on the two convex solids with exact
  `i128` arithmetic and checked operations. A 4 m part is about 2^42 nm,
  so the triple products stay within `i128`; an overflow is an error, not
  a panic.
- The overlap's volume and the message's numbers convert to floats at the
  boundary only, as now.

**Mesh.** Walls from the polygon's edges, caps from the same triangulation,
subtracted from the part's mesh with the existing cut code. The mesh must
be watertight (the test added in `fe8b2d2` covers every example).

**Delete.** `AngledSlot`, `Unsupported::Angled`, `Unsupported::AngledSlot`,
the can't-cut branch of `exempt` (`stave-check/src/lib.rs:673-677`), the
float-only angled-slot test in `sequence.rs`.

**Dump changes to explain.** `joint_samples`: the `splice`
(`kanawa_tsugi`) and `end_dovetail` (`ari_tsugi`) parts lose their
`skipped` entries and gain their cut solids. The chidori's lock and
slots get cut solids. If a real overlap shows up in an example (the
check could not see one before), fix the kind's numbers in the same
commit and say what was wrong. Never add an exemption.

**Gate change.** `stave check examples` becomes 0 errors and 0 warnings.
Update the gate line and acceptance item 1 in this stage's commit.

**Tests.**
- Unit (`stave-geom`): `a_polygon_that_crosses_itself_is_rejected`,
  `an_l_shaped_outline_splits_into_two_convex_pieces`,
  `convex_split_is_the_same_every_run`,
  `a_dovetail_tail_fills_its_socket_exactly` (tail and socket from one
  joint overlap by zero volume and touch over their sloped faces),
  `a_tail_one_degree_wider_overlaps_its_socket`,
  `a_slope_1_in_8_is_exact`, `an_angled_slot_is_cut`.
- Corpus: `examples/joint_samples` checks with no warnings; new broken
  designs `tests/broken/dovetail_too_wide` (a tail angle that doesn't match
  its socket: `error[overlap]` with the sloped faces named) and
  `tests/broken/outline_crosses_itself`.
- Mesh: the watertight test passes on `kanawa_tsugi` and `ari_tsugi`.

**Risks.** Real overlaps the old exemption hid may surface in
`kanawa_tsugi` or the chidori; they are fixed in the kind, with the dump
change explained. Separating-axis tests near zero volume (faces that only
touch) must count touching as contact, not overlap. Exact integers make
that a clean comparison with zero, so no tolerance is involved. Cuts on round
or turned parts stay `not-supported`; nothing in the examples uses one.

**Done when** the gate is green with 0 warnings, `rg
'AngledSlot|Unsupported::Angled' crates` finds nothing, and
`stave-check` has no exemption for an uncut cut.

## Stage 3. One collision model (A, B) — size L

**Goal.** Every member shape is a body. One overlap check, one sweep. The
hinge-only sweep and the own-parts exemption are deleted.

**Spec (first).** Replace 1372-1375 with rule A; delete "may overlap the
hardware's own role parts"; 951-955 and 1630-1633 become one sentence in
"Scenes" about every body; the fastener sentence (1190-1191) gains "and
are the only thing not modelled"; state the rule for a kind's own members
(gap 2) and what a sweep does with cuts. A member without a shape is
`error[missing-field]`.

**Decide before starting** (gaps 1 and 2 of the audit): the rim and pins,
and members of one item against each other. The recommended answers keep
the examples as they are.

**Files.** `stave-geom`: `sweep.rs`, `collide.rs`, `lib.rs`, `hinge.rs`
(deleted). `stave-check`: `lib.rs`, `motion.rs`, `hinges.rs`. `stave-scene/
src/graph.rs`. `stave-model/src/solved.rs`. `std/hardware/*.stave` (every
member gets its shapes). If H1 has landed, bare members are gone and a
member without a shape is an error for every name. If it has not,
`euro_hinge` keeps its bare `member links` for the panel, and a member
without a shape is allowed only when its name is one of the old table's
words; stage 8 deletes that allowance.

**New types.** `solved::Body { id: BodyRef, shapes: NonEmpty<SolvedShape>,
carrier: Carrier }`, `BodyRef { hardware: HardwareId, body: BodyId }`;
`stave_geom::sweep::Mover` (a pose function with a speed bound: `Fixed`,
`Rigid(&Path)`, `Member(&Path, BodyId)`), `sweep::first_contact(a: &Convex,
ma: &Mover, b: &Convex, mb: &Mover, range) -> Option<Reach>`,
`sweep::deepest_overlap(..) -> Option<Depth>` (GE3 made general);
`stave_check::Item::Member(BodyRef)` replaces `Item::{Hardware, Fixed}`
inside the checker (the public `ContactItem` keeps its shape).

**Delete.** `MechanismMember`, `HingePart`, `MechanismHit`, `Mechanism`,
`MEMBERS`, `mechanism_contact`, `SteelInWood`, `steel_in_wood` (GE1 to
GE3); `stave-geom/src/hinge.rs` with `RIM_WIDTH`, `RIM_THICKNESS`, `WALL`
(GE7); `hardware_fixed`, `hardware_links` (GE8); `Checker::mechanisms` and
its call (CH2); the own-parts arm of `exempt`, `lib.rs:663-672` (CH5); the
arm-and-knuckle fold at `lib.rs:238-246` (CH3); `LinkFollower` (SN1) once
nodes follow bodies.

**`no_cup_hinge_puts_steel_in_wood`, re-routed.** Today
(`crates/stave-app/tests/focus.rs:737-797`) it opens the cabinet, the
nightstand and the box, picks hardware with `mechanism` and
`HardwareData::Hinge`, and calls `steel_in_wood` (2000 samples, at most
0.05mm) and `mechanism_contact` (must be `None`). After this stage, same
name, same three examples:
1. for every piece of hardware with a mechanism (no test on its kind or
   base), for every body of it and every part of the design, call
   `sweep::deepest_overlap` over the whole drive range at 2000 samples and
   assert at most `CONTACT_TOLERANCE`;
2. assert that `stave_check::check` on the same design reports no `motion`
   and no `overlap` naming a member.
And the proof that the path can fail: new broken design
`tests/broken/hinge_link_in_wood` (a copy of `euro_hinge` with the outer
boss pin moved 1.5mm toward the door's edge) must give one `error[motion]`
naming `outer_link` and the stile, from `Checker::sweeps`, the same
function that sweeps doors.

**Tests.** Every `fit__*.snap` unchanged except
`fit__a_door_that_swings_into_a_knob` and
`fit__a_knob_in_the_way_of_another_part` if the item's name gains its
member (explain). New: `a_cup_sits_in_its_bore_and_nowhere_else` (a kind
without its `cut` gives `overlap` with its own door),
`a_leaf_without_its_mortise_overlaps`, `swept_wood_has_its_bores`.

**Risks.** This is where hidden steel shows (the audit's first surprise).
Performance: `stave check examples` must stay well under a second in
release; keep the broad phase, and the exact slide and closed-form turn as
fast paths chosen by the motion's shape (GE11). Sweeping wood with holes:
start with round holes whose axis lies in the plane (the cup's bore) and
box cuts; leave the rest whole.

**Done when** the gate is green, the three examples' dumps are identical,
and `rg 'steel_in_wood|mechanism_contact|HingePart' crates` finds nothing.

## Stage 4. The general solver; butt hinges and slides (C, D) — size L

**Goal.** One solved mechanism per assembly, from any kinds. A pin and a
rail are its simplest cases.

**Spec (first).** "Mechanisms": the three step types, the branch rule,
dead points, `error[mechanism]` (new code, layer 3, in the diagnostics
table), the rule for several items on one assembly; "Scenes" 1617-1625
rewritten from "the slides, or the hinges" to "the hardware that has a
drive"; version 1 limits (planar; an assembly inside a moving assembly is
not swept).

**Files.** `stave-lang/src/diagnostics.rs`. `stave-model`: `linkage.rs`
becomes `mechanism.rs` (the exact circle-circle check kept, a circle-line
check added), `solved.rs`. `stave-solve`: `mechanism.rs`, `hardware.rs`,
`build.rs`. `stave-geom`: `linkage.rs` becomes `mechanism.rs`,
`sweep.rs`, `collide.rs`, `clearance.rs`. `stave-check/src/motion.rs`.
`stave-scene/src/graph.rs`. `std/hardware/{hinges,slides}.stave`.

**New types.** `solved::Mechanism` (validated: `Mechanism::new` checks one
freedom, an order of steps and the first step's dead points, exactly);
`Step::{TwoLinks, LinkAndSlot, Carried, Rail}`; `Drive::{Turn { range:
AngleRange }, Travel { range: LengthRange }}`; `Stop { at: DriveValue,
message }`; `Branch`; `solved::Motion` becomes a struct holding the
assembly's mechanism and its plane; `stave_geom::mechanism::Path` (floats)
with `pose(body, at)` and `speed_bound`.

**Delete.** `Motion::{Turn, Linkage, Travel}` as variants and `Pivot`
(M8); `stave_model::linkage::{Linkage, Link, LinkageFrame}` as public
types (M9); `MotionExact`, `Data`, `CupFrame`, `CupArmFrame`, `cup_arm`,
`cup_linkage`, the slide arm at `hardware.rs:607-666` with its
`Direction::Front` (SO1, SO2, SO8); `LinkagePath`, `LinkageSweep`,
`linkage_contact`, `linkage_reach` (GE5, GE6); `Sweeper::{Turn, Linkage,
Travel}`, `motion_hardware`, the `Motion::Linkage` test at
`motion.rs:433-437` (CH9 to CH11); "first hardware wins" at
`build.rs:1439-1458` (SO16); the per-variant mirror code (SO14).
`legacy.rs` fills the old `Motion` shape for the dump until H3, and
`stave_geom::clearance::closest_for` keeps its signature (it takes that
`Motion`; `stave-app/src/focus.rs:786`, `794` call it) until H2.

**Butt hinge, two commits.** First: `butt_hinge` as `pivot leaf, fixed_leaf
at point(across: 0mm, depth: 0mm)`, byte-identical dumps. Second (gap 3,
if the user agrees): the pivot moves out to the knuckle; the box's dump
and `lid_hits_back`, `scene_pose_clash` change by the knuckle's radius;
say so.

**Tests.** `model/mechanism`: the existing linkage tests, renamed, plus
`a_slot_is_a_circle_and_a_line`, `a_dead_point_is_found_exactly`,
`the_branch_is_the_closed_pose's`. `app/pipeline`:
`the_cabinet_doors_move_on_their_hinges_linkage` unchanged. New broken
designs: `tests/broken/hinges_move_differently` (a cup hinge and a butt
hinge on one door: `motion`), `tests/broken/mechanism_cannot_be_solved`
(`mechanism`). New corpus design `tests/designs/lid_stay` (two butt hinges
and a stay with a slot: a follower; opens to the stay's stop). Slides:
`a_full_extension_slide_has_a_middle_member` (a `couple`).

**Risks.** The exact check today covers one circle pair; later steps are
checked in floats with a bounded step. State the margin in the spec. The
old `Motion` is matched in `stave-app` and `stave-cli`: the legacy fill
must be exact for today's kinds (the dump prints the pins).

**Done when** the gate is green; dumps identical but for the listed butt
hinge change; `rg 'Motion::(Turn|Linkage|Travel)' crates/stave-{solve,
geom,check,scene}/src` finds only `legacy.rs`.

## Stage 5. Proof: a 165° hinge is one new kind — size S

**Goal.** Show that a new mechanism needs no Rust.

**Files.** `std/hardware/hinges.stave` (one kind), one corpus design
`tests/designs/wide_hinge/project.stave`, one test. `git diff --stat` for
the stage must show nothing under `crates/*/src`.

**Draft kind.** The pins below are **placeholders**: they must be fitted
to a maker's catalogue with `refit_the_cup_hinge` (stage 0) before the
kind ships, and the kind's doc comment must name that catalogue.

```
/// A wide-angle concealed cup hinge: the door opens to 165°, clear of
/// pull-out shelves. Mounted like euro_hinge. PLACEHOLDER PINS.
kind wide_hinge_165 : hinge {
  roles {
    piece: Part
    on:    Part
  }
  param opening_angle = 165deg within 90deg..165deg
  param cup_inset     = 5mm within [3mm, 4mm, 5mm, 6mm]
  param plate_setback = 37mm within [37mm]
  param plate_height  = 0mm within [0mm, 3mm]
  cup_diameter: 35mm
  cup_depth:    11.7mm

  mount {
    face:   back
    across: from(edge(piece, nearer: on), inset: cup_inset + cup_diameter / 2)
    along:  open
  }

  member cup   { with: piece  shape: cylinder(diameter: cup_diameter, depth: 0mm..cup_depth) }
  member plate { with: on     shape: box(...)  mount { ... } }      // as euro_hinge
  member arm   { with: plate  shape body: box(...)  shape knuckle: box(...) }
  member inner_link { shape: bar(from: point(across: 14.00mm, depth: 9.50mm),
                                 to:   point(across: -7.00mm, depth: 6.00mm), thickness: 1.50mm, along: 9mm) }
  member outer_link { shape near: bar(...)  shape far: bar(...) }

  mechanism {
    pivot arm, inner_link at point(across: 14.00mm, depth: 9.50mm)    // placeholder
    pivot cup, inner_link at point(across: -7.00mm, depth: 6.00mm)    // placeholder
    pivot arm, outer_link at point(across: 12.00mm, depth: 2.50mm)    // placeholder
    pivot cup, outer_link at point(across: -10.00mm, depth: 1.00mm)   // placeholder
    drive: turn(cup, from: 0deg, to: opening_angle)
    stop:  opening_angle "the arm's limiter stops the door here"
  }

  cut crossing(piece, cup) { drill(diameter: cup_diameter, depth: cup_depth) }
  adjust arm on plate: across within 2mm
}
```

**Test.** `app/fit::a_165_degree_hinge_is_only_a_kind`: the corpus design
checks clean, its door's `open` pose is 165°, the stage 3 depth test finds
no steel in wood, and a pose of 170° is refused with the kind's `stop`
message.

**Risks.** A four-bar may not reach 165° without steel in wood or a dead
point; real ones have more links. If the fit fails, write the kind with
the links it needs. If that mechanism has no order of steps from
`turn(cup)`, that is gap 4: add the step type to the spec and the solver
in a stage of its own (size M), not a special case here.

**Done when** the test passes and the stage's diff touches no `src/`.

## Stage 6. `adjust`, `require` and the help search (H) — size M

**Goal.** Catalogue rules live in kinds. `hinge-overlay` and `slide-gap`
are deleted.

**Spec (first).** New "Adjustment" under "Hardware"; `require` in hardware
kinds with `between(mount, <region>)`; "Checks and `require`": the help
search over `within` lists; diagnostics table: remove `slide-gap` and
`hinge-overlay`, add `adjust` (layer 4). 763-770 deleted.

**Files.** `stave-lang` (`diagnostics.rs`, parser: `adjust`),
`stave-model`, `stave-solve` (`hardware.rs`, `eval.rs`), `stave-check`
(`lib.rs`; delete `hinges.rs`), `std/hardware/{hinges,slides}.stave`,
`tests/broken/{hinge_overlay,drawer_does_not_fit}` (their `// expect:`
lines), `crates/stave-lang/src/lib.rs:583` (the code list).

**New types.** `AdjustDef { body, on, axis: FrameAxis, within: RExpr }`;
`solved::Setting { body: BodyRef, axis, offset: Length }` (the solved rest
setting, in the dump's JSON); `HelpSearch` in `stave-solve` returning
`Vec<(ParamName, Value)>`, sorted.

**Delete.** `Checker::hinge_overlays`, `MAX_INSET_NM`, `MAX_PLATE_NM`
(CH1); `Checker::slide_gaps`, `misfit_slides` and its arm of `exempt`
(CH4, CH6: replaced by "hardware whose `require` or `adjust` failed is
left out of the overlap check"); `Code::{HingeOverlay, SlideGap}` (L4);
`OverlayRange`, `OverlayRangeExact`, the `base_overlay` reads (M6, SO10);
`HINGE_OPTIONAL`'s three overlay entries.

**Diagnostics that change (explain each).**
- `tests/broken/hinge_overlay`: was `error[hinge-overlay] ... overlays
  side_l by 9mm, but the hinge gives 15.5mm to 19.5mm`; becomes
  `error[adjust]` with the arm's offset, and help that names a `cup_inset`
  or `plate_height` from the kind's lists (never 2.5mm).
- `tests/broken/drawer_does_not_fit`: was `error[slide-gap]`; becomes
  `error[require]: 'gap' fails: the slide needs exactly this much beside
  the drawer`, with the measured and the needed gap in a note.
- Snapshots `fit__a_door_too_far_out_for_its_cup_hinges`,
  `fit__a_drawer_too_wide_for_its_slides`,
  `fit__double_doors_with_no_gap...` (its help names `plate_height`).

**Tests.** `the_help_only_offers_values_the_kind_allows`,
`a_design_inside_the_adjustment_checks_at_the_setting_it_needs` (overlay
18mm on a 17.5mm nominal: setting 0.5mm, as the cabinet has today),
`an_axis_without_adjust_must_be_exact`.

**Risks.** The cabinet's doors overlay by 18mm, 0.5mm off nominal: the
solved setting must reproduce today's arm position exactly (dump line
`arm 20.6..38.5`).

**Done when** the gate is green and `rg 'hinge.overlay|slide.gap|HingeOverlay|SlideGap' crates docs/grammar.md` finds nothing.

## Stage 7. Boards and edge joints (I) — size L

**Goal.** A part wider than its stock is made of boards, joined the way
the user chooses, without glue if they wish.

**Spec (first).** `stock`: `max_width` row rewritten; new "Boards" section
under "Sizes" (the split rule, `boards: joined(by: ...)` on a part and on
the project, the default, lists, sheets and length stay errors, the help
for length). "Roles and pairs": a kind with one `Parts` role. "Cut
operations": `tenon` on `crossing` for parts that butt edge to edge. `part`
and `project` field tables: `boards`.

**Files.** `stave-schema/src/{fields,functions}.rs`; `stave-model`
(`defs.rs`, resolver, `solved.rs`); `stave-solve` (`build.rs`, `joints.rs`,
new `boards.rs`); `stave-geom/src/solid.rs` (the edge tongue);
`std/joinery/{glued_edge,tongue_and_groove,splined_edge,chigiri,
suitsuki_zan}.stave`, `std/joinery/library.stave`, `recipes.stave` if an
operation gains a use; `examples/side_table/table.stave` and
`examples/clothing_cabinet` comments ("the cut list plans a glue-up").

**New types.** `BoardIndex(NonZeroU32)`, `solved::Boards { boards:
NonEmpty<Board>, seams: Vec<Seam>, methods: NonEmpty<KindId> }`, `Board {
width: Length, cuts: Vec<Cut> }`, `Seam { between: (BoardIndex,
BoardIndex), at: Length }`; `Part::boards: Option<Boards>` (derived, in the
JSON dump); `Joining { by: NonEmpty<KindValue> }`.

**Dump changes to explain.** Parts wider than `max_width` (the side
table's top, the cabinet's sides, top, base, shelves) gain their boards in
`--json`; the text dump changes only when H3 prints them. The hardware
list gains nothing for `glued_edge`.

**Tests.** `a_wider_part_than_max_width_is_planned_as_a_glue_up` is
rewritten to assert the boards (count, equal widths, seams); new
`boards_are_joined_by_the_projects_default`, `a_part_names_its_own_joining`,
`a_list_combines_edge_joints`, `tongue_and_groove_needs_no_glue`,
`a_sheet_wider_than_max_width_is_an_error`, `a_longer_part_names_an_end_joint`,
`a_spline_is_a_made_part_per_seam`. Corpus `tests/designs/unglued_top`
(tongue and groove with battens). `chigiri` and `suitsuki_zan` are cut for real (they use `outline` and
`dovetail_socket`, made in stage 2a); corpus `tests/designs/chigiri_top`
and `tests/designs/batten_top` check with no warnings.

**Risks.** `stave-workplan` and `stave-output` are not written, so "in the
cut list" means "in `SolvedDesign`" for now. Cuts that other joints make
on a board-made part are assigned to boards when the work plan exists;
record that in STATUS. Equal board widths are a simplification; say so in
the spec.

**Done when** the gate is green and `docs/test-coverage.md:183` reads
"covered".

## Stage 8. Remove the old contract; the standing rule (J) — size M

**Needs** H1, H2 and H3 merged.

**Goal.** Nothing kind-shaped is left in the core, and a test keeps it so.

**Spec and docs.** "Hardware members" section deleted (its content is now
"Mechanisms" and "Shapes"); `CLAUDE.md` gains the text of J (this is the
one edit to `CLAUDE.md`, made with the user's agreement);
`docs/STATUS.md`, `docs/test-coverage.md`, `docs/plan.md` (a "done" note),
`docs/hardware/cup-hinge-linkage.md` (the data is the kind).

**Delete.** `crates/stave-solve/src/legacy.rs`; `HardwareData`,
`Hardware::{envelope, fixed, mechanism, data}`, `CupMechanism`, `LinkBar`,
`HardwareMember`, `is_butt_hinge` (M5, M7); `MemberName`, the four member
tables, `members_of` (SC5); `KindDef::members` and `lower_members` (M2,
M3); the bare `member links` in `euro_hinge`; `Requirement::Conditional`
uses for hinges; `stave-geom`'s dependency on `stave-schema` if nothing
else needs it.

**New test.** `crates/stave-check/tests/standing_rule.rs`,
`the_core_names_no_kind`: reads `std/hardware` and `std/joinery` as text,
collects every `kind` name, every `member` name, and every field or param
name of a hardware kind; adds a short fixed list (`cup`, `butt`, `euro`,
`leaf`, `knuckle`, `linkage`); subtracts an allow-list written in the test
with a reason beside each word (the bases `hinge`, `slide`, `pull`,
`fastener`, `tool`; cut operations such as `tenon` and `slot`; `dowel` the
stock form; `length`, `diameter`, `travel` and other general words). Then
reads every `src/**/*.rs` of the nine core crates, skips comments and
everything after a `#[cfg(test)]` line, and fails with file, line and word
for each whole-word hit in code or in a string. The other half of the
rule, that no check lets a pairing off, is held by behaviour, not by
reading source: stage 3's `a_cup_sits_in_its_bore_and_nowhere_else` and
`tests/broken/hinge_link_in_wood`.

**Risks.** The allow-list is where special cases could creep back; every
entry needs its reason, and review 3 should read the list.

**Done when** the gate is green and the test passes.

**Status: done.** Everything listed above and under "Deleted by the end"
is gone (`Pivot` lives on as `stave_model::mechanism::TurnLine`, the line
of the new `MotionShape::Turn`; `stave-geom` still needs `stave-schema`
for `Grain`). The test scans eight crates (`stave-workplan` doesn't exist
yet) and also reads each hardware kind's fields; its allow-list is
`length`, `diameter`, `travel`, `bar`, `dowel`, `envelope`, `socket`,
`centres`, `projection` and `clearance`, each with its reason, and an
entry that is no longer a word of `std/` fails it. `butt` is left off the
fixed list (it is `Contact::Butt`, the joint contact; `butt_hinge` is
still a hit). Making it pass removed the last special cases: help lines
that named `dado`, `cup`, `plate`, `ari_tsugi`, `glued_edge`,
`tongue_and_groove` and `opening_angle`, and the built-in default edge
joint (`DEFAULT_BOARD_JOINT = "glued_edge"`), which is now
`std:joinery`'s `library { boards: joined(by: glued_edge) }` (grammar,
"Boards").

## Stage 9. Models checked against the text (E) — size M

**Goal.** `warning[model-mismatch]` when a `.glb` and its kind disagree.

**Spec (first).** 1377-1387 rewritten: names of nodes (a pivot's node is
named for its two members, a shape's for its member and shape), 0.01mm,
the help text about metres, +Y up and unapplied scale, `sha256:`;
`align: bounds` and its 0.5mm removed.

**Files.** `stave-geom/src/glb.rs` (new: reads the header and the JSON
chunk with `serde_json`; node names, transforms and each mesh's `POSITION`
`min`/`max`; `serde_json`, which `stave-scene` already uses; a SHA-256 implementation
is a new dependency, so ask before adding one),
`stave-check/src/models.rs`, `stave-solve/src/eval.rs` (`sha256:`).

**New types.** `ModelNode { name, translation: [f64; 3], bounds }`,
`Sha256([u8; 32])` with `FromStr`; `ModelMismatch::{Point, Shape, Hash,
MissingNode}`.

**Tests.** Small `.glb` fixtures under `tests/models/` built by a Rust test
helper (no Blender, no Python): `a_matching_model_is_quiet`,
`a_point_off_by_a_tenth_warns`, `a_model_in_millimetres_gets_the_scale_help`,
`a_z_up_model_gets_the_axes_help`, `a_changed_file_fails_its_pin`.

**Risks.** No example has a model, so this is exercised only by fixtures.

**Done when** the gate is green; STATUS no longer says models are not read.

## Stage 10. Authoring tools (proposal for the CLI lane; not scheduled) — size M to L

`stave hardware import model.glb` drafts a kind from named nodes (members,
shapes from bounds, pivots from nodes named for two members);
`stave hardware import --update kind.stave model.glb` rewrites only points
and shape sizes, so the result is a text diff; `stave hardware preview
<kind>` opens the kind on a scrap of wood with its drive on a slider. All
three sit on `stave-geom::glb` and `stave-app`. Hand-off H4.

---

## Deleted by the end

- `crates/stave-schema/src/hinges.rs`: `CUP_HINGE_110`, `CupMechanism`,
  `CupLink`, `CupPoint`.
- `crates/stave-schema/src/kinds.rs`: `MountRule`, `OpenAxes`, the `SLIDE`,
  `HINGE`, `PULL` tables, `CUP_HINGE_ENTRIES`, `BUTT_HINGE_ENTRIES`,
  `MemberName`, the member tables, `members_of`,
  `DEFAULT_SCREW_DIAMETER_NM`.
- `crates/stave-lang/src/placement.rs`: `HardwareTerms`,
  `HardwarePlacement`, `HardwarePlacementError`; `Code::{HingeOverlay,
  SlideGap}`.
- `crates/stave-model`: `HardwareData`, `OverlayRange`, `CupMechanism`,
  `LinkBar`, `HardwareMember`, `Hardware::{envelope, fixed, mechanism}`,
  `is_butt_hinge`, `Motion::{Turn, Linkage, Travel}`, `Pivot`,
  `CutPlace::{Recess, Mount}`, the cup-or-butt resolver code.
- `crates/stave-solve/src/hardware.rs`: `Data`, `OverlayRangeExact`,
  `Mechanism`, `HingeStyle`, `Bore`, `Recess`, `MotionExact`, `cup_linkage`,
  `cup_arm`, `CupFrame`, `CupArmFrame`, `butt_hinge`, `hinge_plate`,
  `HingeFrame`, every `match kind.base`; `bore_cut`, `recess_cut`.
- `crates/stave-geom`: `hinge.rs`; `steel_in_wood`, `SteelInWood`,
  `mechanism_contact`, `MechanismHit`, `MechanismMember`, `HingePart`,
  `Mechanism`, `MEMBERS`, `LinkageSweep`, `linkage_contact`,
  `linkage_reach`, `LinkagePath`, `hardware_fixed`, `hardware_links`;
  `AngledSlot`, `Unsupported::{Angled, AngledSlot}` (stage 2a).
- `crates/stave-check`: `hinges.rs` (`hinge_overlays`, `mechanisms`,
  `MAX_INSET_NM`, `MAX_PLATE_NM`), `slide_gaps`, `misfit_slides`, the
  own-parts arm of `exempt`, `Sweeper`'s three variants, `motion_hardware`,
  `cavity_noun`'s word list, the can't-cut arm of `exempt` (stage 2a).
- `crates/stave-scene`: `LinkFollower`, the member labels in `graph.rs`.
- `tools/hinge-sim/linkage.py`, `tools/hinge-sim/fit.py`,
  `tools/joint-sim/chidori.py`.
- Spec: the "Hardware members" table, the mount-face table, "what each
  base does", the own-parts sentence, `slide-gap`, `hinge-overlay`.

## Decisions wanted before the stage that needs them

| Before | Decision | Recommended |
|---|---|---|
| 0 | May the core lane edit `crates/stave-app/tests/`? | Yes |
| 3 | The cup's rim and the Ø3mm pins are drawn in wood today (audit gap 1) | Declare only true shapes, stop drawing the rest; record the door gap as follow-up |
| 3 | Are a kind's own members checked against each other (gap 2)? | No, as a general rule in the spec |
| 4 | Move a butt hinge's pivot out to its knuckle (gap 3)? | Yes, as its own commit with the dump change explained |
| 6 | New codes `adjust` and `mechanism`; `slide-gap` and `hinge-overlay` removed | Yes |
| 7 | Equal board widths; sheets and length stay errors | Yes |
| 2a | Accept `slope: 1 in n` as an exact alternative to a dovetail's `angle:` | Yes |
| 8 | The `CLAUDE.md` text of rule J | As drafted in the audit |
