# Checks gaps review (2026-11-17)

Designs that `stave check` passes with no errors and no warnings, but that
cannot be built. Found from a user report (dowels in the clothing cabinet's
top), then widened to every kind in `std/` and every engine check. The
review was read-only; each gap below was reproduced by running
`stave check` (and `stave dump` where noted) on a copy of an example, never
on the repo. Line numbers were read at `main` `64f0e70`.

The owner's direction: close these with **generic concepts that can be
written in `.stave` files**, with as few engine special cases as possible.
Engine checks stay only as backstops for geometry that no kind should be
able to get wrong.

## The report that started it

In `examples/clothing_cabinet/carcass.stave`, set `base` to
`length: cabinet_width * 2` and `top` to `length: cabinet_width * 1.97`.
The top (1773 mm, centred on an 1800 mm base) then covers only 5.5 mm of
each side's 19 mm end. `stave check` says ok. `stave dump`:

```
top     cut drill 4 x diameter 8 depth 15 on thickness start: where side_l meets it, local 0..5.5 x 20..580 x 0..0
side_l  cut drill 4 x diameter 8 depth 15 on thickness end:   where side_l meets it, local 0..19 x 0..560 x 19..19
```

Three faults at once:

1. **The holes do not line up.** `crossing(p, q)` carries a footprint (the
   intersection of the two cut boxes, `crates/stave-solve/src/joints.rs`
   ~1690); `end(p, facing: q)` carries none (`CutPlace::End`,
   `crates/stave-model/src/solved.rs:899`). So `face_extent`
   (`crates/stave-geom/src/solid.rs:1051`) falls back to the whole end and
   `hole_centres` (`solid.rs:1476`) centres the side's holes on its 19 mm
   end and the top's on the 5.5 mm strip: 6.75 mm apart.
2. **The top's holes break out.** An 8 mm hole centred on a 5.5 mm strip
   runs 1.25 mm past the end of the board.
3. **Nothing checks it.** `std/joinery/dowel.stave` only requires
   `diameter <= piece.thickness / 2` and that the hole stops short of the
   far face. Nothing relates the dowel to how much the parts touch, nothing
   checks that a hole stays inside its board, and the pins are only counted
   (`uses:` is "counted, never placed", `docs/grammar.md` line 787), so
   they are never collision-checked and joint focus has nothing to draw —
   which is why the dowels do not show in "how it works"
   (`crates/stave-app/src/focus.rs` and `crates/stave-scene/src/focus.rs`
   never read the fastener list).

### A first proposal, and what the review changed

The first proposal was: (a) drill both parts from one shared contact patch,
(b) an engine check that every cut stays inside its board with a minimum
wall, (c) place pins and screws and collision-check them like the cup
hinge's steel, (d) optionally let a kind's `require` read the contact size.

The review's corrections:

- **(a) hides the bug instead of fixing it.** Both holes would sit 2.75 mm
  from the top's end, which is also 2.75 mm from the side's inside face, so
  an 8 mm hole breaks out of both parts. Changing `end` to use the patch
  would also move tenons, which should stay centred on the part.
- **(a) and (c) are one mechanism.** If the pin is a member placed on the
  contact and both cuts are `crossing(role, pin)`, the holes align by
  construction and the pin is drawn and overlap-checked. The precedent
  already exists: `std/hardware/pulls.stave` declares `member screw` with a
  `shape:` and `cut hole: crossing(piece, screw) { drill(...) }`.
- **(b) as stated would false-positive.** `notch`, `slot`, `tenon` and
  rabbets leave the part on purpose. Split it: "a closed cut leaves the
  part through a face it was not aimed at" is geometry (engine);
  "minimum wall" is woodworking judgement (std `require`).
- **(c) should not extend `steel_in_wood`** (`crates/stave-geom/src/sweep.rs`),
  which the special-cases audit already marks for generalising (GE3). Static
  pins only need the layer-4 rest overlap check. It reverses a written
  spec rule, so the spec changes in the same change.
- **(d) is not optional.** It is the cheapest piece and gives the beginner
  the right message.

