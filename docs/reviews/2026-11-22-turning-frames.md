# Turning frames: weaving angles, pivot bolts and set positions

Status: **proposed**. Nothing here is built yet.

A tapestry loom (`~/stavetest/tapestry_loom`) cannot be written. Its frame
hangs between two uprights on a pivot bolt each side. It tilts back to a
weaving angle, turns all the way round so the warp can be wound on, and is
set higher or lower by moving the bolts to another hole in a row. Four gaps
stop it:

1. No part can be placed at an angle ("rake and splay are out of scope for
   version 1").
2. A mechanism's pins have to lie in the mount face, like a hinge pin. A
   pivot bolt goes *through* the face, so it can't be written.
3. An assembly has one motion. Raising and turning can't be combined, a
   motion inside a moving assembly is never swept, and a turn can't be set
   to fixed angles.
4. A patch can't change the `project { }` block, and there is no field for
   the finish.

This note proposes five small language changes that cover all four. It also
says why the general `turn(about:, by:)` placement term asked for in gap 1
is the wrong tool for this job.

## The idea in one paragraph

A loom frame's angle is not a measurement on a drawing. It is where the
frame has been *set* on its bolts. Stave can already turn a group of parts:
an assembly turns as one piece when the hardware inside it has a drive, and
a scene poses it (`pose door = 110deg`). So the angle should come from the
hardware's motion, as a door's does, and not from a new placement term. That
keeps principle 2 (woodworking words, not geometry): the pivot line *is* the
bolt, a thing you buy and drill for, and not a line the design has to draw.
What is missing is narrower than gap 1 suggests. The motion system needs to
accept a pin that runs into the face (gap 2), two drives on one assembly
(gap 3), set positions (gap 3), and a correct sweep for a pin turning in
its own hole. Gap 4 is separate and small.

## Change 1: `pins:` in a mechanism (gap 2)

Add one optional entry to `mechanism { }`:

| Entry | Meaning |
|---|---|
| `pins: along \| across \| depth` | the frame axis every pin is parallel to. The members move in the plane of the other two axes. Left out: `along`, or `across` when a `rail` runs `along along` (today's rule, unchanged) |

`pins: depth` is a pin that runs into the mount face: a pivot bolt, a lazy
Susan, a swivel. Points are then written in `along` and `across`. A
`point(...)` that gives a value on the pins' own axis is
`error[mechanism]` (`this mechanism's pins run along 'depth', so a point
is given by 'along' and 'across'`). So is a rail along the pins' axis, as
today.

**Which way is positive.** Today "a positive turn carries `across` toward
`depth`". This generalises cyclically, in the order along, across, depth.
With pins along `along`, a positive turn carries `across` toward `depth`,
as now. With pins along `across`, it carries `depth` toward `along`. With
pins along `depth`, it carries `along` toward `across`. Every existing kind
keeps its meaning.

**Why this is cheap and safe.** The solver already works in a plane whose
pin axis is a world `Axis` (`stave-solve/src/mechanism.rs`, `Plane::axis`).
A slide already moves in the along/depth plane. `pins:` only chooses which
two frame axes become the plane's two coordinates when the mechanism is
declared. The six solving steps, the branch choice, the dead-point proof
and the fast paths are planar and don't change.

**The workaround in the report** (mounting the pivot on a frame beam and
reaching out to the upright) is not needed and should not be documented.

## Change 2: a role in a mechanism entry

Today every name in a mechanism entry must be a member with a body. A pivot
bolt turns in a hole in the upright. The upright is a *part*, and the kind
has no steel member on it to pin the bolt to. Today's loom kind gets round
this by holding the knob on the upright, and that leaves the knob behind
when the bolt moves to another hole.

Allow the **second** name of a `pivot`, `slot` or `rail` to be a role: the
role's part, held or moving as the role is.

```
slot bolt, piece from point(along: 0mm, across: 0mm) to point(along: -300mm, across: 0mm)
```

A role adds nothing to the freedom count, because a part is never free. A
role whose value is a region, or a list of parts, is `error[mechanism]`
(`a pin turns in one part; 'opening' is a region`). The first name stays a
member, because the moving side is what the mechanism places.

## Change 3: two drives, one turning and one travelling (gap 3)

A mechanism may have **up to two** drives: at most one `turn(...)` and at
most one `travel(...)`.

- **Freedom.** The count must come to the number of drives (one or two).
  The formula is unchanged: three for each free member, less two for each
  `pivot` or `rail`, less one for each `slot` or `couple`. A pin in a slot
  that also turns, which is a pivot bolt in a slot or a row of holes, is
  three less one: two freedoms.
- **One new solving step**, before "Hang": *Set*. A member whose angle and
  travel are both known, with its pin in a slot in a placed member or part,
  is placed. Its pin sits `travel` along the slot from `from`, and the
  member is turned by its angle about that pin. It is closed form, like
  the rest.
- **One motion each.** The assembly now has one motion for each drive,
  and the motion is named by what it does. `pose` picks it by the value's
  type: an `Angle` sets the turn and a `Length` sets the travel. So a scene
  may pose one assembly twice, once with each type.

  ```
  scene weaving "Weaving" {
    pose frame = 15deg       // tilted back to weave
    pose frame = 150mm       // three holes down
  }
  ```

  Two poses of the same type for one assembly are `error[duplicate-name]`,
  as now. A `Length` pose on an assembly that only turns is
  `error[type-mismatch]` (`frame turns; it doesn't slide`). `open` and
  `closed` set every drive of the assembly.
- **Several items on one assembly** must still move alike, drive for
  drive. The rule needs one correction for pivots: two drivers **mirrored
  across the assembly** (a pivot bolt on the left face of the left upright
  and one on the right face of the right upright) move alike when their pins
  lie on one line and their drives match, with the mirrored one's turn read
  the other way. Without this, every pair of pivots is refused. See open
  question 1.

**Why two drives in one kind, and not a carriage assembly holding a frame
assembly?** The loom has no carriage. One bolt both slides between holes
and turns. Splitting it into an outer assembly with no parts of its own and
an inner one would make the beginner invent a part that doesn't exist.
Nesting is still the answer for two *different* pieces of hardware (a
drawer on slides that carries a tilting tray), and Change 5 makes it work.

**Why a cap of one turn and one travel?** It covers every real case found
so far (a loom, an easel, a tilting drafting table, a swivel chair on a
gas lift), and it lets the type of a pose choose the motion with no new
naming. Two turns about different axes (a mirror that tilts and swivels)
is two pieces of hardware, so it is two nested assemblies.

## Change 4: set positions and lift-out members (gap 3)

**`set_at:` on a drive.** `turn(...)` and `travel(...)` take an optional
`set_at:`, which is the values the drive can be locked at. It is either a
list (`[0deg, 15deg, 30deg]`) or `every(d)`, meaning every `d` from `from`
up to `to`.

```
drive: travel(bolt, from: 0mm, to: -(holes_below * hole_pitch), set_at: every(hole_pitch))
```

- A pose must be one of the values: `error[motion]` at the pose otherwise
  (`frame can't be set at 120mm: it sets every 50mm`), with a help line
  giving the nearest values. `open` is the furthest set value within range.
- Every value must lie within `from..to`, and must not be past a `stop`
  (`error[motion]` at the kind).
- A drive without `set_at:` is continuous, as now. A knob bolt clamped by
  friction sets at any angle, and that is the right model for the weaving
  angle.

**`loose: true` on a member.** This reuses the part field of the same name,
which already means "meant to lift out". A loose member is taken out to
change a drive that has `set_at:`. It is not swept while that drive moves.
It is checked wherever the hardware is set: at rest, at every pose and at
every set value. A member carried by a loose member (`with:` it) is loose
too. While a continuous drive moves, loose members stay in and are swept
like any other.

This is what makes a row of holes correct. Moving the frame down a hole
means pulling the bolt, sliding the frame down and putting the bolt back.
Sweeping the bolt through the wood between holes would report a false
collision. Not sweeping the frame at all would miss a real one: the bottom
beam passing through the stretcher on its way down. With `loose`, the frame
is swept and the bolt is not.

## Change 5: one rule for sweeping several motions (gaps 2 and 3)

Today each assembly's single motion is swept from rest and from each
scene's poses, and a motion inside a moving assembly is skipped
(`stave-check/src/motion.rs`, the `nested` test). Replace that with one
rule that covers both two drives and nesting:

> **Motions are made one at a time.** Each motion is swept through its
> whole range while every other motion is held still at each of: closed,
> each value a scene poses it at, and each of its `set_at` values. A motion
> inside a moving assembly is swept with its parent held at each of those
> values. The parent is swept carrying the child, held at the child's own
> such values.

This has two consequences.

- **It is finite and exact.** Each sweep is still one motion about one
  fixed line or along one fixed direction, so the existing exact fast paths
  apply. The loom's turn is swept at each of its 7 heights, and its height
  is swept at rest angle and at each angle a scene names. Nothing samples
  a two-dimensional space.
- **It is honest about what it doesn't check.** Two motions made at the
  same moment are not checked, and a person doesn't make them. A
  continuous motion is checked against the other only at the values the
  design names. A beginner who wants "turned at the lowest height" checked
  writes that scene. The grammar should say this in those words.

The "version 1 limits" paragraph loses "an assembly inside a moving
assembly is not swept".

**A pin turning in its own hole.** "Sweeps see cuts" today takes out a
round hole only when its axis lies in the turning plane (a cup's bore). A
hole whose axis *is* the turn's line was left as wood, so a pivot bolt
turning in its hole in the upright would hit the upright. Add:

> A round hole whose axis lies on the line a body turns about is taken out
> as itself. A round shape turning about its own axis sweeps only itself.

Both statements are exact, not conservative, so this adds no false
negatives.

## Gap 1: why not `turn(about:, by:)` on a part

A placement term that tilts any part looks like the direct answer. It is
the wrong first step, for four reasons.

1. **Faces lose their names.** A part's faces are named by the world
   direction they face (`top`, `left`). A part tilted 15deg has no `top`.
   Every `against`, `flush`, `crossing` and `mount` that reads its faces
   would need new rules.
2. **Joints need angled joinery.** A tilted part jointed to a square one
   is a raked mortise and tenon. Joint kinds, walls, "cuts stay in their
   part", the work cards and the order of operations would all have to
   handle it. That is the real rake-and-splay project, and it is far
   larger than the loom needs.
3. **The loom doesn't need it.** Its frame is jointed only to itself and
   is held to the stand only by bolts. Turning the whole assembly on those
   bolts gives the angle, and every joint inside the frame stays square in
   the frame's own terms.
4. **`about: <pivot line>` is geometry in the language.** The pin of a
   pivot bolt is a woodworking thing, with a diameter, a hole and a knob,
   and it already has a frame. Putting the line there keeps principle 2.

So the sentence at `docs/grammar.md:1210` becomes:

> A part is never placed at an angle. A group of parts turns only as an
> assembly, on its hardware (see [Mechanisms](#mechanisms)), and a scene
> sets the angle. Angled joinery between parts (rake and splay) is out of
> scope.

A frame fixed at one angle, such as an easel with a single setting, is a
turning assembly with `set_at: [15deg]`. If designs later need the *as
built* position to be at an angle rather than closed, the follow-up is a
rest pose on the assembly. It is left out here because nothing needs it
yet.

## Gap 4: the project block

Two parts.

**A `finish` field.** Add to the `project "Title" { }` table:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `finish` | `Text` | no | the finish, in words ("Danish oil, three coats, cut back between coats"). Printed on the cover and as the last step of the build plan |

It stays text for now. A structured finish, one that knows to finish parts
before glue-up or to mask glue surfaces, is a separate design with its own
review.

**Patches can address the project.** Add `project` as a path root in
`set` and `unset`:

| Path | Addresses |
|---|---|
| `project.goal`, `project.finish`, `project.units`, … | a field of the `project { }` block, added when it isn't written yet |
| `project.title` | the title string |

`project` is already a keyword, so it can't clash with an item name. `set
project.title = ""` is refused, because a project needs a title. A field
that isn't in the table is `error[unknown-field]`, as in the file. A
module that is a `library` addresses `library.about` the same way. `replace
project { ... }` replaces the block whole. This is what the MCP `suggest`
and `try_patch` tools need in order to record a goal or a finish.

## The loom, written with these changes

```
kind pivot_bolt : fitting {
  roles {
    piece: Part     // the stand's upright
    into:  Part     // the frame's stile
  }
  param knob_diameter = 50mm
  param projection    = 30mm
  param bolt_diameter = 8mm
  param thread_length = 25mm
  param wall          = 6mm
  /// Spare holes below the bolt, for setting the frame's height.
  param holes_below   = 0
  param hole_pitch    = 50mm

  mount {
    face:   front
    across: open
    along:  open
  }

  /// The bolt: through the upright, into a threaded insert in the stile.
  member bolt {
    with:  into
    loose: true
    shape: cylinder(diameter: bolt_diameter, depth: 0mm..piece.thickness + thread_length)
  }
  /// The knob: tighten to hold the frame at an angle, loosen to turn it.
  member knob {
    with:  bolt
    shape: cylinder(diameter: knob_diameter, depth: -projection..0mm)
  }

  cut bolt_hole:   row(crossing(piece, bolt), pitch: hole_pitch, down: holes_below) { drill(diameter: bolt_diameter + 1mm, depth: through, wall: wall) }
  cut insert_hole: crossing(into, bolt) { drill(diameter: bolt_diameter + 4mm, depth: thread_length, wall: wall) }

  mechanism {
    pins:  depth
    slot bolt, piece from point(along: 0mm, across: 0mm)
                     to   point(along: -(holes_below * hole_pitch), across: 0mm)
    drive: turn(bolt, from: 0deg, to: 360deg)
    drive: travel(bolt, from: 0mm, to: -(holes_below * hole_pitch), set_at: every(hole_pitch))
  }
}

component Frame {
  param stand_l : Part
  param stand_r : Part
  // stile_l, stile_r, top_beam, bottom_beam, tension_bar, shed brackets:
  // as in the current file, placed against stand_l / stand_r.
  hardware pivot_l : pivot_bolt(stand_l, into: stile_l, face: left,  holes_below: 6) {
    place {
      center(stand_l, deep)
      flush(stand_l.top, inset: 60mm)
    }
  }
  hardware pivot_r : pivot_bolt(stand_r, into: stile_r, face: right, holes_below: 6) {
    place {
      center(stand_r, deep)
      flush(stand_r.top, inset: 60mm)
    }
  }
}

assembly frame : Frame {
  stand_l: upright_l
  stand_r: upright_r
}

scene weaving "Weaving" {
  pose frame = 15deg
  pose frame = 150mm
}

scene warping "Winding the warp" {
  pose frame = 90deg
}
```

Stave would then check the frame turning all the way round at each of its
seven heights, the frame sliding between heights with the bolts out, and
the weaving pose. It would cut the row of holes in each upright and put
the bolts and knobs on the hardware list. The cut list and work cards
don't change, because every part is still square in its own frame.

## What changes where

| Area | Change |
|---|---|
| `docs/grammar.md` | Mechanisms (`pins:`, role in an entry, two drives, `set_at:`); How a mechanism is solved (positive turn, step *Set*, freedom count, version-1 limits); Shapes (`loose:` on a member); One collision model (pin in its own hole); Scenes (pose by type, the one-at-a-time rule); Placement (the rake/splay sentence); `project` table (`finish`); Patches (`project.` paths). Lexical grammar: `pins` and `set_at` as soft keywords in a mechanism |
| `stave-lang` | parse the new entries; patch paths rooted at `project` / `library` |
| `stave-schema`, `stave-model` | a mechanism's pin axis, up to two drives, set values; member `loose` |
| `stave-solve` | declare the plane from `pins:`; step *Set*; roles in entries; the mirrored-driver rule |
| `stave-check` | sweep per motion with the others held (remove the `nested` skip); pin-axis holes; poses against `set_at` |
| `stave-output` | `finish` on the cover and as the last plan step |
| `stave-gtk`, `stave-web` | one motion-bar slider per drive. A `set_at` drive snaps to its values |
| `editors/` | highlight `pins`, `set_at` |
| corpus | `examples/tapestry_loom`; diagnostic snapshots for each new error |

No kind name enters a core crate. `pins`, `set_at` and `loose` are language
words, and they work for every kind.

## Open questions to settle before building

1. **Mirrored drivers.** Check whether `assembly_mechanism`'s "move alike"
   test (`same_hand`, `stave-solve/src/mechanism.rs`) already accepts a
   pair of pivots mirrored across the assembly. If it does, the rule in
   Change 3 is a clarification of the spec. If it doesn't, it is new
   behaviour.
2. **A full turn.** `to: 360deg` must be accepted, with 0deg and 360deg
   treated as the same pose for the overlap check. Confirm that the
   dead-point proof has no wrap-around assumption.
3. **Sweep count.** With many scenes and `set_at` values, the number of
   sweeps grows (motions × held values). The loom needs about 7 + 3. If a
   design reaches hundreds, the help should name the cause rather than the
   check silently slowing down.
