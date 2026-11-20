# Cut list allowances: kerf, rough sizes, bought thickness, edge banding

Audit finding 106: the cut list model had no allowances. Four 450mm parts
"fit" an 1800mm board only with no saw kerf; a part exactly `max_length`
long passed the length check although it has to be cut a little long and
trimmed; nothing said that 19mm oak is bought as 4/4 (1in) and planed; and
an edge-banded plywood shelf was listed at its drawn size, a band's
thickness too wide. There was also no cut list to put any of this in: the
spec described one, but no crate made it. This note records what was
built and how it differs from the audit's sketch.

## What the audit proposed

`kerf: 3mm` and `rough: allowance(length: 25mm, width: 5mm)` on the
project, overridable per stock; `sold: 4/4` or `nominal: 2x4` on solid
stock; `banding: edge_band(thickness: 1mm, on: [front])` on sheet parts,
the drawn size including the band and the cut size taking it off; and the
rough allowance applied in the `max_length` check.

## What was built

- **A cut list** (`stave_solve::cut_list`, printed by `stave cut-list`):
  one row per part to cut (declared parts, mirror images, made parts,
  pieces of a spliced part, and a part made of boards one row per board),
  each with its cut size, its rough size and its banding; how many boards
  of each solid or dowel stock to buy; and how much banding. It is derived
  from the solved design and never stored. Every size is exact until it is
  printed and rounded once to the project's `resolution:` with
  `ExactLength::round_to`, which records how far it moved.
- **`kerf:` on the project (default 3mm) and on any stock.** It is used
  only to count boards: rough lengths laid end to end, longest first, first
  fit, a kerf between neighbours, each piece taking a whole board's width.
  The count is never short; side-by-side ripping, which can save boards,
  is mentioned rather than worked out.
- **`rough_length:` and `rough_width:` on the project (default 0mm) and on
  solid stock**, rather than `rough: allowance(...)`. Two plain lengths
  need no new type and no new built-in function, read the same way as
  `max_length:` and `max_width:` beside them, and are what a beginner
  copies from a woodworking book ("add an inch in length and a quarter
  inch in width"). They are measured along and across the grain, as
  `max_length` and `max_width` are. They default to 0mm so that existing
  designs keep passing their length check; a project opts in.
- **The length check uses the rough length** for solid stock (the cut
  length plus `rough_length`), in `check_max_length` and for each piece of
  a spliced part; the message names both parts of the sum.
- **`sold:` on solid stock, a length**, rather than `sold: 4/4` or
  `nominal: 2x4`. `4/4` would read as the ratio 1 in the expression
  grammar, and `2x4` is not a token; a length says the same thing exactly
  (`sold: 1in`, `sold: 2in`) and needs no lexer change. It is the
  thickness only: a 2x4's actual width is already its stock `width:`.
  It must be at least the finished thickness.
- **`banding: edge_band(1mm, on: ...)` on a part of plywood or MDF**, as
  the audit sketched, with the faces named as placed (the way `edges:`
  names them). The drawn and cut sizes include the band, so placement,
  the checks and the 3D view see the banded panel; only the cut list
  takes the band off, once per banded face. The banding needed is totalled
  per thickness.

## Left out

- Laying parts out on sheets (a 2D nesting). Sheet rows are listed with
  no count of sheets to buy.
- Ripping several narrow parts from one wide board when counting boards.
- Banding on solid wood, and a band of a different species shown in the
  3D view.
- A cut list view in the window: `stave-app`'s Cut list view does not show
  rows yet; the function is headless and ready for it.
