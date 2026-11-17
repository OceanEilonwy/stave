# The cup hinge linkage

How Stave moves a door on concealed 35mm cup hinges, where the numbers come
from, and how they are checked. The mechanism's data is text: the members
and `mechanism { }` of `euro_hinge` in `std/hardware/hinges.stave`
(grammar, "Mechanisms"); the check is
`crates/stave-app/tests/oracle_hinge.rs`, which measures every number
below on small cabinets Stave solves, and the fit that found the pins is
the ignored test `refit_the_cup_hinge` in the same file (run by hand,
see "How it was found").

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
| Inner link, in the arm | 13.50 | 10.90 |
| Inner link, in the boss | −8.80 | 7.33 |
| Outer link, in the arm | 13.28 | 3.56 |
| Outer link, in the boss | −9.54 | 1.39 |

The links are 22.58mm and 22.92mm pin to pin. Closed, they lie across the
cup, from the knuckle (deep in the cup, on the cabinet's side) to the boss
pins near the cup's rim on the door's edge side. The inner link is a solid
bar 9mm wide; the outer link is a strap 14mm wide whose two flanges pass
either side of it, so although the links cross in plan view from 42° on, they
never meet (as in the real hinge, where the outer link straddles the inner).
The arm is 16mm wide; its body runs from the back of the plate to the door's
back face, 2.1mm off the side's face at the middle screw setting (so it stays
off the side at −2mm); its knuckle, 7mm wide, reaches 12.4mm into the cup
and carries the arm pins. Links and pins are 1.5mm and Ø3mm.

At rest the two link lines meet 65mm outside the door's hinge edge, a little
behind its back face: the door starts by lifting straight off the cabinet
(its back face moves forward, never into the side's front edge), and as it
turns that instant centre moves in.

## How it was found

A four-bar has eight numbers (two pins on each member). They were found by
differential evolution with feasibility rules (constraints first, then the
fit), over:

- pins: boss pins inside the cup (1.5mm clear of its wall, at least 0.6mm
  in from its mouth), arm pins inside the knuckle (1.5mm clear of its
  sides);
- assembling from 0° to 125° at least 1mm of reach from a dead point;
- **no steel in wood**, from 0° to 110°, for doors 16 to 22mm thick, boring
  distances 3 to 7mm and the side screw anywhere in ±2mm: only the cup's
  bore is taken out of the door. Both faces of each link (sampled along it,
  including half its thickness past each pin) and the bore's edges at the
  door's face (wood corners that could poke into a bar between samples)
  are tested, and steel must keep 0.1mm off the wood; the door may rest on
  the side's front edge (wood on wood, touching);
- the fit: the minimum reveal table (18–20mm doors) and the protrusion at
  110°.

**Why it was refitted.** An earlier fit allowed the links to rest up to
1.5mm deep in the wood around the cup ("the rim's shadow", called the
hinge's stop) and sampled each link at only 21 points. A cup's rim lies on
the door's face and removes no wood, so that was steel through wood: in the
example cabinet both links went into the door's stile from 98° (the outer
link) to 110°, 0.80mm deep (inner link, 5.8mm³) and 0.74mm (outer link,
13.4mm³) at 110°, measured by `stave_geom::sweep::steel_in_wood` with
only the bore removed. Stave's check carried the same exemption, so it
didn't say. The exemption is gone from both; the real stop is a limiter
inside the arm (steel on steel), not modelled as a shape, and the opening
is checked free of wood all the way to it. The knuckle was widened from
5.8mm to 7mm so the arm pins can sit nearer the cup's far wall, which
keeps the links inside the cup's mouth as they fold out.

The fit is a model of the mechanism in millimetres (the side and the
door as half-planes, the members sampled), seeded so a run repeats:

```
cargo test --release -p stave-app --test oracle_hinge -- --ignored --nocapture refit_the_cup_hinge
```

`STAVE_REFIT_GENERATIONS` (default 300) and `STAVE_REFIT_SEED` (default
1) choose the run; it prints the pins rounded to hundredths, how they
score once rounded, and whether they are today's, and fails only if it
finds nothing feasible or nothing as good as today's pins (which are the
first candidate). The model is the one the pins were found with: a
regular test, `the_refit_scores_todays_pins_as_the_script_did`, checks
that it scores today's pins 0.26394, as the original Python fit (numpy)
did. The random numbers differ from that script's, so the run differs
too: from seed 1 the Python fit kept today's pins over 300 generations,
while the Rust run ends at 0.263 with pins on the edge of the 0.1mm
steel clearance (rounded to hundredths they miss it by 2e-14mm). Today's
pins stay as they are.

