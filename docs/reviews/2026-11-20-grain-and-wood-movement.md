# Grain and wood movement

Audit finding 89: nothing checked grain direction. A batten glued across
a solid top, an apron butted end on against a leg and only glued, a
tenon whose grain runs across it, and a top screwed down tight at two
rows 370mm apart across its grain all checked clean, and one shipped
example (`examples/side_table`, and with it `side_table_from_box` and
`tapered_table`) was the last of these, while its cleat's note told the
builder to slot the holes. Wood movement and short grain are the most
common reasons a correctly sized piece of furniture fails. This note
records what was built (grammar, "Grain and wood movement") and how it
differs from the audit's sketch.

## What the audit proposed

Make `grain` readable on a part; add a built-in `across_grain(p, of:
contact(a, b))`; give `glued_face` a rule `!glue || across_grain(...) <=
80mm || ...`; give `mortise_tenon` and `round_tenon` a rule `piece.grain
== length`; give `groove` a rule for solid panels; add an engine pass
warning `wood-movement` for rigid fixings spread across a solid part's
grain, with a species-dependent span; and add slotted clearance holes to
`screw` and use them in the side table.

## What was built

- **Checks, not kind rules.** All four mistakes are warnings the checker
  makes for every kind alike, as the backstops of "Cuts stay in their
  part" are, rather than `require`s in a few std kinds. A kind's rule
  protects only that kind: a library's own glued joint or tenon would be
  as unprotected as before, and the audit's `glued_face` rule needed a
  new built-in and a readable grain to say what the checker can see
  directly. The checks read only the language's own words: `glue:`, a
  part's `grain:` and its stock's `form:`, the `tenon`, `spigot`, `drill`
  and `pocket` operations, a joint's members and its `adds:`. No kind is
  named (the standing rule holds). `grain` stays unreadable: nothing
  needs it now, and making it readable is a separate change when a kind
  does.
- **One code, `warning[grain]`, four messages**: end grain glued, glue
  across the grain, short grain, held fast across the grain. One code
  because they are one subject a beginner looks up once; the messages
  say which. They are layer 4 and, like the held check, asked only of a
  design with no errors. A part warned for glue is not warned again for
  being held fast.
- **Stage 1, glue.** Only glue that is all on the patch where the parts
  touch is measured: a glued pair whose kind cuts, places, makes and adds
  nothing (`glued_face`). Glue in a joint's cuts (a tenon's cheeks) is
  not, because the contact patch is not where that glue is: measuring a
  mortise and tenon's patch would call every 90mm apron's end-on meeting
  with its leg "end grain" and "glue across the grain", which the cheeks
  make sound. End grain: one part's grain runs square to the patch. Glue
  across the grain: the other part holds still (a sheet, or solid wood
  whose grain runs that way) over more than 80mm of the patch, measured
  along the way the solid part moves.
- **The stated width is 80mm, for every species.** The audit asked for a
  species-dependent span; it would need a table of shrinkage figures per
  species and per cut (flat or quarter sawn, which Stave doesn't know)
  that would pretend to more precision than the check has. 80mm is about
  where a season's movement in flat-sawn oak reaches a millimetre, the
  figure the audit's glue rule used. The spec states it and says species
  are not yet taken into account.
- **Stage 2, short grain.** A `tenon` or `spigot` cut on the end of a
  solid part whose grain runs along its width. A tongue (a `tenon` on
  `crossing`, along an edge) has the same grain geometry but is captured
  in its groove along its whole length, and is left alone.
- **Stage 3, held fast.** A solid board is fixed to another part by glue
  on the patch, by glue in a housing a joint extends it into across its
  grain (a solid panel glued into its grooves), or by a member with a
  round hole (`drill`) in both parts (a screw, a dowel). Two refinements
  were needed to keep the shipped designs honest:
  - Fixings in different parts pin the board only when those parts are
    joined to each other other than through the board. Two legs doweled
    into a bench seat front and back, with no stretcher, splay a little
    as the seat moves; two cleats joined through the aprons and legs
    can't (`tests/designs/spliced_bench` versus the side table).
  - A part made of boards whose seams are all unglued moves board by
    board, and each board is measured on its own
    (`tests/designs/unglued_top`, battens screwed across a dry tongue
    and groove top, one screw per board).
  Fixings in one other part count with their spread only when that part
  holds still the way the board moves; so a cleat running along the
  top's grain, screwed at three places along it, doesn't count its own
  spread, while a batten running across the grain does.
- **Slots without a new operation.** A slotted clearance hole is a
  `pocket` on `crossing(piece, member)`, which the geometry already cuts,
  centred on the member's shape, its `width` along the part's length and
  its `thickness` across it. The spec now says so (it said only "in the
  entering part's axes", which a member doesn't have). A member whose
  hole in either part is not a `drill` is taken as free to move. Which
  way the slot is long is not checked: that would need the slot's
  direction compared with the moving part's grain, and a slot is only
  ever long across the piece in std. Stated in the spec.
- **std gains `slotted_screw`**, `screw` with its clearance hole cut as a
  slot (`slide`, default 3mm each way) and a seat rule that allows for
  the slot. `screw` itself is unchanged: a new kind rather than a
  `slotted:` flag on `screw`, because a kind can't make a cut depend on
  a param, and most screws should hold tight. `std:frames`'s `Cleat`
  uses it.

## The examples

- **Corrected, really wrong: the side tables' tops.**
  `examples/side_table`, `examples/tapered_table` and `std:frames`'s
  `Cleat` (so `examples/side_table_from_box`) screwed a solid top down
  through round holes in a front and a back cleat. They now use
  `slotted_screw` (the cleats' note already asked for slots). The old
  form is kept as `tests/broken/top_screwed_tight_through_cleats`, and
  `tests/broken/slats_held_by_nothing` (a copy of the side table) was
  changed to match.
- **Corrected, really wrong: the picture frame's box base.**
  `examples/picture_frame` glued a solid 160mm base under all four sides
  of a small box; the ends held it still across its grain over its whole
  depth. Its base is now plywood. (A solid base glued only to the front
  and back would still be held across its grain, between them.)
- Every other example, std module and test design checks clean as it
  was: their glued joints run with the grain or have their glue in cuts,
  and their screws and dowels sit along the grain or in parts that move
  with the board.

## Left for later

- Glue in a joint's cuts: a tenon wider than 80mm glued across a leg, a
  panel glued in a groove that doesn't extend it.
- `groove`'s `glue: true` default and `clearance = 0mm`: the audit asked
  for a rule that a solid panel is unglued and has room to swell. With
  the extension rule above a solid panel glued into grooves on two sides
  across its grain is now warned; room to swell within the groove is not
  checked.
- A slot that is long the wrong way, movement in a part's thickness, and
  species.

Glue in a joint's cuts, slot direction and a limit per stock
(`max_held_width`) were taken up afterwards: see
`2026-11-20-grain-gaps.md`.
