# Shelf supports: adjustable shelves on pins

Audit finding 102: an adjustable shelf, the most common shelf in a
beginner's first bookcase or cabinet, could not be written. A shelf on pins
doesn't touch the side, so it can't be a joint pair; hardware had no base
for it; and the row of spare holes that lets the shelf move had nothing to
make it, because every hole came from a member and the spare holes are
empty. The only way out was `loose: true` on a shelf floating in the air,
with no pin holes in the cut list and nothing checking that the shelf was
held. This note records what was built and how it differs from the
audit's sketch.

## What the audit proposed

A hardware base `support` (words: "rests on") and a `std` kind
`shelf_pin`, with the shelf as its first role and the side as `on:`; an
optional `holes: row(pitch: 32mm, from: 300mm, to: 900mm)` on the
`hardware` declaration to drill the adjustment row; and the held check
counting a part resting on a support as held.

## What was built

- **The base `support`, type `SupportKind`.** Like `slide`, `hinge` and
  `pull` it chooses words (the window calls it a "shelf support" and its
  focus card "What this shelf rests on") and a type, and nothing else. A
  support kind may hold exactly what the other mounted kinds may hold.
- **The side is the first role, not the shelf.** Hardware is mounted on a
  face of its first role, and the pins go into the side; making the shelf
  the first role would have needed a new rule for mounting hardware on a
  part other than the piece. So `shelf_pin(side_l, shelf: shelf)`.
- **`face(piece, toward: q)` for a mount's `face:`.** A pin goes into the
  face of the side the shelf is on, which is the left face of one side and
  the right face of the other. `face(piece, nearer: r)` only took a Region
  role, and its rule (the face nearer the same-named face of the region)
  picks the wrong face for a part beside the piece. The new form takes a
  `Part` role and picks the face nearer its middle, the same rule as the
  existing `face(p, toward: q)` anchor of `across:` and `along:`.
- **Rows of holes on the kind's cut, with counts, not on the hardware
  with heights.** `cut pin_hole: row(crossing(piece, shank), pitch:
  hole_spacing, up: holes_above, down: holes_below) { drill(...) }` makes
  the member's own hole and then the same cut again, a pitch apart,
  toward each direction named. The counts are the kind's params, so the
  design chooses them with the call (`shelf_pin(side_l, shelf: shelf,
  holes_above: 5)`). The audit's `from: 300mm, to: 900mm` gives heights
  measured from the floor, which are coordinates (principle 3); a count
  either way from the pin is a relationship, and it moves with the shelf.
  The row lives on the kind's `cut` rather than on the `hardware`
  declaration because a kind may have several cuts and only the kind
  knows which one makes sense to repeat. The directions are the six
  direction words, which a beginner already knows from `orient(...)`;
  the frame's own `along`/`across` would make "up" depend on which way a
  side's length runs.
- **Nothing new in the held check.** A piece of hardware already holds the
  parts its roles name, once each role's part meets one of its members
  (the contact rule). The shelf rests on the `peg` member and is held by
  it, so a shelf on pins needs no `loose: true` and no joint, and a shelf
  with its pins left out is still `warning[loose-part]`.
- **`std:casework` gains `AdjustableShelf`**: a board a millimetre clear of
  each side, two pins under each end and four spare holes above and below
  each pin by default. `examples/adjustable_bookcase` uses two of them.

## Checks

At the hardware's call, in layer 4: a pitch of 0mm or less and a count
that is not a whole number from 0 to 1000 are `error[out-of-range]`; a
direction into the face the hole is drilled in is
`error[type-mismatch]`; a copy that doesn't lie wholly on the part along
the row is `error[wall]`, with the room left that way in the help. A copy
that lies on the part is an ordinary hole to every other check: it keeps
a wall to the faces and to every other closed cut. In a joint kind,
`row(...)` is `error[type-mismatch]`: a joint lays out a row with a
member's `count:`, and every hole in it has something in it.

## What is left out

- **Resting is not checked against gravity.** The pegs are under the shelf
  because the design places them `against(shelf.bottom)`; written
  `against(shelf.top)` the pins would touch the shelf from above and still
  satisfy the contact rule. A rule that knew which way is down for a
  support would be the base choosing behaviour, which `CLAUDE.md` forbids;
  if it is wanted, it is a language primitive any kind can use (a
  `require` that a member is below a role's part), not a support rule.
- **One `hardware` per side.** The four pins of a shelf are two arrays, one
  for each side, because a hardware's first role is one part.
- **A row of holes is one cut repeated; the 32mm system's continuous
  line-boring of a whole side is not modelled separately.** A side with
  two shelves gets two rows, one per shelf. Where the two rows are a
  whole number of pitches apart their holes fall on each other and are
  shared; a hole that runs partly into another row's is caught by the
  wall backstop (`its pin hole in side_l runs into the pin hole of
  upper_shelf.left_pins[front]`).
