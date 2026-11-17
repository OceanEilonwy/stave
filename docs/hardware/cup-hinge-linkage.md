# The cup hinge linkage

How Stave moves a door on concealed 35mm cup hinges, where the numbers come
from, and how they are checked. The mechanism's data is
`crates/stave-schema/src/hinges.rs` (`CUP_HINGE_110`); the check is
`tools/hinge-sim/linkage.py` (run it: exit code 0 when every target is met).

## What a cup hinge is

A cup hinge is not a pin. It is a **four-bar linkage**:

- the **arm**, clipped to the mounting plate on the inside face of the
  cabinet side (it stays put; its front end, the **knuckle**, reaches into
  the cup when the door is closed);
- the **boss**, pressed into the 35mm cup bored in the back of the door (it
  moves with the door);
- two **links**, each turning on a pin in the arm and a pin in the boss.

The door's opening angle is the one input. Turning the boss by an angle θ
fixes where it must be for both links to keep their lengths, so the door
turns *and moves*: forward, away from the cabinet, and sideways. That is
what lets a door that covers the front edge of the side (a full-overlay
door) open without scraping it, and why its hinge edge swings out a little
as it opens (the catalogues' **minimum reveal**: the gap a door needs beside
its hinge edge, to a wall or a neighbouring door).

## Published data

Catalogues give behaviour, not pin coordinates. The linkage is fitted to the
Hettich Sensys 8645i (110°, full overlay) data, which is the most complete
public set:

| Quantity | Value | Source |
|---|---|---|
| Cup | Ø35mm, 12.8mm deep | Sensys 8645i data sheet |
| Overlay | A = C + 12.5mm − D (C boring distance, D plate distance), ±2mm on the side screw | Sensys brochure, "Distance D = C + B − A, B = 12.5mm" |
| Minimum reveal per door, edge radius 1mm (table by door thickness 15–24mm and C 3–7mm) | 19mm door: 0.8mm (C 3–5), 0.7mm (C 6–7); 18mm: 0.6mm; 22mm: 2.2mm (C 3) to 1.5mm (C 7); sharp edge (radius 0): +0.4mm | Sensys brochure, "Minimum reveal per door" |
| Door protrusion T (D = 0, C = 3) | 8.5mm | Sensys brochure, "Hinge protrusion H / door protrusion T" |
| Opening | 110° | |

Blum's CLIP top 110° catalogue gives the same structure with a base of 11mm
(overlay 14–18mm for C 3–7 on a 0mm plate) and a similar minimum-gap table
(0.9–1.0mm for a 19mm door). Stave's `euro_hinge` uses the Hettich figures;
note that neither catalogue gives 18mm overlay at 3mm boring distance on a
0mm plate (Hettich: 15.5mm; Blum: 14mm).

Sources:

- Hettich, *Sensys concealed hinge system* brochure (2015, HUK 14987 EN),
  pp. 8–9: Sensys 8645i/8645/8675 110°; installation dimensions, minimum
  reveal table, overlay table and formula, protrusion H/T.
- Blum, *Catalogue 2022/2023*, CLIP top 110° page: overlay table by plate
  spacing and drilling distance, minimum gap table.

The hinge protrusion H (25mm) isn't used: the brochure doesn't say where it is
measured from.

## Frame

Everything is in the plane across the hinge axis, in millimetres, door
closed:

- `x` across the door, from the cabinet side's inside face toward the middle
  of the door (into the cabinet);
- `y` from the door's back face toward its front.

