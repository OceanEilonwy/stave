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

A fifth gap showed up while writing the loom: its frame stiles sit flat
against the uprights, wood on wood, and would turn rubbing face to face.
Nothing reports it, because two bodies may touch. And a sixth: with the
stile moved 3mm off the upright but no washer in the gap, the design also
checks out, though tightening the knob would pull the gap shut.

This note proposes eight small primitives that cover all six: `pins:`, a
role in a mechanism entry, named drives, `set_at:`, `loose` on a member,
one rule for sweeping several motions, a running clearance worked out
from the wood, and `clamp` for hardware that draws parts together. Each is generic. None names a kind, and each removes a
restriction or a special case rather than adding one. It also
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
accept a pin that runs into the face (gap 2), more than one drive on an assembly
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

## Change 3: named drives (gap 3)

A mechanism may have **any number of drives**, each with a name:

```
drive tilt:   turn(bolt, from: 0deg, to: 360deg)
drive height: travel(bolt, from: 0mm, to: -300mm, set_at: every(50mm))
```

- **Freedom.** The count must come to the number of drives. The formula is
  unchanged: three for each free member, less two for each `pivot` or
  `rail`, less one for each `slot` or `couple`. A pin in a slot that also
  turns, which is a pivot bolt in a slot or a row of holes, is three less
  one: two freedoms, so two drives.
- **No new solving step.** Once a travel along a slot is known, the pin's
  point is known. So "Hang" is reworded from "pinned to a placed member" to
  "**pinned at a known point**" (a pivot on a placed member, or a slot pin
  whose travel is known), and it places the bolt. The steps stay closed
  form.
- **One motion per drive.** The assembly has one motion for each drive,
  and `pose` names it through the assembly:

  ```
  scene weaving "Weaving" {
    pose frame.tilt   = 15deg     // tilted back to weave
    pose frame.height = 150mm     // three holes down
  }
  ```

  A mechanism with one drive may leave it unnamed (`drive: turn(...)`),
  and `pose door = 110deg` works as today, so no existing file changes.
  With two or more drives, each must be named, and a pose that doesn't
  name one is `error[motion]` (`frame moves two ways; write frame.tilt or
  frame.height`). A drive name that clashes with an item of the component
  is `error[duplicate-name]`. `open` and `closed` without a drive name set
  every drive.
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

**Why names, not a cap?** An earlier draft allowed one turn and one
travel and let the pose's type pick between them. That only worked
because of the cap, and the cap was arbitrary: a mirror that tilts and
swivels on one fitting would have needed nesting for no reason. Names
cost one word and have no limit. They are also what the motion bar and
the MCP tools need to label each slider.

## Change 4: set positions and lift-out members (gap 3)

**`set_at:` on a drive.** `turn(...)` and `travel(...)` take an optional
`set_at:`, which is the values the drive can be locked at. It is either a
list (`[0deg, 15deg, 30deg]`) or `every(d)`, meaning every `d` from `from`
up to `to`.

