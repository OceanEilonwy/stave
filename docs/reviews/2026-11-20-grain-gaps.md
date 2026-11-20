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