## Every gap found, most likely first

Each was reproduced with 0 errors, 0 warnings. "Dump" means the bad
geometry was read in `stave dump`.

### 1. A part that nothing holds (any project)

- `side_table/table.stave`: delete `joint slat_screws ...` (slats loose on
  the stretchers) or `joint top_screws ...` (top attached to nothing).
- Why missed: joints are only checked where written; nothing asks whether
  every part is joined to the rest. A `fit` length catches some cases by
  luck (deleting the bookshelf's `kick_dowels` gives `fit-undecided`).
- Closed by: **held** (below).

### 2. A joint size bigger than the wood (mortise_tenon, round_tenon, groove)

- `side_table`: `thickness: 30mm` on `stretcher_tenons`. Dump: tenon
  25 x 30 x 30 on a 50 x 19 stretcher, and a 30 mm pocket 0.5 mm out of the
  leg's face.
- `clothing_cabinet/carcass.stave`: `foot_tenons` with `diameter: 80mm`.
  Dump: an 80 mm spigot on a 70 mm foot. Same in `stool` with
  `diameter: 40mm` on a leg 34 mm at the top.
- `box/box.stave`: `bottom_groove` with `depth: 12mm` (walls are 12 mm).
  Dump: `notch depth 12 ... 6..12 x 0..12`, each wall cut in two.
  `depth: 40mm` also passes.
- Why missed: `mortise_tenon` has no rule tying `thickness` to the board,
  `round_tenon` none tying `diameter` to the piece, `groove` no "not too
  deep" rule (`dado` and `rabbet` have one). The engine has no rule that a
  tenon only removes wood, or that a stopped cut leaves wood behind it.
- Closed by: std requires, **cuts only remove wood**, **stopped cuts leave
  wood behind**.

### 3. Too many or too fat fasteners (dowel, screw)

- `bookshelf`: `kick_dowels` with `count: 9`. Dump: 9 x 8 mm holes along a
  70 mm kick, 7 mm apart, running into each other.
- `wall_shelf`: `shelf_screws` with `diameter: 30mm`. Dump: a 22.5 mm pilot
  hole in the arm's 19 mm edge.
- `wall_shelf` `count: 60` (60 screws on a 140 mm arm) and `side_table`
  `top_screws` `count: 40` also pass (not dumped).
- Why missed: nothing relates count or diameter to the contact; fasteners
  are not bodies.
- Closed by: `contact()` requires in `dowel` and `screw`; the breakout part
  of `wall()` catches the fat screw. Joint members alone do not catch
  crowded pins, because a kind's own members are not checked against each
  other.

### 4. A cut with no wood beside it (socket, mortise_tenon, two joints)

- `carcass.stave`: rod `flush(side_l.front)`. Dump: the 28 mm socket at
  `0..28` of the side's width, open at its front edge.
- `carcass.stave`: rod `against(shelf.bottom, gap: 0mm)`. Dump: socket
  `1455..1483`, shelf dado `1483..1502`, nothing between them.
- `side_table`: `apron_tenons` with `shoulder: 0mm`. Dump: pocket
  `491..581` on a 581 mm leg, open at the top.
- `side_table`: `setback = 0mm` with `thickness: 18mm` on
  `stretcher_tenons` leaves a 0.5 mm mortise wall (by arithmetic).
- `stool`: delete the hand-written `rungs_miss` and set
  `high_rung = 172mm`; the two rung holes sit edge to edge (by arithmetic).
  That the example needs `rungs_miss` at all shows the gap in the language.
- Why missed: overlap only sees wood against wood.
- Closed by: **`wall()`**.

### 5. Hardware deeper than its board (euro_hinge, wide_hinge_165)

- `clothing_cabinet/doors.stave`: `frame: maple_12`. Dump: a 35 mm cup
  drilled 12.8 mm into a 12 mm stile, out through the door's front.
- Why missed: the kind has no rule on `piece.thickness`, and steel in air is
  not an overlap. The only cup rule (`stile_holds_cup`) lives in the
  example's door component, not the kind.
- Closed by: a `require` in the kind; **stopped cuts leave wood behind**.

### 6. Boards longer than you can buy (stock)

- `bookshelf`: `param case_height = 9000mm within 1000mm..9000mm`. Nine-metre
  sides pass.
- Why missed: `max_length` is optional in the spec and no example sets it.
- Closed by: require `max_length` for solid stock, or warn when it is absent.

### Already caught (often by a neighbouring check, not a rule about the real fault)

Meeting tenons too long; a slide longer than the case; a hinge off the end
of a door; a foot or leg hanging off its base; a butt hinge leaf wider than
the wall (the box's own require); a knob over the panel groove; a lip 2 mm
above a housing (`error[order]`).

### Suspected, not reproduced

- Dados at half depth on both faces of one divider meeting in the middle.
- A hole from one joint running into a hole from another (a foot tenon under
  a side's dowels). Joint members would catch it.
- Rung sockets in a thin turned leg: `socket`'s rule uses the square blank,
  not the turned diameter.
- A fault that exists only at other values in a param's range: `contact()`
  and `wall()` need the range layer too.
- Wood movement (a wide solid top screwed down tight, a solid panel glued in
  its groove) and strength (shelf sag, hinge count for door weight, screws
  in end grain). These need grain direction and species as values in
  `require`; a separate design.

## The concepts

| Concept | Where | What it is | Closes |
|---|---|---|---|
| `contact(a, b)` | language | The patch where two parts touch, with `.length` (longer side) and `.width` (shorter side). `error[no-contact]` if they do not touch. Usable in `require` and as a member's `on:`. | 3, the report |
| Joint members | language | `member` in joint kinds, placed `on:` a patch with `count:` and `spaced:`; cuts become `crossing(role, member)`. `uses:` stays only for unplaced consumables; the hardware list counts placed members. | the report's alignment, focus drawing, holes running into other joints' holes |
| `wall(cut)` | language + engine | The least wood between a named cut and any face or other cut of the same part, usable in `require`. Engine backstop: `wall > 0` for every closed cut (`drill`, `pocket`, `socket`), which subsumes a separate breakout check. | 4, parts of 2, 3, 5 |
| Cuts only remove wood | engine | A `tenon` or `spigot` larger than its part is an error. | tenon cases in 2 |
| Stopped cuts leave wood behind | engine | A depth given as a length must be less than the part there; only `through` goes through. | groove in 2, cup in 5 |
| Held | engine, opt-out in text | `warning[loose-part]`: every part is reachable from the ground part through joints or hardware. A part or kind can opt out (a loose shelf, a tray). | 1 |

### `dowel.stave` after stages 1 and 2

```
kind dowel : joint {
  param diameter = 8mm
  param count = 2
  param depth = 15mm
  /// Wood left each side of a hole.
  param wall = 4mm

  require pin_fits_piece : diameter <= piece.thickness / 2 "the dowel is too thick for this board"
  require hole_stops     : depth <= extent(into, length, of: piece) - 3mm "the dowel hole would come through the other side"
  require seat_wide      : contact(piece, into).width >= diameter + 2 * wall "the parts overlap too little for these dowels"
  require seat_long      : contact(piece, into).length >= count * (diameter + wall) + wall "too many dowels for this joint"

  insert: [push(piece, along: length)]

  /// The pins: glued into matching holes in both parts.
  member pin : dowel_pin(diameter: diameter, length: 2 * depth) {
    on:     contact(piece, into)
    spaced: evenly
    count:  count
    shape:  cylinder(diameter: diameter, depth: -depth..depth)
  }
  cut crossing(piece, pin) { drill(diameter: diameter, depth: depth) }
  cut crossing(into, pin)  { drill(diameter: diameter, depth: depth) }
}
```

What a beginner sees for the reported edit:

```
error[require]: 'seat_wide' fails: the parts overlap too little for these dowels
  --> carcass.stave:<line of top_dowels>
   = note: for side_l and top in top_dowels: the rule measures 5.5mm and needs 16mm
   = help: top covers only 5.5mm of side_l's 19mm end; make top longer or base shorter so it covers the whole end
```

## Staged plan

1. **`contact()` and the std requires.** Grammar, `stave-schema` (builtin
   and a `Patch` value), `stave-solve` (evaluate from the existing cut-box
   intersection; also in the range layer), std (`dowel`, `screw` contact
   rules; `groove` not too deep; `mortise_tenon` and `round_tenon` tenon
   smaller than the piece; both cup hinges' cup depth below the piece's
   thickness), and `tests/broken` snapshots for each gap above that the
   stage closes. The reported edit becomes an error. Overlaps that pass
   `seat_wide` can still misalign by up to 1.5 mm until stage 2.
2. **Joint members.** Spec (reverse "counted, never placed" for placed
   members), `stave-lang` if the parser rejects them, `stave-schema`,
   `stave-model` (member bodies on joints; `CutPlace::Member` generalised
   beyond `HardwareId`), `stave-solve` (layout on the patch, cuts from
   members), `stave-check` (bodies in the overlap check), std. Decide how a
   screw's thread may bite into its pilot hole (a declared core diameter, or
   a shape at pilot size).
3. **Engine backstops.** `wall()` with its `> 0` backstop, cuts only remove
   wood, stopped cuts leave wood behind; then `held`. `stave-geom` measures,
   `stave-check` reports at the joint line. Remove the stool's hand-written
   `rungs_miss` once `wall()` covers it.
4. **Show it.** `stave-scene` and `stave-app` focus draw member bodies;
   `stave dump` prints them; the hardware list counts members. Retire
   `drill(count:)` spreading in `hole_centres`; update the special-cases
   audit's S4 and SO18 entries.
5. **Stock lengths.** Require or warn on missing `max_length` for solid
   stock.

## Resolution: stage 1

- **`contact(a, b)`** is a built-in returning a typed `Patch`
  (`TypeName::Patch`, `stave_schema::functions::PatchSize`,
  `stave_model::value::Patch`), read as `.length` and `.width`, and usable
  as `of:` in `extent` (`length`, `width` along the patch, `thickness`
  square to it). It is evaluated in `crates/stave-solve/src/contact.rs`
  from the intersection of the two parts' **drawn** boxes: the cut boxes
  are not final while a joint's rules run (joints are still adding their
  extensions), and a tenon reaching into its mortise is not where the
  parts touch. A turned part touching with an end touches over its
  profile's diameter at that end. Parts that don't touch are
  `error[no-contact]`. Spec: grammar, "Built-in functions".
- **Messages.** Every failed kind rule that compares now notes `the rule
  measures X and needs at least Y` (any comparison, not only `==`); other
  pairs of the same joint breaking the same rule are `the same for` notes;
  a rule reading `contact()` where one part covers only some of the other
  says how much (`top covers only 5.5mm of side_l's 19mm end; move or
  resize top so it covers the whole end`); otherwise the general help names
  the params the rule reads. The note reads "needs at least 16mm", not the
  review's "needs 16mm".
- **std rules:** `dowel` (`wall`, `seat_wide`, `seat_long` as above),
  `screw` (`wall = 6mm`, `gap = 15mm`, `seat_wide`, `seat_long`), `groove`
  (`not_too_deep`: half the board measured square to the patch, so a groove
  in a frame's edge is held to the stile's width, not its thickness),
  `mortise_tenon` (`tenon_thin`: at most half the rail; `mortise_fits`:
  the tenon's width on the patch's length), `round_tenon` (`shoulder =
  3mm`, `tenon_fits` on the patch's width, the turned end's diameter),
  `euro_hinge` and `wide_hinge_165` (`cup_stops`: 12.8mm + 2mm, the maker's
  tables starting at 15mm doors).
- **The range layer** does not exist yet (`require-range` is pending), so
  there was nothing to extend; `broken/contact_range` is pending with it,
  and the grammar says how `contact()` fits z3.
- **Not closed here:** joint members, `wall()`, the engine backstops,
  `held`, focus drawing (stages 2 to 5). The reported cabinet's holes still
  misalign by up to 1.5mm on overlaps that pass `seat_wide`. The `tenon_thin`
  rule catches the side table's 30mm tenon, but an 18mm tenon with a 0mm
  setback (a 0.5mm mortise wall) still needs `wall()`.

## Resolution: stage 2

- **Joint members** (grammar, "Joint members"). A joint kind writes
  `member pin : dowel_pin(...) { on: contact(piece, into), count:,
  spaced:, shape: ... }`. The `: fastener` follows the `part name : stock`
  and `joint name : kind(...)` convention and is optional; the parser
  takes any expression there. `on:` is a `Patch` (required), `count:` a
  `Count` (default 1), `spaced:` a `Spacing` (default `evenly`): the same
  closed vocabulary parts use (`evenly`, `ends`, `ends(inset:)`,
  `pitch(d)`, `gap(d)`), so a new spacing is one variant of
  `stave_model::value::Spacing` for parts and members alike. A joint
  member has no `with:` or `mount`, always has a body, and only a joint's
  member may name a fastener. Each copy has the hardware frame's words:
  `along` the patch's length, `across` its width, `depth` square to it
  from `a` into `b` of `contact(a, b)` (`Patch::into_second`), origin on
  the contact plane in the middle of the width. Copies are as long as
  their shapes along `along`, so `evenly` leaves equal *wood* between and
  beyond them, which is what `seat_long` measures.
- **Types.** `Carrier::Contact(Box<LayoutDef>)` (`stave_model::defs`) is
  what carries a joint member; `BodyDef::item` is what it is;
  `HardwareCutDef` became `MemberCutDef`, shared by hardware kinds and
  `JointKindDef::member_cuts`. In the solved design, `BodyOwner` (a piece
  of hardware, or a joint pair) replaces `BodyRef::hardware`, and
  `CutPlace::Member { owner, body, footprint, shape }` carries the shape's
  whole box besides the footprint. `JointPair::members` holds each
  member as one `Body` with a shape per copy.
- **Where it lives.** Layout: `crates/stave-solve/src/members.rs`
  (`offsets`, `lay_out_members`, exact `ExactLength` arithmetic, rounded
  once in `build::joint_body`). Cuts: `build.rs` (`finish_joint`, the
  shared `member_cut`), one per shape per copy, on the face
  `hardware::crossing_face` picks. Geometry: `stave-geom`'s `face_extent`
  measures a member cut on the shape, not the clipped footprint, so a
  hole is centred on the pin even where the patch is narrower than one
  part's end. Mirrored assemblies reflect a joint's members and cut the
  parts outside them again (`JointWork::member_cuts`), as hardware does.
  Checks: `stave-check` adds every joint pair's members as bodies in the
  layer-4 rest overlap check (not `steel_in_wood`); the "a kind's own
  members aren't checked against each other" rule is now per owner;
  `explain::joint_member` words the help.
- **Screws: a shape at pilot size.** The screw's shape is its core, the
  size of its pilot hole (`param pilot = diameter * 3 / 4`; `shape:
  cylinder(diameter: pilot, depth: -piece.thickness..length -
  piece.thickness)`), which the pilot hole holds exactly and the
  clearance hole holds loosely. A declared core diameter would be a field
  the overlap check reads for fasteners only, an engine special case; a
  shape is the one thing the check already reads for every body, so the
  rule "a shape is the room a piece takes once it is in" covers screws
  without a new concept. What it gives up: the shank's real 4mm in the
  piece's 4.5mm hole isn't modelled (it is thinner there, never thicker).
- **std.** `dowel` and `screw` are members now, with the stage 1 rules
  unchanged; no other `std/joinery` kind used `uses:` or `drill(count:)`
  for something placed. Nothing in `std/`, `examples/` or `tests/` spreads
  holes with `drill(count:)` any more; it still works (two spec tests use
  it) until stage 4 retires it. The spec described its spreading as
  `spaced(evenly)`, but the code steps centres equally (less wood between
  holes than at the ends); the spec now says what the code does.
- **The alignment check.** The cabinet with `base` at
  `cabinet_width * 2` and `top` at `cabinet_width * 2 - 4mm` (the top,
  centred, covers 17mm of each side's 19mm end; `seat_wide` holds) checks
  clean, and every hole in a side's top end has a hole in the top with
  the same centre: side_l's at across 10.5mm, deep 109.6, 223.2, 336.8,
  450.4mm, up 1666..1681mm, the top's at the same centres, up
  1681..1696mm; side_r's at across 1789.5mm (`app/members::
  the_cabinets_dowel_holes_line_up_where_the_top_covers_only_part_of_the_sides`).
  Before, they were up to 1.5mm apart on such overlaps.
- **New diagnostics** (`tests/broken/pin_past_its_hole`,
  `dowel_into_tenon`, `dowels_cross_dowels`), all `error[overlap]` at the
  joint's call: `shelf_pegs's peg and side overlap` (help: `the peg of
  'shelf_pegs' goes 5mm past the bottom of its hole in side; make the
  hole as deep as the peg reaches, or the peg 5mm shorter`);
  `side_dowels's pin and foot overlap` (help: `the pin of 'side_dowels'
  goes 8mm into foot, where no hole is drilled for it; move foot or the
  joint so the pin misses it, or make the pin shorter`); `side_dowels's
  pin and cap_dowels's pin overlap` (help: `the pin of 'side_dowels' and
  the pin of 'cap_dowels' run 8mm into each other inside base; move one
  of the joints, or use fewer or shorter ones`). Also `pegs: its pegs run
  10mm past the end of where its parts touch` (`pitch`/`gap` overrun),
  `<joint>: its <member> doesn't reach <part>` (`no-contact`), a
  not-supported warning for members between boards, and the resolver's
  `member 'peg' is missing 'on'`, `member 'peg' has no field 'with'`,
  `member 'screw' of a hardware kind can't name a fastener`.
- **Showing it.** `stave dump` names the member a cut is for (`where
  top_dowels's pin meets it`; hardware cuts now say `lid.lift_knob's
  screw` too) and lists each pair's member shapes under the joint, so all
  six dump snapshots changed. The pins are drawable from the solved
  design alone (`stave_geom::body(&pair.members[i])`, tested); wiring them
  into `stave-scene`'s graph and joint focus needs a scene `Item` for a
  joint's member and its handling in `stave-app`'s view, session and
  problems, which is stage 4's work and was left out here.
- **Left for stages 3 and 4.** A pin running into another joint's empty
  hole (no pin or tenon in it) is air, not overlap: `wall()` (stage 3).
  Crowded pins of one pair are still only the `seat_long` rule's to
  catch. Stage 4: scene and focus drawing of joint members, retiring
  `drill(count:)` spreading in `hole_centres`, and the special-cases
  audit's S4 and SO18 (still justified: the hardware list reads a
  fastener's `diameter` and `length`, now for member items too).

### Merged with "generalise kinds"

Stage 2 was merged with the generalise-kinds work (named cuts, no member
vocabulary, the standing rule). What changed in stage 2 for it:

- `dowel`'s and `screw`'s member cuts are named, keeping the names that
  work gave their old cuts: `cut end_hole: crossing(piece, pin)`, `cut
  hole: crossing(into, pin)`, `cut clearance_hole: crossing(piece,
  screw)`, `cut pilot_hole: crossing(into, screw)`;
  `tests/broken/pin_past_its_hole` and `app/members` name theirs too, so
  none warns `unnamed-cut`.
- Members are checked by `check_members`, which has no word table: a
  joint kind may have members (each with a body), a fastener or tool kind
  none. A joint member without a body is still `member 'pin' is missing
  'shape'`; the grammar's old "Hardware members" table is gone and the
  joint-member rules it held are under "Joint members".
- Helps no longer name a kind (`dowel_pin`, `screws`), for the standing
  rule.

## Resolution: stage 3

- **`wall(cut)`** (grammar, "Walls") is a built-in `Length`: the least
  wood between one of the kind's named cuts and the part's faces and other
  cuts, whichever joint or hardware makes them. It names a cut of its own
  kind, is read only in a joint or hardware kind's `require`, and the rule
  is one comparison with `wall(cut)` alone on one side, so everything but
  the wall is worked out when the kind's rules run. Resolver:
  `RExprKind::Wall(CutName)` (`stave-model/src/resolve/expr.rs`,
  `kinds.rs`); solver: `stave-solve/src/walls.rs` keeps each such rule,
  with its message and the other side rounded once, as a
  `solved::WallRule` in `SolvedDesign::wall_rules`; the fit check measures
  and reports it at the joint's or hardware's call, as `error[require]`
  with the kind's rule as a second label, `the rule measures X and needs
  at least Y`, `the same for` notes and a help that says where the least
  wood is (`the socket in side_l leaves 0mm of wood at the dado that
  shelf_dados cuts in side_l; ...`, or `runs 8mm past ...`).
- **Where it lives.** Measurement: `stave-geom/src/wall.rs` (new;
  `Walls::of(part, design, solid)`, `wall`, `too_deep`, `oversize`). It
  reuses where the solid puts a cut (`face_extent`, `place_centre`,
  `depth_range`, `entering_axes`, `slot_run`, `Profile::diameter_at`
  became `pub(crate)` in `solid.rs`; nothing else there changed) and keeps
  its own copy of the drill spreading until stage 4 retires
  `drill(count:)`. Exact between boxes, to flat faces and between parallel
  holes in line; a rounded-down square root on a slant; alternating
  projections in `f64` for two holes square to each other. Checks:
  `stave-check/src/walls.rs` (wall rules and the three backstops) and
  `held.rs`.
- **Engine backstops** (grammar, "Cuts stay in their part"), at the
  joint's or hardware's call: `error[cut-size]` for a tenon or spigot
  bigger than its part (`rail_stubs: its stub on rail is 25mm thick, but
  rail is only 19mm thick`; `... is 80mm across, but foot is only 70mm
  across at that end`) and for a depth that doesn't stop inside the part
  (`bottom_slots: its slot in front is 12mm deep, but front is only 12mm
  thick there`, help ending `or write depth: through if it should go
  right through`); `error[wall]` for a `drill` or `pocket` with no wood
  between it and a face it isn't aimed at (`side_pegs: its peg hole in
  base breaks out of the left end of base`) or a closed cut of another
  pair or piece of hardware (`post_a_bore: its bore in base runs into the
  bore of post_b_bore`; `meets` when they only touch). Gap (d), a dowel
  or screw into another joint's empty hole, is caught by the dowel's and
  screw's own `wall()` rules, or by the backstop for a kind without one.
- **Which error wins.** A kind's own rule first: one that fails in the
  solver stops the design, so std's `tenon_thin`, `tenon_fits` and
  `not_too_deep` still speak for the review's tenon, spigot and groove
  cases, alone (`app/walls::a_kinds_own_rule_speaks_before_the_backstop`);
  a cut too big or too deep that a `wall()` rule of its own kind measures
  waits for that rule. A cut too big or too deep marks its part, so the
  overlap check doesn't report the parts that no longer fit. An overlap
  beats a wall: a wall rule or backstop is not reported for a part or
  member that already overlaps (a pin running into another joint's tenon
  stays one `overlap`, as stage 2 wrote it), because the overlap names the
  things that collide. Two closed cuts that run into each other are one
  error, for the first.
- **Two false positives found and fixed in the check**, not the examples:
  the first backstop held closed cuts to every other cut, which flagged
  the cabinet doors' stub-tenon pockets running into their panel grooves.
  A cut inside another is now not a wall to it, and the backstop compares
  closed cuts only with other closed cuts: how near a hole may come to a
  groove or dado is a kind's judgement, written with `wall()`.
- **One kind fixed, found by the backstop:** `cope_and_stick` cut its
  sticking as a closed `pocket` that, with the rails flush with the ends
  of the stiles, broke out of the stile's end. Cope and stick runs the
  groove the whole stile, out of both ends; the kind now cuts it as a
  `slot`.
- **Held** (grammar, "Held"): `warning[loose-part]` for every group of
  parts that joints and hardware don't join to the ground part, once, at
  its first part: `slat[0] is loose: no joint or hardware joins it to top
  or to anything joined to top`, or `leg[front_left], leg[front_right],
  leg[back_left], leg[back_right] and 13 more are joined only to each
  other: ...`, with a help that names the held part it touches most (`join
  slat[0] to stretcher_front, which it touches, with a joint or hardware,
  or write loose: true in the part if it is meant to lift out`). Hardware
  holds its piece, its roles' parts, the parts carrying its members and
  the parts its members touch (a slide's runner on a region). The opt-out
  is the part field **`loose: true`**: a group with a loose part lifts out
  together. It is asked only of a design with no errors.
- **std.** `wall()` replaces the ad-hoc "stops 3mm short of the far side"
  rules: `dowel` `hole_wall: wall(hole) >= wall` (for `hole_stops`),
  `round_tenon` `hole_wall: wall(hole) >= wall` with `param wall = 3mm`
  (for `hole_stops`), `mortise_tenon` `mortise_wall: wall(mortise) >=
  wall` with `param wall = 6mm` (for `mortise_stops`), `screw`
  `pilot_wall: wall(pilot_hole) >= 3mm` (for `stays_inside`, and its
  `bite` param, which only that rule read, is gone). `socket` gains
  `socket_wall: wall(socket) >= wall`, `param wall = 6mm`. New
  `glued_face` (two parts glued face to face, nothing cut, no insert step).
- **Examples.** All check clean. `stool`: `rungs_miss` removed; socket's
  wall rule covers it. At `high_rung = 172mm` the rung holes are 2mm
  apart on the slant between their bottom corners, not edge to edge (the
  sockets are 15mm deep in a 40mm leg, so they don't reach each other's
  centre lines), and `socket_wall` fails there (`broken/rung_holes_too_close`).
  Real bugs the new checks found: `side_table`'s cleats were glued to the
  aprons only in a note, so the whole base was loose from the top (now
  `joint cleat_glue : glued_face(...)`); its 25mm apron tenons left 0.47mm
  between the two mortises in each leg (now 20mm, 7.5mm of wood);
  `clothing_cabinet`'s drawer had four front screws, one 1.1mm from a bar
  pull's bolt hole (now two, clear of both the bar pull and the
  nightstand's centred knob). `joint_samples`' two back samples stand
  alone on the bench and say `loose: true`. Test rigs in `tests/designs`
  and `tests/broken` that left a part unjoined got a joint.
- **Tests.** New `tests/broken`: `socket_at_the_edge`,
  `socket_against_dado`, `mortise_open_at_the_top`, `mortise_wall_too_thin`
  (the review's 0.5mm wall, reached with a 0.5mm shoulder: its own route,
  an 18mm tenon at a 0mm setback, is `tenon_thin` since stage 1),
  `rung_holes_too_close`, `top_held_by_nothing`, `slats_held_by_nothing`,
  `hole_breaks_out`, `holes_run_together`, `dowel_into_empty_hole`,
  `tenon_bigger_than_part`, `groove_through_the_wall`, each with a snapshot
  in `app/walls`, which also pins `wall()`'s exactness and its resolver
  errors and the held opt-out; unit tests in `geom/wall` and
  `solve/walls`. The app test harness leaves `loose-part` out of its
  fragment projects (`common::open_files`; `open_files_with_every_warning`
  keeps it), and `common::solved` allows `wall` and `cut-size` as fit
  errors.
- **Not closed here.** The range layer doesn't measure walls (still no
  z3). Dovetails, outlines, angled slots and the cuts between boards aren't
  measured: a rule about one is left out with `warning[not-supported]`.
  `stave dump` lists requires at the current values, so the wall rules
  (measured later, by the check) no longer appear in its REQUIRES list.
  Wood movement and strength remain a separate design.