The side is `x ≤ 0, y ≤ 0` (19mm thick). The door is `0 ≤ y ≤ t`, its hinge
edge at `x = −overlay`. The cup's centre is `C + 17.5mm` in from the door's
edge; with the catalogue's overlay that puts it `5mm + D` from the side's
face, whatever C is. Pins are written relative to the cup (from its centre
along `x`, from the door's back face along `y`), so the mechanism moves with
the cup: a different boring distance, plate height or side screw setting
moves the door and the arm together, as it does in the real hinge, and the
overlay table is met exactly by construction.

## The linkage

| Pin | x (from the cup's centre) | y (from the door's back) |
|---|---|---|
| Inner link, in the arm | 11.99 | 10.81 |
| Inner link, in the boss | −10.33 | 7.75 |
| Outer link, in the arm | 10.87 | 2.24 |
| Outer link, in the boss | −11.47 | 1.51 |

The links are 22.53mm and 22.35mm pin to pin. Closed, they lie across the
cup, from the knuckle (deep in the cup, on the cabinet's side) to the boss
pins near the cup's rim on the door's edge side. The inner link is a solid
bar 9mm wide; the outer link is a strap 14mm wide whose two flanges pass
either side of it, so although the links cross in plan view from 57° on, they
never meet (as in the real hinge, where the outer link straddles the inner).
The arm is 16mm wide; its body runs from the back of the plate to the door's
back face, 2.1mm off the side's face at the middle screw setting (so it stays
off the side at −2mm); its knuckle, 5.8mm wide, reaches 12.4mm into the cup
and carries the arm pins. Links and pins are 1.5mm and Ø3mm.

At rest the two link lines meet 65mm outside the door's hinge edge, a little
behind its back face: the door starts by lifting straight off the cabinet
(its back face moves forward, never into the side's front edge), and as it
turns that instant centre moves in.

## How it was found

A four-bar has eight numbers (two pins on each member). They were found by
differential evolution with feasibility rules (constraints first, then the
fit), then refined by a simplex search, over:

- pins: boss pins inside the cup (1.5mm clear of its wall, at least 0.6mm
  in from its mouth), arm pins inside the knuckle;
- assembling from 0° to 125° at least 1mm of reach from a dead point;
- nothing passing through wood from 0° to 110° (door against side and arm;
  links against side and door outside the cup; knuckle against the cup's
  wall as the door leaves), with the door resting on the side's front edge
  with no gap;
- the fit: the minimum reveal table (weighted to 18–20mm doors) and the
  protrusion at 110°.

A direct optimisation over the catalogue's constraints alone failed until the
links were allowed to cross in plan view: of 150,000 random four-bars that
assemble over 110°, only about 300 keep a door resting on the side's edge out
of the side, and nearly all of those cross. That matches how the real hinge
is built.

## How well it fits

`python3 tools/hinge-sim/linkage.py`:

- closed is exact (offset 0 at 0°); in Stave the door's pose at 0° is the
  identity, no float involved;
- the links reach a dead point only past 130° (2.1mm of margin at 110°);
- overlay = C + 12.5mm for C = 3–7mm (by construction);
- minimum reveal (edge radius 1mm) for 18–20mm doors within 0.36mm of the
  table (19mm door, C = 5: 1.0mm against 0.8mm); thinner doors swing out a
  little more than the table (15mm: 0.6 against 0.2), thicker ones less
  (24mm, C = 5: 2.2 against 3.0);
- door protrusion at 110° (C = 3): 6.7mm against 8.5mm;
- nothing goes into wood for doors 16–22mm, C = 3–7mm and the side screw
  anywhere in ±2mm. The links come to rest on the cup's rim (a 40mm steel
  rim on the 35mm cup, within 1.5mm of the door's face) near full opening:
  that is the hinge's stop.

The example cabinet's left door (19mm, 18mm overlay, C = 5, so the side
screw at +0.5mm), its front hinge corner (`x`, `y` as above; world across =
19 + x, deep = −y):

| Opened | Front hinge corner | Back hinge corner |
|---|---|---|
| 0° | (−18.00, 19.00) | (−18.00, 0.00) |
| 30° | (−15.97, 30.02) | (−6.47, 13.57) |
| 60° | (−16.01, 21.05) | (0.44, 11.55) |
| 90° | (−14.21, 10.01) | (4.79, 10.01) |
| 110° | (−10.96, 3.58) | (6.90, 10.08) |

The door's back corner, which rests on the side's front edge, is 13.6mm in
front of it by 30° and 10mm at 90°; the door's hinge stile is 0.84mm clear of
the side by 1°, 8.3mm by 10°, and comes closest again at 110°, its front
corner 3.58mm in front of the side's edge.

## In Stave

- `stave-schema::hinges::CUP_HINGE_110`: the pins (hundredths of a
  millimetre), arm, knuckle, link and pin sizes, and the widest opening
  (110°). A hinge kind never names pins: every cup hinge kind moves on this
  mechanism, placed relative to its cup.
- `stave-model::linkage::Linkage`: the pins exact (whole nanometres) in a
  `LinkageFrame`, checked by `Linkage::new` in integers: links with length,
  no shared pin, and from 0° to the opening angle the two circles the boss
  must lie on stay at least 0.1mm from touching (a dead point), with the
  sinusoid of their centres' distance bounded in closed form and integer
  fixed-point cosines.
- `stave-geom::linkage::LinkagePath`: the pose at an angle in floats (the
  geometry boundary), the offset's derivative and a bound on how fast any
  point moves, used by the sweeps (`stave-geom::sweep::linkage_contact`, by
  conservative advancement then bisection) and by the viewer.
- `stave-check`: the door swept on the linkage against everything; the
  hinge's own arm and links against its door and side
  (`sweep::mechanism_contact`); `hinge-overlay` against the kind's
  `base_overlay`, `plate_height` and `overlay_adjustment`.
