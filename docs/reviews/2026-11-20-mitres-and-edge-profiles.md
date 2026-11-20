# Mitres and edge profiles

Audit finding 105: no std kind cut a mitre, and no operation chamfered or
rounded an edge, so a picture frame, a mitred box or a table top with
eased edges, first projects for many beginners, could not be written.
This note records the two new primitives and the choices made.

## What the audit proposed

- std: `kind mitre : joint`, with parts drawn overlapping square at the
  corner and a cut on each written as `cut mitre: end(piece, facing:
  into) { outline(...) }`, plus an optional spline member.
- Language: a part field `edges: [chamfer(3mm, on: [top.front,
  top.left]), roundover(radius: 6mm, on: top.front)]`, naming each edge by
  the two faces that meet there.

## What was built

### Mitres: the `diagonal()` cut operation, and std's `mitre`

- **Why a new operation, not an `outline`.** An outline's marks are
  measured from a part's reference edge, and which edge is the outside of
  a frame depends on where the part is placed: the left stile's outside
  is its left edge, the right stile's its right. A kind can't say "the
  edge away from the other part" in marks, so an outline-based mitre would
  be right for some corners and wrong for others. `diagonal()` is worked
  out from the relationship instead: on `crossing(p, q)` where the parts
  cross square at a corner, the outside corner is the corner of the
  crossing at the end of both, the inside corner the one opposite, and
  each part gives up the triangle at its own end. The same kind is right
  at all four corners and for a box's standing sides (mitred across their
  thickness) as for a frame's flat members (across their width).
- **Parts drawn crossing**, as the audit proposed: each member is drawn
  at its full length to the outside of the frame, which is also its
  length on the cut list before mitring (the long point). No `adds:`.
- **Unequal widths** meet on the corner-to-corner line, as a mitre between
  a wide and a narrow member is cut; equal widths meet at 45deg. The
  triangle's corners are corners of the crossing, so nothing is rounded.
- **Errors** are the geometry's shape errors, reported at the joint as
  `type-mismatch` with the part drawn uncut, as an outline that crosses
  itself is: parts that run the same way, and parts that don't cross at
  an end of each (a rail mitred into the middle of a stile, parts that
  only butt).
- **The kind is named `mitre`** as the audit asked, so the operation
  can't be: the core may not use a std kind's name. `diagonal` says what
  the cut is.
- **Left out: the spline.** A splined mitre needs a slot square to the
  mitre face, which no operation cuts yet (an angled `slot` turns in the
  face, not across the joint line). It is left for a later change; the
  kind's doc comment says to choose a half lap or a mortise and tenon for
  a frame that takes a load.
- **Recipes**: `diagonal_by_mitre_saw` and `diagonal_by_hand` (a sliding
  bevel, a tenon saw and a block plane), and a `mitre_saw` tool.

### Edge profiles: a part's `edges:`

- **`edge(a, b)` and `around(face)`, not `top.front`.** `top.front` already
  means the front face of a part named `top`, and std's tables have parts
  named `top`; inside a part's body it would read either way. `edge(top,
  front)` can't be misread, and `around(top)`, all four edges of a face,
  is what a table top usually wants. Faces are the world faces the part
  shows once placed, as `face_side:` and a splice's `from:` use.
- **A profile is a cut of the part itself** (`CutSource::Edges`), made
  as a prism along the edge, so drawing, the fit and contact checks,
  mirrored assemblies and splices handle it like any other cut. It runs
  along the part's drawn box: what joints add (a shelf's ends in their
  dados) is left square, as the visible edge stops at the joint.
- **A roundover is twelve flats touching the circle**, so the geometry
  stays exact polygons in nanometres (the corners from the integer
  `Angle::sin_cos`, rounded once). The flats leave a little more wood than
  the curve, never less, as the slanted members' boxes do: a check can
  only find more wood than there is.
- **`chamfer` and `roundover` are operations** so recipes can be written
  for them (router bits, or a block plane), but they are written under no
  selector: in a kind's `cut` block they are `type-mismatch`, saying to
  use the part's `edges:`. The operations table lists their place as
  `edges:`.
- **Checks**: two faces on one axis, an edge shaped twice, a size of 0mm
  or as big as the part across a face it runs along, and a profile on a
  round or turned part.
- **Left out**: unequal chamfers (two sizes), other profiles (cove, ogee,
  bead), and stopped profiles that run only part of an edge. Two profiles
  on opposite edges of one face that together are wider than it are not
  an error; walls don't measure profiles, as they don't measure outlines.
- **Recipes**: `chamfer_by_router`, `chamfer_by_hand`,
  `roundover_by_router` and `roundover_by_hand`, with `chamfer_bit` and
  `roundover_bit` tools.

## Where it is used

`examples/picture_frame`: a frame mitred at its four corners with its
outside top edges rounded over and its inside top edges chamfered, and a
box whose standing sides are mitred across their thickness, chamfered
along their top edges, on a base rounded over underneath.
