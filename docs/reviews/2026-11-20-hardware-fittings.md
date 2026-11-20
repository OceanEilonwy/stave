# Fittings: opening the hardware bases

Audit finding 111: the hardware bases were a closed list (`slide`,
`hinge`, `pull`, and since finding 102 `support`). The base chooses only
the words a message uses and the kind's type, so the list was the only
thing stopping a library from declaring a tabletop fastener, a cam lock,
a catch or a lid stay. Writing one under a base it isn't gave the wrong
words (a figure-8 fastener "pulled out"), and writing it as a joint lost
`mount`, `adjust` and `mechanism`. This note records what was built.

## What the audit proposed

One neutral base, `fitting`, with type `FittingKind`, accepting every
entry a mounted hardware kind accepts and giving generic words; then std
kinds `tabletop_fastener`, `cam_lock`, `confirmat` (as a joint member
kind) and `shelf_pin`.

## What was built

- **The base `fitting`, type `FittingKind`.** Like `slide`, `hinge`,
  `pull` and `support`, it chooses words (the window calls it a "fitting"
  and its focus card "How this fitting is fixed") and a type, and nothing
  else. A fitting kind may hold exactly what the other mounted kinds may
  hold, and is placed, drawn, checked, held and cut as they are. It joins
  `HardwareKind`, so a component that takes any hardware takes a fitting.
- **Why one base and not user-declared bases.** A base chooses only words
  and a type. Letting a library declare its own bases would add a new item
  (a base with its words and its type name) for no behaviour; one neutral
  base covers every product that is no slide, hinge, pull or support, and
  its type is still a type a component can ask for. The closed list stays
  an enum (CLAUDE.md: closed vocabularies are enums), now with a member for
  "everything else".
- **`end(p, nearer: q)` for a kind's `mount`.** A cam lock's cam is set a
  fixed distance in from the end of the panel that meets the other panel,
  and a panel has a cam lock at each end. `end(p, toward: dir)` names the
  end by a direction, which differs at each end, and `crossing(p, q)` is
  only a member's anchor. The new form picks the end nearer the middle of
  another role, the same rule as `edge(p, nearer: q)`. Like every `from`
  term it also turns the frame's axis to run away from the anchor, so the
  same kind serves both ends. On an axis the piece's length doesn't run
  along it is `error[type-mismatch]`.
- **The frame's directions, as the solver already had them.** The grammar
  said `along` runs from the end only for `from(end(...))` and `across`
  from the edge only for `from(edge(...))`; the solver turns the axis for
  every `from(...)` term. The spec now says so.
- **std:hardware `tabletop_fastener` and `cam_lock`**, in
  `std/hardware/fittings.stave`. The fastener sits in a recess in the
  apron's top edge, a millimetre in from its inside face, and its other
  end reaches past that face and is screwed up into the top; its rules
  keep the recess on the apron's edge and the upper end clear of it. The
  cam lock bores its cam in the panel's face, drills the bolt's hole into
  the panel's end and the thread's hole into the other panel; its rules
  keep the bolt's head inside the cam and the bore short of the far face.
- **The lid stay** in `tests/designs/lid_stay` was declared `: hinge`
  because no other base would take it; it is now `: fitting`, and moves,
  stops and is checked exactly as before.
- **Designs and broken cases:** `tests/designs/top_on_fasteners` (a top
  held by six fasteners and no joint), `tests/designs/knock_down_cabinet`
  (a bottom and a top held between two sides by cam locks, the top's in
  its underside through `face: bottom`), `tests/broken/tabletop_fastener_in_thin_apron`
  (the kind's own rule), `tests/broken/fitting_where_a_pull_is_needed`
  (the type) and `tests/broken/end_nearer_across` (the new anchor on the
  wrong axis).

## What was left out

- **`confirmat`.** A confirmat screw is a joint's fastener, not mounted
  hardware: it belongs with `screw` and `pocket_screw` in `std:joinery`
  (a joint kind with a stepped pilot hole), not with this base. It is
  not part of this change.
- **The side table still uses cleats.** `examples/side_table` holds its
  top through screwed cleats, a sound way to let a top move; it was not
  rewritten. `tests/designs/top_on_fasteners` shows the fastener instead.
- **No wood-movement check.** Nothing checks that a solid top is free to
  move; a fitting is checked like any hardware, for room, contact and
  holding, not for what it is for.
- **Message words.** No core message currently words itself by hardware
  base (motion messages word themselves by the drive: "opened", "pulled
  out"), so "fitted" is the word the base offers, not one any check uses
  yet.
