# Turning frames: decision

Status: **go** (2026-11-22). Phases A and B are built; Phase C (running
clearance) is still to do. The loom is
[`examples/tapestry_loom`](../../examples/tapestry_loom/loom.stave). The full design, with reasons, worked
examples and the evidence, is in
[`2026-11-22-turning-frames.md`](2026-11-22-turning-frames.md). Where the
two differ, this page is the proposal.

## The problem

A tapestry loom can't be written in Stave. Its frame tilts and turns on a
pivot bolt each side, and it is set higher or lower by moving the bolts
between holes. Building it, and five more designs like it, turned up two
places where a bad design checks out:

- a frame that rubs its stand face to face, and
- a bolt that would pull a gap shut when tightened.

## What we'd add

Ten changes. Each one is generic: none names a kind, and most remove a
restriction rather than add a case.

| # | Change | In the language | Why |
|---|---|---|---|
| 1 | **`pins:`** | `mechanism { pins: depth ... }`. Pins run along `along`, `across` or `depth`. Default as today | A pivot bolt or turntable's pin goes *into* the mount face. Today pins must lie in it |
| 2 | **A role in a mechanism entry** | `slot bolt, piece from ... to ...`. The second name may be a role's part | A bolt turns in a hole in the wood, not on a steel member |
| 3 | **Named drives** | `drive tilt: turn(...)`, `drive height: travel(...)`, any number of them; `pose frame.tilt = 15deg`. One unnamed drive works as today. Solver: "Hang" reworded to "pinned at a known point". No new step | Raising and turning one frame |
| 4 | **`set_at:`** | `travel(..., set_at: every(50mm))` or a list. Poses must land on one of the values | A row of holes, an indexed angle |
| 5 | **`loose` on a member** | `member bolt { loose: true }`. Checked wherever it's set, never swept | You pull the bolt to move between holes |
| 6 | **Motions one at a time** | Each motion is swept with every other held at closed, at each scene value and at each `set_at` value. Nested assemblies are now swept too. A round body turning about its own axis, in a hole on that axis, sweeps only itself | Combining motions exactly. Without the second rule, a bolt turning in its own hole would be reported as a collision |
| 7 | **Running clearance** | Wood moving past wood needs a gap greater than the seasonal swell (worked out from `max_held_width`), or it's `error[motion]` ("binds"). A gap under swell plus `running_gap` is `warning[clearance]`. Seating at the end of a motion is fine. **A spacer is a bearing**: wood is a wear warning, steel or nylon is clean. New project field: `running_gap` (default 1mm) | A frame rubbing its stand checked clean |
| 8 | **`clamp`** | `clamp knob, bolt along depth` in a hardware kind. The stack it draws together must be solid, or it's `error[no-contact]`. Checked where the hardware is tightened | A bolt across an empty gap checked clean |
| 9 | **Project block** | `finish: Text`, plus patch paths `set project.goal`, `project.finish`, `project.title`, `replace project { }` | The goal and finish couldn't be recorded |
| 10 | **Two spec sentences** | "A part is never placed at an angle; an assembly turns only on its hardware" (replaces "rake and splay are out of scope for version 1"). "A hardware member is bought hardware, never wood" | Keeps rotation in one place, and stops a wooden disc passing as steel |

## What we would not do

- **`turn(about:, by:)` on a part.** A tilted part has no `top`, and it
  would need angled joinery: the whole rake-and-splay project. The loom's
  angle is a pose of its hardware.
- **A cap of one turn plus one travel, with poses picked by value type.**
  Replaced by named drives.
- **A new solving step.** Rewording "Hang" covers it.
- **A structured finish.** Text for now. It would get its own review.

## Evidence

- **Today, every bad version checks clean.** We built the loom plus a
  cheval mirror, a camp stool, a drafting table, a sawhorse and a porch
  glider, in their gap-without-washer and touching versions. All of them
  check clean on `main` (7f9343a). Today Stave catches only a spacer that
  is too thick.
- **Under changes 7 and 8, worked by hand:**
  - Touching, in a design that moves: error.
  - A gap with nothing in it, or a spacer too thin: error.
  - A spacer that fills the gap: clean (wood gives a wear warning).
  - The sawhorse, which doesn't move, is clean touching, as it should be.
