# Tapers and arches

Audit finding 104: the only `shape:` was `turned(profile)`, and an
`outline` could only be cut by a joint or hardware kind, from straight
marks. A leg tapered on its inside faces (Shaker and mid-century tables,
nightstands) and an apron with a gently arched lower edge, both ordinary
first projects, could not be written; faking a taper with an outline in a
kind left the cut list and the overlap check treating the leg as a full
square. This note records the new primitive and the choices made.

## What the audit proposed

- `shape: tapered(profile, faces: [right, back])`, reusing a lathe
  profile's stations as the width at each distance from the start, cut off
  the named faces.
- `arc(through: mark(...))` between two marks of an outline, so a curve is
  a list of marks.

## What was built

Two shapes for a square part, said the way a woodworker marks them out:

```
shape: tapered(to: 25mm, toward: bottom, on: [right, back], square: 125mm)
shape: arched(rise: 30mm, on: bottom, ends: 40mm)
```

- **`tapered`**: the faces `on:` names (as placed, the way `edges:` names
  faces) are planed in a straight line from `square:` below the wide end to
  the narrow end `toward:` names, where the part is `to:` across. Both
  faces of one axis share the taper, so the part stays centred.
- **`arched`**: a part of a circle sawn into the face `on:` names, its
  ends on the face `ends:` in from each end of the part and its middle
  `rise:` into the part.
- **Made as outline cuts of the part itself** (`CutSource::Shape`,
  `CutPlace::Side` from the part, `CutKind::Outline`, through): one
  triangle per tapered face, one 25-cornered outline per arch. So the 3D
  view, the fit and contact checks, the overlap check (which sees an
  outline's slanted pieces), mirrored assemblies, splices and the work card
  (an `outline` recipe) all handle them with no new operation, and the cut
  list sizes the square blank, as a turned part's. The marks are measured
  on the drawn box and worked out once, in nanometres: the arch's corners
  are on its circle, found with an integer square root, rounded toward the
  chord, so it leaves a little more wood than the curve, never less.
- **Mirror images** take their source's cuts in their own frame, moved to
  their own drawn box, so a leg mirrored about the table's middle tapers on
  its own inside faces (`examples/tapered_table` writes one leg and mirrors
  it three times).

## Choices that differ from the audit's sketch

- **Plain words, not a profile, for a taper.** A profile's stations are
  coordinates along the part; a beginner says "taper the inside faces to
  25mm at the foot, starting below the aprons". `to:`, `toward:` and
  `square:` say exactly that, and cover the tapers beginners build. A taper
  that curves (a profile on a square part) is left for later.
- **`arched(...)`, not arcs in an outline.** An outline is cut only by a
  kind, from marks measured from a reference edge, so an arc in one would
  still not let a part be arched on its own, and would ask a beginner to
  place three marks. A shape on the part, with its rise and its straight
  ends, is the way the curve is laid out with a bent batten or a trammel.
  Freely marked curves in an outline are left for later.
- **`on:`, not `faces:`**, matching `edges:` and `banding:`.

## Checks

Each has a `tests/broken` case: a face along the length in `toward:`
(`taper_toward_a_side`), an end in `on:` (`taper_on_an_end`,
`arch_on_an_end`), a face named twice (`taper_face_twice`), `to:` as big
as the part (`taper_wider_than_the_leg`), a rise as big as the part
(`arch_too_high`), `square:` or `ends:` leaving nothing to shape
(`taper_square_too_long`, `arch_ends_too_long`), round stock
(`taper_on_a_dowel`), and an edge profile on a shaped face
(`chamfer_on_a_tapered_face`).

## Left out

A curved taper, an arch that is not part of a circle, an arch on an end,
two shapes on one part, and freely marked curves in an outline.
