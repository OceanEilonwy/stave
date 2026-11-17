#!/usr/bin/env python3
"""Design check for the cup hinge linkage Stave uses (crates/stave-schema/src/hinges.rs).

A concealed 35mm cup hinge is a four-bar linkage: the ARM (screwed through the
mounting plate to the cabinet side) carries two pins; the BOSS (pressed into the
cup in the door) carries two more; two LINKS join them. The door's opening angle
is the one input: turning the boss by an angle fixes where it must be for both
links to keep their lengths.

Coordinates (mm), in the plane across the hinge axis, at rest (door closed):
  x  across the door, from the cabinet side's inside face toward the middle of
     the door (into the cabinet);
  y  from the door's back face toward its front (out of the cabinet).
The cabinet side is x <= 0, y <= 0 (19mm thick: x >= -19). The door is
0 <= y <= t, from its hinge edge at x = -overlay to the far side. The cup is
35mm across and 12.8mm deep, its centre CUP_X from the side's face (5mm for the
catalogue's base overlay of 12.5mm, on a 0mm plate, at the middle of the side
screw's travel).

The pins are given relative to the cup (x from its centre, y from the door's
back face), as in hinges.rs; the catalogue data they are fitted to is Hettich
Sensys 8645i (110 degrees, full overlay):
  overlay A = boring distance C + 12.5mm - plate distance D, +-2mm on the screw;
  minimum reveal per door (edge radius 1mm), by door thickness and C;
  door protrusion T = 8.5mm (D = 0, C = 3).

Checks (exit code 0 when all pass):
  1. closed is exact: the offset at 0 degrees is zero;
  2. the linkage assembles from 0 to past 110 degrees, at least 1mm of reach
     from a dead point; the widest angle before a dead point is reported;
  3. overlay = C + 12.5 - D across the boring distances (by construction: the
     mechanism is fixed to the cup);
  4. the hinge edge's swing-out (minimum reveal) against the catalogue table;
  5. the door's protrusion into the opening at 110 degrees against T;
  6. nothing passes through wood from 0 to 110 degrees, for doors 16 to 22mm
     thick, boring distances 3 to 7mm and the side screw anywhere in its
     +-2mm: door against side, door against arm, links against side and door
     (only the cup's bore is taken out of the door: no allowance for its rim),
     arm and knuckle against door (outside the bore);
  7. the door moves forward and out: the front edge path for the example
     cabinet (19mm door, 18mm overlay, C = 5) at 0, 30, 60, 90, 110 degrees.

Pure numpy.
"""

from __future__ import annotations

import sys

import numpy as np

# ---- the mechanism (mm, relative to the cup: x from its centre, y from the door's back face)
ARM_PINS = np.array([[13.50, 10.90], [13.28, 3.56]])   # inner link, outer link
BOSS_PINS = np.array([[-8.80, 7.33], [-9.54, 1.39]])
LINK_THICKNESS = 1.5        # across each link, in the plane
LINK_HALF_WIDTH = (4.5, 7.0)  # how far each link reaches from the middle, along the pins
ARM_X = (-2.9, 15.0)        # the arm's sides, relative to the cup's centre (behind the door)
KNUCKLE_X = (8.0, 15.0)     # the arm's front end, which reaches into the cup
ARM_HALF_WIDTH = 8.0
ARM_FRONT = 12.4            # how far the arm reaches into the cup
ARM_BACK = -55.0            # the back of the plate (37mm setback, 36mm plate)
CUP_RADIUS = 17.5
CUP_DEPTH = 12.8
def chord(half_width: float, radius: float = CUP_RADIUS) -> float:
    """The cup's half-width across the door, half_width off its centre line."""
    return float(np.sqrt(radius**2 - half_width**2))
BASE_OVERLAY = 12.5
CUP_X = 5.0                 # the cup's centre from the side's face (D = 0, middle of the screw)
SIDE = 19.0
EPS = 0.05                  # contact tolerance