```
drive height: travel(bolt, from: 0mm, to: -(holes_below * hole_pitch), set_at: every(hole_pitch))
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
which already means "meant to lift out". A loose member is **checked
wherever it is set and never swept**: at rest, at every pose and at every
set value. A member carried by a loose member (`with:` it) is loose too.
It has one meaning, whatever the drives are. Nothing is lost by not
sweeping it while a continuous drive moves: a round bolt turning in its
own hole sweeps only itself (Change 5).

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
rule that covers both several drives and nesting:

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

## Change 6: running clearance (gap 5)

**The problem.** In the loom, `frame_stile_l` is placed
`against(upright_l.right)`. When the frame turns, the two faces slide on
each other the whole way round. The sweep allows it, because two bodies
may touch within the 0.05mm contact tolerance. In the shop, the frame
binds as soon as the beech swells. The design is bad and it checks out.

**No annotation is needed.** The stile doesn't need to say that it moves:
with Changes 1 to 3, its hardware already says so. Following principle 6
(store intent, compute results), the needed gap is **worked out** from
what Stave already knows: which bodies move relative to each other, and
how much solid wood swells across its grain (grammar, "Grain and wood
movement": about 1mm per `max_held_width` of width).

**The rule.** For every sweep, take each pair of **wooden** bodies that move
differently (parts, and joint members of wood). Let:

- *gap* be the least distance between them over the sweep, leaving out
  **seating contact**: a closest approach at an end of the range (closed,
  or a `stop`) where the gap opens as the motion leaves it. A door shutting
  onto its stop, or an overlay door resting on the carcass front, is
  seating, not rubbing.
- *swell* be how far the pair can close up with the seasons along the line
  of that closest approach: for each solid board whose grain runs across
  that line, half its size along the line divided by its stock's
  `max_held_width`, times 1mm. A sheet, or a board whose grain runs along
  the line, adds nothing. The two boards' amounts are added.
- *running gap* be the new project field below.

Then:

| Gap | Result |
|---|---|
| *gap* ≤ *swell* | `error[motion]`: it binds. `frame.stile_l rubs upright_l the whole way as frame.tilt turns: the gap is 0mm, and the beech closes up to 0.8mm with the seasons`, help `leave at least 1.8mm between them, against(upright_l.right, gap: 1.8mm), with a washer on the pivot to fill it` |
| *swell* < *gap* < *swell* + *running gap* | `warning[clearance]`: it is tight. `frame.stile_l passes 1.2mm from upright_l as frame.tilt turns; leave 1.8mm (0.8mm for the seasons and a running gap of 1mm)` |
| otherwise | clean |

So the error is kept for a design that *will* fail (touching while it
moves, or closing up with the seasons), and the warning is for one that
might. That is the split you suggested, but it comes from the wood
instead of from a severity the author has to pick.

**Running gap.** Add to the `project "Title" { }` table:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `running_gap` | `Length` | no (default `1mm`) | the least gap, beyond what the wood moves with the seasons, between wooden parts that move past each other. 0mm or more |

**What it doesn't cover, on purpose.**

- **Steel members** keep today's rule: they may touch. Hardware is where a
  design puts a bearing surface (a washer, a slide, a hinge knuckle), and a
  kind's `require` can ask for its own gap, as `slide.clearance` does.
  This is also how the rule stays free of exceptions: wood that has to
  bear on something moving bears on a steel member, and no pair of parts
  is let off.
- **Bodies that move together** (inside one assembly, held by joints) are
  not checked. They don't rub.
- **At rest, nothing changes.** A gap matters only to things that move.

**How it is checked.** The sweep already finds the first contact by
conservative advancement to the contact tolerance. Asking whether two
bodies come within a distance *d* is the same query with *d* in place of
the tolerance, so the error and the warning are each one more pass of the
existing sweep. Seating is read from the distance at the two ends of the
range and its sign as the motion leaves them.

**It finds more than looms.** The same rule catches an inset door whose
hinge-side edge brushes the cabinet side as it swings, a lid that scrapes
its back rail, and a drawer front with no reveal. These are common
beginner mistakes, and none of them is reported today.

**For the loom**, the fix is a washer: a `washer` member of `pivot_bolt`,
`with: piece`, between the upright and the stile, and the stile placed
`against(upright_l.right, gap: washer_thickness)`. The stile then turns
on the washer, steel on wood, with a running gap to the upright.

## Change 7: `clamp`, hardware that draws parts together (gap 6)

**The problem.** With the stile placed `against(upright_l.right, gap:
3mm)` and no washer, the design checks out. It shouldn't. The bolt is
threaded into the stile and slides freely through its hole in the upright.
The knob bears on the upright's outer face. Tighten the knob and it pulls
the stile across the 3mm until it meets the upright. The frame is not
where the drawing puts it, and once it is, it rubs.

**Why nothing catches it.** "Held" asks whether a part is joined to the
rest, not where it ends up: a piece of hardware holds every part its roles
name, whatever the gap. A joint is checked for contact, but hardware only
needs one member to touch each part. Nothing in the language says that
this hardware *pulls*.

**The primitive.** One new entry in a hardware kind:

| Entry | Meaning |
|---|---|
| `clamp a, b along <word>` | tightening draws member `a` and member `b` toward each other along `along`, `across` or `depth` of the kind's frame |

```
clamp knob, bolt along depth
```

**The check.** Take the line along that axis through the clamp, from the
face `a` bears on to the part that carries `b` (here, the knob's face on
the upright to the stile the bolt is threaded into). Every body the line
passes through is in the **stack**: parts, and members of the same piece
of hardware (a washer). The room cut for the clamp itself (the bolt hole)
doesn't count as a gap. Each body in the stack must touch the next, within
the contact tolerance. A gap is `error[no-contact]` at the hardware call:

```
error[no-contact]: pivot_l draws stile_l toward upright_l, but they are 3mm apart with nothing between
   = note: tightening the knob would pull stile_l 3mm toward upright_l
   = help: put a washer or spacer in the gap, or place stile_l against upright_l
