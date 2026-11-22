# Turning frames: decision

Status: **awaiting go/no-go**. The full design, with reasons, worked
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

1. **Mirrored pivots.** Two pivots mirrored across one assembly must count
   as "moving alike". Check `same_hand` in `stave-solve/src/mechanism.rs`
   first. If it refuses them, B grows a little.
2. **A full 360deg turn.** 0deg and 360deg must be treated as the same
   pose, and the dead-point proof checked for a wrap-around assumption.
3. **Sweep count.** It grows as motions times held values: the loom needs
   about 10. Report it if a design reaches hundreds.
4. **Pivots that don't draw** (a plain pin, a cradle) aren't covered by
   `clamp`. They are left out of scope.
5. **End grain under a clamp.** A spindle-turned disc has end-grain faces
   and crushes. A possible future grain warning, not part of this.

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