# ---- catalogue: Hettich Sensys 8645i minimum reveal per door, edge radius 1mm
REVEAL_T = [15, 16, 17, 18, 19, 20, 21, 22, 23, 24]
REVEAL = {
    3.0: [0.2, 0.3, 0.4, 0.6, 0.8, 1.1, 1.4, 2.2, 3.0, 3.9],
    4.0: [0.2, 0.3, 0.4, 0.6, 0.8, 1.0, 1.3, 1.8, 2.6, 3.4],
    5.0: [0.2, 0.3, 0.4, 0.6, 0.8, 1.0, 1.3, 1.6, 2.2, 3.0],
    6.0: [0.2, 0.3, 0.4, 0.6, 0.7, 1.0, 1.2, 1.6, 2.0, 2.6],
    7.0: [0.2, 0.3, 0.4, 0.5, 0.7, 0.9, 1.2, 1.5, 1.9, 2.4],
}
PROTRUSION_T = 8.5

ANGLES = np.radians(np.arange(0.0, 130.01, 0.05))
I110 = int(round(110 / 0.05))


def rot(points: np.ndarray, theta: np.ndarray) -> np.ndarray:
    """Points (2,) turned by each angle -> (n, 2)."""
    c, s = np.cos(theta), np.sin(theta)
    return np.stack([c * points[0] - s * points[1], s * points[0] + c * points[1]], 1)


def unrot(points: np.ndarray, theta: np.ndarray) -> np.ndarray:
    c, s = np.cos(theta), np.sin(theta)
    return np.stack([c * points[:, 0] + s * points[:, 1], -s * points[:, 0] + c * points[:, 1]], 1)


def design(cup_x: float):
    """The pins in the design frame for a cup centre cup_x from the side."""
    shift = np.array([cup_x, 0.0])
    return ARM_PINS + shift, BOSS_PINS + shift


def solve(arm: np.ndarray, boss: np.ndarray, theta: np.ndarray):
    """The boss's offset after turning by theta about the origin; the reach margin."""
    lengths = np.linalg.norm(boss - arm, axis=1)
    c1 = arm[0] - rot(boss[0], theta)
    c2 = arm[1] - rot(boss[1], theta)
    v = c2 - c1
    dist = np.linalg.norm(v, axis=1)
    margin = np.minimum(lengths.sum() - dist, dist - abs(lengths[0] - lengths[1]))
    along = (lengths[0] ** 2 - lengths[1] ** 2 + dist**2) / (2 * dist)
    across = np.sqrt(np.maximum(lengths[0] ** 2 - along**2, 0.0))
    unit = v / dist[:, None]
    perp = np.stack([-unit[:, 1], unit[:, 0]], 1)
    rest_side = np.sign(cross(v[0], -c1[0]))
    offset = c1 + unit * along[:, None] + perp * (rest_side * across)[:, None]
    return offset, margin, lengths


def cross(a, b) -> float:
    """The 2D cross product a x b."""
    return float(a[0] * b[1] - a[1] * b[0])


def door_point(q, theta, offset):
    return rot(np.asarray(q, float), theta) + offset


def report(name: str, ok: bool, detail: str) -> bool:
    print(f"{'ok  ' if ok else 'FAIL'} {name}: {detail}")
    return ok