```

- It is checked wherever the hardware is tightened: at rest, at every
  pose and at every set value. While a continuous drive moves, the clamp
  is loose, so it isn't checked during the sweep.
- It reads only language words (`clamp`, members, `with:`, the frame
  axes) and never a kind's name, so it covers every fitting that draws
  parts together: knob bolts, bed bolts and barrel nuts, cam locks,
  connector bolts.
- A kind without `clamp` is checked as today. Nothing changes for hinges,
  slides or shelf pins.

**How Changes 6 and 7 work together on the loom.** Each catches one wrong
version, and only the right one passes both:

| Stile placed | Change 6 (running clearance) | Change 7 (clamp) |
|---|---|---|
| against the upright, no washer | error: it binds | clean |
| 3mm off, no washer | clean | error: the knob pulls it shut |
| 3mm off, a 3mm washer between | clean (wood clears wood) | clean (stile on washer on upright) |

No rule names a washer. The washer is simply the only way to satisfy
both.

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
  /// The steel washer the frame turns on, between the upright and the stile.
  param washer_thickness = 3mm
  param washer_diameter  = 30mm
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
    shape: cylinder(diameter: bolt_diameter, depth: 0mm..piece.thickness + washer_thickness + thread_length)
  }
  /// The washer: the stile turns on it, steel on wood, clear of the upright.
  member washer {
    with:  bolt
    shape: cylinder(diameter: washer_diameter, depth: piece.thickness..piece.thickness + washer_thickness)
  }
  /// The knob: tighten to hold the frame at an angle, loosen to turn it.
  member knob {
    with:  bolt
    shape: cylinder(diameter: knob_diameter, depth: -projection..0mm)
  }

  clamp knob, bolt along depth

  cut bolt_hole:   row(crossing(piece, bolt), pitch: hole_pitch, down: holes_below) { drill(diameter: bolt_diameter + 1mm, depth: through, wall: wall) }
  cut insert_hole: crossing(into, bolt) { drill(diameter: bolt_diameter + 4mm, depth: thread_length, wall: wall) }

  mechanism {
    pins:  depth
    slot bolt, piece from point(along: 0mm, across: 0mm)
                     to   point(along: -(holes_below * hole_pitch), across: 0mm)
    drive tilt:   turn(bolt, from: 0deg, to: 360deg)
    drive height: travel(bolt, from: 0mm, to: -(holes_below * hole_pitch), set_at: every(hole_pitch))
  }
}

component Frame {
  param stand_l : Part
  param stand_r : Part
  param middle : Anchor          // the plane the frame is mirrored about
  param frame_height = 1250mm
  param frame_drop   = 0mm      // how far the stile's top sits below the upright's
  param washer       = 3mm

  part stile_l : beech_45 {
    length: frame_height
    width:  70mm
    place {
      orient(length: up, width: back)
      flush(stand_l.top, inset: frame_drop)
      against(stand_l.right, gap: washer)   // was against(upright_l.right): rubs
      center(stand_l, deep)
    }
  }
  part stile_r = mirror(stile_l, about: middle)
  // top_beam, bottom_beam, tension_bar, shed brackets: as in the current file.

  hardware pivot_l : pivot_bolt(stand_l, into: stile_l, face: left,  holes_below: 6, washer_thickness: washer) {
    place {
      center(stand_l, deep)
      flush(stand_l.top, inset: 60mm)
    }
  }
  hardware pivot_r : pivot_bolt(stand_r, into: stile_r, face: right, holes_below: 6, washer_thickness: washer) {
    place {
      center(stand_r, deep)
      flush(stand_r.top, inset: 60mm)
    }
  }
}

assembly frame : Frame {
  stand_l: upright_l
  stand_r: upright_r
  middle:  mid(envelope, across)
}

scene weaving "Weaving" {
  pose frame.tilt   = 15deg
  pose frame.height = 150mm
}

scene warping "Winding the warp" {
  pose frame.tilt = 90deg
}
```

