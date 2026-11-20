# Grain and wood movement: the three gaps

A follow-up to `2026-11-20-grain-and-wood-movement.md`. Audit finding 89
was left partly fixed with three gaps: one 80mm limit for every wood,
glue inside a joint's cuts not measured, and slots counted as room to
move whichever way they run. The owner asked for all three to be closed.
This note records what was decided for each; the grammar's "Grain and
wood movement" and `stock name { }` sections hold the rules.

## 1. Each wood's own limit: `max_held_width`

The owner asked for "a parameter to each type of wood you add that
defaults to 80mm". The wood a person adds is a `stock` declaration (the
species is a closed word list with no declaration of its own, and a
stock is where the other sizes a board comes in are written), so the
field is a stock field.

- **The name.** `max_held_width`: the widest a board may be *held fast*
  across its grain. "Held fast" is the phrase every grain message already
  uses for glue and tight fixings, and the `max_` prefix sits with
  `max_width` and `max_length`, the other limits of what a board of this
  stock will stand. Names that say how it is computed (`movement_span`,
  `shrinkage`) were rejected: a beginner doesn't know what a span or a
  shrinkage figure is, but can read "the most it may be held".
- **The default is 80mm**, the number the check used before: about where
  a season's movement in flat-sawn oak reaches a millimetre. It is not a
  species table. How much a board moves depends on the species and on
  how it was sawn (flat or quarter), which Stave doesn't know, so a table
  would claim more precision than the check has. A person who knows their
  wood writes the number.
- **Solid only.** Plywood and MDF don't move across a grain and a dowel
  is too narrow to, and the check never measures them. Written on such a
  stock the field is `error[unknown-field]` with a help that says why, as
  `rough_length` is on a sheet: a value that would be silently ignored is
  a mistake worth hearing about.
- **More than 0mm.** Zero or less is `error[out-of-range]` at the field,
  as the stock's other sizes are. A limit of zero would warn about every
  glued joint, which is not a design anyone means.
- **Which limit counts.** The board that moves is measured across its own
  grain against its own stock's limit. The other part only decides
  whether it holds that board still (a sheet, or wood whose grain runs
  the way the board moves). When both parts are solid boards that move,
  each is measured against its own limit: a stable wood glued to a
  restless one can't make the restless one stable.
- **Messages.** The width measured stays in the message; the limit, and
  where it comes from, is in the help: `glue only a strip no wider than
  60mm across the grain of top (the max_held_width of beech), ...`, so
  the reader sees both numbers and which stock to look at.
- **std and the examples stay on the default.** Their woods are white
  oak, hard maple, ash, cherry, walnut, pine and hinoki. Oak is the wood
  the default is set for and is among the most restless; the others move
  as much or less, so 80mm is cautious for all of them, and nothing in
  the corpus is near enough to the limit for a looser figure to change a
  result. A figure per wood (pine at perhaps 140mm, from its tangential
  shrinkage of about 6% against white oak's 10.5%) would be a guess
  presented as a fact in a file people copy from, so none was written.

## 2. Glue in a joint's cuts

The first pass measured only glue on the patch where two parts touch,
because the patch is not where a mortise and tenon's glue is: measuring
it would call every apron's end-on meeting with its leg end grain. The
glue in a cut is where one part houses the other, so that is what is
measured now.

- **Where the glue is.** The overlap of the two parts' *cut* boxes (the
  cut box includes what a joint's `adds:` lengthens a part by, which is
  what reaches into the housing; the drawn boxes only touch). Across the
  moving board's grain the overlap is narrowed by a `tenon` or `spigot`
  cut on the board for the pair, and by a `pocket` cut for it in the
  other part: the tenon's cheeks are as wide as the tenon, not as the
  rail. Notches, slots, dovetails and the rest are taken at the overlap.
  Parts whose boxes only touch house nothing; a member's hole (a `drill`,
  or anything on `crossing(p, member)`) is the member's glue, and the
  member is already measured where it is.
- **The same rule and limit as glue on the patch**: the other part must
  hold still the way the board moves (a sheet, or wood whose grain runs
  that way), over more than the board's `max_held_width`. A shelf in a
  dado in a side whose grain runs up and down is not warned: side and
  shelf both move front to back. A plywood shelf glued in a solid side's
  dado is.
- **A message of its own**, so the reader knows the glue is in a cut and
  what to change: `tenons: apron is glued across its grain to leg over
  130mm in the mortise, ...`, help `leave the glue out of tenons (glue:
  false) so apron can move in the mortise, or make the mortise no wider
  than 80mm across the grain of apron (the max_held_width of oak)`.
- **Not a fixing for "held fast".** The first draft also counted glue in
  a cut as a fixing for the held-fast rule. Against the corpus it called
  the stool (turned legs round-tenoned into a solid seat, joined by
  rungs) held fast 220mm apart, and two bookcases held between a kick
  rail and a plywood back joined only through the other side, which
  moves the same way. A chair's or stool's legs and rungs flex with the
  seat; that is how the traditional construction survives. Rather than
  refine the joining rule until those pass, glue in a cut is measured
  joint by joint only, which is what the gap asked for; the existing
  rule for an edge a joint extends into a housing (a panel glued into
  grooves on both its long edges) is kept.
- **The frame-and-panel trap.** std's `groove` still defaults to glue,
  since `glue:` is a setting every joint has with the default `true` and
  a kind can't change it; a plywood panel or drawer bottom is rightly
  glued. A solid panel glued in a groove now warns where the glue holds
  it across its grain (grooves in the frame pieces at its ends), and the
  existing extension rule warns where it is held between grooves on its
  long edges. The kind's doc says to write `glue: false` and a
  clearance. std's own solid-panel use (`frame_panel_door`) already
  floats its panel; the drawers' bottoms are plywood in every example.
  Whether a floating panel has room at the bottom of its groove
  (`clearance`) is not checked: that needs the groove's depth compared
  with the extension, which the check can't yet read generically.

What changed in the corpus, each a true warning:

- `examples/tapered_table`: 110mm aprons with 10mm shoulders made 90mm
  tenons glued across each leg's grain. 15mm shoulders make them 80mm.
- `examples/shelf_unit`: plywood shelves glued along the whole 280mm of
  dados in solid oak sides. The dados are now dry (`glue: false`). A
  woodworker would glue only the front few centimetres, or screw the
  shelves near the front; Stave can't yet say either (glue over part of
  a dado, or a member in a housed joint, which the screw and dowel kinds
  reject as overlapping), so the example says why the dados are dry.