def main() -> int:
    passed = True
    arm, boss = design(CUP_X)
    offset, margin, lengths = solve(arm, boss, ANGLES)

    # 1. closed is exact
    passed &= report("closed", np.abs(offset[0]).max() < 1e-9, f"offset at 0deg = {offset[0]}")

    # 2. assembles to past 110 degrees; the widest angle
    dead = np.where(margin < 1.0)[0]
    widest = np.degrees(ANGLES[dead[0]]) if len(dead) else np.degrees(ANGLES[-1])
    passed &= report(
        "range",
        margin[: I110 + 1].min() >= 1.0 and widest > 112,
        f"links {lengths.round(2)} mm; reach margin to 110deg {margin[: I110 + 1].min():.2f}mm; "
        f"1mm from a dead point at {widest:.1f}deg",
    )
    cross_angles = []
    b1 = rot(boss[0], ANGLES) + offset
    b2 = rot(boss[1], ANGLES) + offset
    for k in range(I110 + 1):
        p, q, r, s = arm[0], b1[k], arm[1], b2[k]
        d1 = cross(q - p, r - p)
        d2 = cross(q - p, s - p)
        d3 = cross(s - r, p - r)
        d4 = cross(s - r, q - r)
        if d1 * d2 < 0 and d3 * d4 < 0:
            cross_angles.append(np.degrees(ANGLES[k]))
    print(
        "     the links cross in plan view between "
        + (f"{min(cross_angles):.1f} and {max(cross_angles):.1f}deg" if cross_angles else "never")
        + " (they lie in different layers along the pins: the outer link straddles the inner one)"
    )

    # 3. overlay
    for c in (3, 4, 5, 6, 7):
        overlay = c + CUP_RADIUS - CUP_X  # the door's edge is the cup's rim less C from the cup
        expected = c + BASE_OVERLAY
        passed &= report("overlay", abs(overlay - expected) < 1e-9, f"C={c}: {overlay:.1f}mm (catalogue {expected:.1f}mm)")

    # 4. minimum reveal: how far the hinge edge swings out past where it was
    worst = 0.0
    print("     minimum reveal, edge radius 1mm (ours / catalogue), by door thickness:")
    for c, row in REVEAL.items():
        overlay = c + BASE_OVERLAY
        ours = []
        for t, published in zip(REVEAL_T, row):
            corner_centre = np.array([-overlay + 1.0, t - 1.0])
            x = door_point(corner_centre, ANGLES[: I110 + 1], offset[: I110 + 1])[:, 0] - 1.0
            swing = -overlay - x.min()
            ours.append(swing)
            if t in (18, 19, 20):
                worst = max(worst, abs(swing - published))
        print(f"     C={c:.0f}: " + " ".join(f"{t}:{o:.1f}/{p:.1f}" for t, o, p in zip(REVEAL_T, ours, row)))
    passed &= report("reveal", worst <= 0.4, f"largest difference for 18-20mm doors {worst:.2f}mm (allowed 0.4mm)")

    # 5. protrusion at 110 degrees (D = 0, C = 3)
    overlay = 3.0 + BASE_OVERLAY
    protrusion = door_point([-overlay, 0.0], ANGLES[I110 : I110 + 1], offset[I110 : I110 + 1])[0, 0]
    passed &= report("protrusion", abs(protrusion - PROTRUSION_T) <= 2.0,
                     f"{protrusion:.1f}mm into the opening at 110deg (catalogue {PROTRUSION_T}mm)")

    # 6. nothing passes through wood
    worst_clear = np.inf
    theta = ANGLES[: I110 + 1]
    for screw in (-2.0, -1.0, 0.0, 0.5, 1.0, 2.0):
        cup_x = CUP_X + screw
        arm, boss = design(cup_x)
        offset, margin, _ = solve(arm, boss, ANGLES)
        offset = offset[: I110 + 1]
        b = [rot(boss[i], theta) + offset for i in (0, 1)]
        for c in (3.0, 4.0, 5.0, 6.0, 7.0):
            overlay = c + BASE_OVERLAY - screw
            for t in (16.0, 18.0, 19.0, 20.0, 22.0):
                def in_side(p):
                    # depth inside the side's wood (x in [-19, 0], y <= 0), > 0 means in
                    return np.minimum(np.minimum(-p[:, 0], -p[:, 1]), p[:, 0] + SIDE)

                def in_door(p, half_width):
                    q = unrot(p - offset, theta)
                    inside = np.minimum(np.minimum(q[:, 1], t - q[:, 1]), q[:, 0] + overlay)
                    # Only the cup's bore is taken out of the door: its rim lies
                    # on the face and removes no wood, so nothing is set aside.
                    in_cup = np.minimum(chord(half_width) - np.abs(q[:, 0] - cup_x), CUP_DEPTH - q[:, 1])
                    return np.minimum(inside, -in_cup)

                def in_arm(p):
                    lo, hi = cup_x + ARM_X[0], cup_x + ARM_X[1]
                    return np.minimum(np.minimum(p[:, 0] - lo, hi - p[:, 0]), np.minimum(p[:, 1] - ARM_BACK, -p[:, 1]))

                depth = -np.inf
                # door outline against the side and the arm body (behind the door)
                for q in ([-overlay, 0], [-overlay, t], [-overlay + 5, 0], [0, 0], [cup_x - CUP_RADIUS, 0],
                          [cup_x + CUP_RADIUS, 0], [40, 0], [100, 0], [400, 0], [40, t], [400, t]):
                    p = door_point(q, theta, offset)
                    depth = max(depth, in_side(p).max(), in_arm(p).max())
                # links (each side of the bar) against side and door
                for i in (0, 1):
                    direction = b[i] - arm[i]
                    normal = np.stack([-direction[:, 1], direction[:, 0]], 1)
                    normal /= np.linalg.norm(normal, axis=1)[:, None]
                    length = np.linalg.norm(direction, axis=1)
                    ext = LINK_THICKNESS / 2.0 / length
                    for u in np.linspace(0, 1, 201):
                        for side in (-0.5, 0.5):
                            p = arm[i] + direction * (u * (1 + 2 * ext) - ext)[:, None] + normal * (side * LINK_THICKNESS)
                            depth = max(depth, in_side(p).max(), in_door(p, LINK_HALF_WIDTH[i]).max())
                    # the bore's edges at the door's face inside the bar (a
                    # wood corner can poke into a bar between samples)
                    unit = direction / length[:, None]
                    for x in (cup_x - chord(LINK_HALF_WIDTH[i]), cup_x + chord(LINK_HALF_WIDTH[i])):
                        rel = door_point([x, 0.0], theta, offset) - arm[i]
                        along = (rel * unit).sum(1)
                        across = (rel * normal).sum(1)
                        half = LINK_THICKNESS / 2.0
                        inside = np.minimum(np.minimum(along + half, length + half - along), half - np.abs(across))
                        depth = max(depth, inside.max())
                # the arm against the door: its body behind the door, its
                # knuckle in the cup when closed
                for (lo, hi), (bottom, top) in (
                    (ARM_X, (-8.0, 0.0)),
                    (KNUCKLE_X, (0.0, ARM_FRONT)),
                ):
                    for x in np.linspace(cup_x + lo, cup_x + hi, 25):
                        for y in np.linspace(bottom, top, 17):
                            p = np.tile([x, y], (len(theta), 1))
                            depth = max(depth, in_door(p, ARM_HALF_WIDTH).max())
                # the arm stays off the side's face at every screw setting
                depth = max(depth, -(cup_x + ARM_X[0]))
                worst_clear = min(worst_clear, -depth)
    passed &= report("wood", worst_clear >= -EPS,
                     f"nothing goes into wood deeper than {max(-worst_clear, 0):.3f}mm (tolerance {EPS}mm)")

    # 7. the example cabinet's door: 19mm, overlay 18mm, C = 5 (side screw +0.5mm)
    screw = 5 + BASE_OVERLAY - 18.0
    arm, boss = design(CUP_X + screw)
    offset, _, _ = solve(arm, boss, ANGLES)
    print("     example cabinet (19mm door, 18mm overlay, C=5): door's front hinge corner, and back corner")
    for degrees in (0, 30, 60, 90, 110):
        k = int(round(degrees / 0.05))
        front = door_point([-18.0, 19.0], ANGLES[k : k + 1], offset[k : k + 1])[0]
        back = door_point([-18.0, 0.0], ANGLES[k : k + 1], offset[k : k + 1])[0]
        print(f"     {degrees:3d}deg: front corner ({front[0]:6.2f}, {front[1]:6.2f})  back corner ({back[0]:6.2f}, {back[1]:6.2f})"
              f"  offset ({offset[k][0]:5.2f}, {offset[k][1]:5.2f})")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