Stave would then check the frame turning all the way round at each of its
seven heights, the frame sliding between heights with the bolts out, and
the weaving pose. It would also check that each stile clears its upright
by the running gap plus what the beech moves. With the old `against`
placement, that check is the error in Change 6. It would cut the row of holes in each upright and put
the bolts and knobs on the hardware list. The cut list and work cards
don't change, because every part is still square in its own frame.

## What changes where

| Area | Change |
|---|---|
| `docs/grammar.md` | Mechanisms (`pins:`, role in an entry, named drives, `set_at:`); How a mechanism is solved (positive turn, "Hang" reworded to a known point, freedom count, version-1 limits); Shapes (`loose:` on a member); Hardware (`clamp a, b along <word>`); One collision model (pin in its own hole); Scenes (`pose a.drive`, the one-at-a-time rule); Checks (running clearance, `warning[clearance]`); Placement (the rake/splay sentence); `project` table (`finish`, `running_gap`); Patches (`project.` paths). Lexical grammar: `pins` and `set_at` as soft keywords in a mechanism, `clamp` at the start of an entry in a hardware kind |
| `stave-lang` | parse the new entries; patch paths rooted at `project` / `library` |
| `stave-schema`, `stave-model` | a mechanism's pin axis, named drives, set values; member `loose` |
| `stave-solve` | declare the plane from `pins:`; "Hang" from a known point; roles in entries; the mirrored-driver rule |
| `stave-check` | sweep per motion with the others held (remove the `nested` skip); pin-axis holes; poses against `set_at`; the running-clearance pass with seating and swell; the clamp stack check |
| `stave-output` | `finish` on the cover and as the last plan step |
| `stave-gtk`, `stave-web` | one motion-bar slider per drive, labelled with its name. A `set_at` drive snaps to its values |
| `editors/` | highlight `pins`, `set_at`, `clamp` |
| corpus | `examples/tapestry_loom`; diagnostic snapshots for each new error |

No kind name enters a core crate. `pins`, `set_at`, `loose`,
`running_gap` and `clamp` are language words, and they work for every kind.

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
4. **Pivots that don't draw.** A plain pin or a shoulder bolt (a cradle
   hung between posts) has no `clamp`, so Change 7 doesn't apply, and the
   part may float along the pin until it rubs. Change 6 catches it only
   where it is drawn touching. Whether a free pivot needs its own primitive
   (a collar, or `clamp` with a stop) is left for its own review.

## Appendix: five more designs with the same fault

To check that Changes 6 and 7 are general and not tuned to the loom, five
other designs that bolt one wooden part beside another were written in
today's Stave. They are in
[`2026-11-22-turning-frames-designs/`](2026-11-22-turning-frames-designs/).
Each uses the same `knob_bolt` fitting as the loom, with a `span` param for
the gap the bolt crosses. Each is drawn in its bad version (a 3mm gap with
nothing in it). The variants were tried with `try_patch` on scratch copies.