- **A wooden disc**, turned as a part with the kind drilling it, is valid
  geometry in today's Stave.

## Cost and order

Rough sizes: S is days, M is about a week, L is two or more weeks. Each
phase ships on its own and leaves `main` green.

| Phase | Changes | Size | Depends on |
|---|---|---|---|
| A: static | 8 `clamp`; 9 project block; 10 the member sentence | M | nothing |
| B: motion | 1 `pins:`; 2 roles in entries; 3 named drives; 4 `set_at:`; 5 `loose`; 6 the sweep rule; 10 the rake/splay sentence | L | nothing |
| C: clearance | 7 running clearance with spacers | M | B (it reads the sweeps), A (spacers come from `clamp`) |

Each phase touches the grammar first, then `stave-lang` through
`stave-check`, with corpus files and diagnostic snapshots. Phase B also
needs the motion bar to show one slider per drive. `examples/tapestry_loom`
lands with B and has to check clean after C.

## Risks and open questions

Risks 1 to 3 were checked on 2026-11-22. Each has a remaining step to
take when Phase B is built.

1. **Mirrored pivots: low risk.** "Moving alike" compares the other
   driver's entries in the first driver's plane (`point_of`), and reverses
   the turn when the two are mirror images (`same_hand`). A pivot pair
   mirrored left to right is reflected along its own pin axis, which drops
   out of the plane's coordinates. So the two land on the same points and
   turn the same way. This is proven by a new unit test on today's code,
   `a_pivot_mirrored_along_its_own_pins_moves_alike` in
   `stave-solve/src/mechanism.rs`. *Done in B:* two `pins: depth`
   pivots written separately (`face: left` and `face: right`) move alike
   end to end: `two_pivots_written_each_on_its_own_face_move_alike` in
   `crates/stave-app/tests/turning.rs`, and the holed cheval
   (`set_positions.rs`) and `examples/tapestry_loom` check clean with
   them.
2. **A full 360deg turn: low risk.** The exact turn sweep measures each
   contact angle with `rem_euclid(TAU)` and has no 180deg assumption.
   Nothing in the solver, the checker or `Angle` caps a range. This is
   proven by a new unit test,
   `a_full_turn_finds_a_contact_past_half_way_round` in
   `stave-geom/src/sweep.rs`: a contact about 300deg round is found, and a
   clear 360deg turn reports none. The dead-point proof only runs for
   linkages, and a plain pivot never reaches it. *Done in B:* the spec
   accepts `to: 360deg` ("Mechanisms"), and
   `a_turn_may_run_all_the_way_round_and_a_whole_turn_is_the_closed_pose`
   in `crates/stave-app/tests/turning.rs` poses 360deg; the loom turns
   all the way round at each of its four heights.
3. **Sweep count: low risk.** Measured with the release `stave check` on
   `examples/clothing_cabinet` (4 motions, 2 scenes): 44ms. With 20 more
   scenes posing every motion (80 more sweeps): 235ms. That is about 2.4ms
   per sweep. The loom needs about 10 to 12 sweeps (about 30ms), and even
   300 sweeps is under a second. *Done in B:* the built loom checks in
   about 27ms in release and 300ms in debug, and
   `the_loom_checks_in_a_bounded_time` in
   `crates/stave-app/tests/tapestry_loom.rs` fails past 3s.
4. **Pivots that don't draw** (a plain pin, a cradle) aren't covered by
   `clamp`. They are left out of scope, and tracked in
   [#89](https://github.com/OceanEilonwy/stave/issues/89).
5. **End grain under a clamp.** A spindle-turned disc has end-grain faces
   and crushes. A possible future grain warning, not part of this. Tracked
   in [#90](https://github.com/OceanEilonwy/stave/issues/90).

## Decision

Recommended: **go**, in the order A, B, C.

- A is small and independent, and it closes the bolt-across-a-gap hole
  straight away.
- B is the bulk, and it is what lets the loom be written at all.
- C finishes the "bad designs don't check out" promise for anything that
  moves.

Alternatives:

- **Go on A only:** the static fixes now, with motion deferred.
- **No-go:** keep the loom flat with no motion, and document the two holes
  as known gaps.
