# Slanted members: angled holes for pocket screws

Audit finding 103: no cut could go into the wood at an angle, so pocket
screws, the joint most beginners build face frames, carcasses and aprons
with, could not be written. This note records the design of the new
primitive and why it differs from the audit's sketch.

## What the audit proposed

`drill(diameter:, depth:, tilt: 15deg, toward: length)`, the same `tilt:`
on member shapes, and a std kind that wrote the tilt twice: once on the
screw's shape and once on each drill.

## What was built

- **The slant lives on the member's shape, and the drill follows it.**
  `cylinder(..., tilt: 15deg, toward: across)` turns a cylinder's axis
  about `at`, from the frame direction it runs along toward another one.
  A `drill` on `crossing(role, member)` of a slanted shape is centred
  where the axis meets the face and runs along the axis. Writing the
  angle twice would let the screw and its holes disagree; with one tilt
  the holes in both parts line up by construction, which is the reason
  joint members exist (grammar, "Joint members"). `toward:` uses the
  kind frame's own words (`along`, `across`, `depth`), which already say
  which way is which on a contact patch.
- **No `tilt:` on a drill written on its own.** A slanted hole without a
  member would need a direction word for the part being drilled, and the
  part's own axes have no sign a beginner can name ("toward the high end
  of its length" is geometry, not woodworking). Splayed legs and other
  free angled holes are left for a later change that can name a
  direction in woodworking words (toward a part, most likely).
- **`from_depth:` on `drill`.** A pocket is a stepped hole: a wide
  counterbore from the face of the rail that stops short of its end, and
  a narrow shank hole that runs on out through the end. Measured from the
  joint line, the counterbore starts `from_depth` along the slant and
  runs `through`; the shank hole runs from the joint line to where the
  counterbore starts. Without `from_depth` the counterbore would come out
  of the end of the rail and leave nothing for the screw head to bear on.
- **`through` on a slant** runs until the hole leaves the part, wherever.
  So a slanted `through` hole opens onto every face it runs out of, and
  the wall backstop does not call the pocket's oval a breakout.
- **Exact by rounding once.** The core crates may not use floats, so the
  sine and cosine of a tilt are worked out in whole numbers
  (`Angle::sin_cos`, a fixed-point Taylor series scaled by 10^18) and a
  length times one is rounded to the nanometre once, as a dovetail's
  `depth × tan(angle)` already is.

## Checks, and where they are conservative

- At rest, a slanted cylinder is tested against boxes of wood along its
  slant (its axis clipped to the box, the depth the lesser of how far it
  runs in and how far its surface reaches past the nearest face), and a
  member in a slanted hole on its own axis that is at least as wide is
  cut back to where the hole doesn't hold it, as a straight pin is.
- Against other members, against cuts that are not holes, against a
  straight hole, and in the checks of moving parts, a slanted cylinder
  counts as the box around it: more wood than there is, never less.
- Walls of a slanted hole: to a flat face, from the nearer end disc
  (`axis distance - radius × sin(angle to the face)`); to another cut, as
  a capsule (axis distance less the radius), which is never more than
  the true wall.

## The std kind

`pocket_screw` in `std/joinery/pocket_screw.stave`: a 15deg slant toward
the board's thickness, a 9.5mm counterbore, a clearance hole through the
end and a pilot hole in the other part. It has no `insert:`: a slanted
screw cannot come home by sliding the rail in, so the parts are clamped
and the screws driven, as its `note:` says. `examples/joint_samples`
uses it to join a face frame's rail to its stile.