## How well it fits

Each line is a test in `crates/stave-app/tests/oracle_hinge.rs`, run on
cabinets Stave solves (a 19mm side, one door on two `euro_hinge`s):

- `closed_is_exact`: closed, the door is exactly where it is placed (its
  pose at 0° is the identity, no float involved), its hinge edge exactly
  the overlay past the side's inside face;
- `the_links_reach_past_110_degrees_with_a_millimetre_to_spare`: links
  22.58mm and 22.92mm; at least 1.02mm from a dead point (at 5.4°), and
  still more than a millimetre at 130°;
- `the_overlay_is_the_catalogues_for_every_boring_distance`: overlay =
  C + 12.5mm for C = 3–7mm (by construction: the cup's centre 5mm in from
  the side, C + 17.5mm from the door's edge);
- `the_hinge_edge_swings_out_as_the_catalogue_says`: minimum reveal (edge
  radius 1mm) for 18–20mm doors within 0.25mm of the table (20mm door,
  C = 3: 0.85mm against 1.1mm); thinner doors swing out a little more than
  the table (15mm: 0.6 against 0.2), thicker ones less (24mm, C = 5: 1.1
  against 3.0); every cell is pinned to 0.01mm;
- `the_door_stands_8_6mm_into_the_opening_at_110_degrees`: door protrusion
  at 110° (C = 3): 8.64mm against 8.5mm;
- `nothing_passes_through_wood_at_any_side_screw_setting`: Stave's own
  checks find nothing in wood for doors 16, 18, 19, 20 and 22mm, C = 3–7mm
  (every millimetre) and the side screw at −2, −1, 0, +0.5, +1 and +2mm (a
  23mm or 24mm door with C = 7 reaches the side just before 110°, at 109.3°
  and 108.4°, and Stave says so);
- `the_front_edge_moves_forward_and_out`: the table below, to 0.01mm.

The example cabinet's left door (19mm, 18mm overlay, C = 5, so the side
screw at +0.5mm), its front hinge corner (`x`, `y` as above; world across =
19 + x, deep = −y):

| Opened | Front hinge corner | Back hinge corner |
|---|---|---|
| 0° | (−18.00, 19.00) | (−18.00, 0.00) |
| 30° | (−12.11, 33.02) | (−2.61, 16.57) |
| 60° | (−13.93, 21.78) | (2.53, 12.28) |
| 90° | (−12.46, 9.73) | (6.54, 9.73) |
| 110° | (−9.03, 2.95) | (8.83, 9.45) |

The door's back corner, which rests on the side's front edge, is 16.6mm in
front of it by 30° and 9.7mm at 90°; the door comes closest to the side
again at 110°, its front corner 2.95mm in front of the side's edge (Stave's
focus mode: "closest at 110°: 2.9 mm").

## In Stave

- `euro_hinge` in `std/hardware/hinges.stave`: the pins (to a hundredth of
  a millimetre) as `pivot` entries, the arm and knuckle as the arm's two
  boxes, the links as bars, the pin diameter, and the widest opening (its
  `opening_angle` param's range and the mechanism's `stop`, 110°), all in
  the kind's frame, relative to its cup. Another cup hinge is another kind
  with its own mechanism; `stave-solve`'s `mechanism` module checks it has
  one freedom and the four-bar form before the door moves.
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
  hinge's own arm, knuckle and links against its door (less only the
  cup's bore) and side, from 0° (`sweep::mechanism_contact`; a member
  already in wood when closed is caught at 0°), and measured
  (`sweep::steel_in_wood`) by a test over every example's cup hinges; `hinge-overlay` against the kind's
  `base_overlay`, `plate_height` and `overlay_adjustment`.