| Design | What turns or locks | Pivot or clamp |
|---|---|---|
| `cheval.stave`, a cheval mirror | the mirror frame tilts between two uprights | stile beside upright |
| `stool.stave`, a folding camp stool (drawn folded) | the inner legs swing inside the outer legs | inner leg beside outer leg |
| `drafting_table.stave`, a tilting drafting table | the board's cleats tilt between the legs | cleat beside leg |
| `sawhorse.stave`, an adjustable sawhorse | nothing moves: extensions are locked at a height | extension beside leg |
| `glider.stave`, a porch glider | the hanger arms swing from the posts, and the seat swings on the arms | arm beside post, seat rail beside arm |

### Today's Stave: none of the faults is caught

Run on 2026-11-22 against `main` (7f9343a):

| Variant | cheval | stool | drafting | sawhorse | glider |
|---|---|---|---|---|---|
| 3mm gap, nothing in it | clean | clean | clean | clean | clean |
| touching, no washer | clean | clean | clean | clean | clean |
| 3mm gap, 3mm washer | clean | clean | clean | clean | clean |
| 3mm gap, 1mm washer | | | | | clean |
| 3mm gap, 5mm washer | | | | | `error[overlap]`: `hang_l's washer goes 2mm into arm_l` |

So today Stave can't tell the broken versions from the right one. The one
thing it already catches is a spacer too *thick* for its gap. Change 7 is
the missing other half: a spacer too *thin*, or none at all.

### The proposed rules, applied by hand

These are worked by hand from the rules as written above, because neither
rule is built yet. *Swell* uses the default `max_held_width` of 80mm (none
of the stocks says otherwise). In every design the bolt runs `across`.
Every part beside a bolt has its thickness along `across`, which is
across its grain, so it moves.

| Design | Swell (both parts) | Gap Change 6 needs |
|---|---|---|
| cheval (45mm upright, 45mm stile) | 0.28 + 0.28 = 0.56mm | 1.56mm |
| stool (30mm legs) | 0.19 + 0.19 = 0.38mm | 1.38mm |
| drafting table (45mm leg, 45mm cleat) | 0.56mm | 1.56mm |
| sawhorse | (nothing moves) | none |
| glider, both joints (45mm parts) | 0.56mm | 1.56mm |

| Variant | Change 6 (running clearance) | Change 7 (clamp) | Result |
|---|---|---|---|
| touching, no washer, in the four designs that move | the faces stay at 0mm the whole way through the turn, which is not seating, so `error[motion]`: it binds | clean: knob, part, part touch | **error** |
| touching, sawhorse | not asked: nothing moves | clean | **clean**, and right: it's how a fixed extension is built |
| 3mm gap, nothing in it, all five | clean (3mm > what each needs) | the stack has 3mm of air between the two parts, so `error[no-contact]` | **error** |
| 3mm gap, 1mm washer | clean | 2mm of air between washer and part, so `error[no-contact]` | **error** |
| 3mm gap, 5mm washer | | today's `error[overlap]` | **error** |
| 3mm gap, 3mm washer, all five | clean: the wood is 3mm apart, and the washer is steel | clean: knob, part, washer, part touch | **clean** |

In every design that moves, only the version with a washer that fills the
gap checks out. In the one that doesn't move, closing the gap and adding a
spacer both check out, which matches the shop. Nothing in either rule
names a washer, a stool or a mirror.

Two things the designs taught:

- **The glider has two clamps on one arm,** at the post and at the seat.
  The stack is checked per clamp, so both need a washer. A rule that
  looked only at "the" pivot of an assembly would have missed the second
  one.
- **The sawhorse shows that Change 6 must stay tied to motion.** If
  running clearance were asked of every bolted pair, the correct sawhorse
  (touching) would fail.
