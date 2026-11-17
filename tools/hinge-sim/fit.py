#!/usr/bin/env python3
"""Refits the cup hinge linkage's pins (see linkage.py) with a strict rule:
no steel in wood, anywhere, at any angle from 0 to 110 degrees.

The earlier fit let the links rest up to 1.5mm into the wood around the cup
("the rim's shadow", called the hinge's stop). A cup's rim lies on the
door's face and removes no wood, so that was steel through wood near full
opening (0.8mm deep at 110 degrees in the example cabinet). Here the only
wood taken away is the cup's bore itself, the links must leave the cup
through its mouth, and the arm's knuckle must clear the bore's wall.

Differential evolution with feasibility rules (a feasible candidate beats an
infeasible one; two infeasible ones compare by how far they violate; two
feasible ones by the fit), started around the current pins, pure numpy.
Prints the best pins; linkage.py checks them over the full grid.

    python3 tools/hinge-sim/fit.py [generations] [seed]
"""

from __future__ import annotations

import sys

import numpy as np

sys.path.insert(0, __import__('os').path.dirname(__file__))
import linkage as L  # noqa: E402

# The fit's grid: the extremes of door thickness, boring distance and side
# screw (linkage.py then checks the full grid at 0.05 degree steps).
FIT_SCREWS = (-2.0, 0.0, 2.0)
FIT_C = (3.0, 5.0, 7.0)
FIT_T = (16.0, 19.0, 22.0)
THETA = np.radians(np.arange(0.0, 110.01, 0.5))
STEEL_CLEAR = 0.1
WIDE = np.radians(np.arange(0.0, 125.01, 1.0))

# Where pins may be (relative to the cup: x from its centre, y from the
# door's back face). Boss pins inside the cup, 1.5mm clear of its wall and
# at least 0.6mm in from its mouth and above its bottom; arm pins inside
# the knuckle, 1.5mm clear of its sides.
LOW = np.array([L.KNUCKLE_X[0] + 1.5, -6.0, L.KNUCKLE_X[0] + 1.5, -6.0, -16.0, 0.6, -16.0, 0.6])
HIGH = np.array([L.KNUCKLE_X[1] - 1.5, L.ARM_FRONT - 1.5, L.KNUCKLE_X[1] - 1.5, L.ARM_FRONT - 1.5,
                 16.0, L.CUP_DEPTH - 1.5, 16.0, L.CUP_DEPTH - 1.5])


def unpack(v):
    return v[:4].reshape(2, 2), v[4:].reshape(2, 2)


def in_door_strict(p, offset, theta, t, overlay, cup_x, half_width):
    """Depth inside the door's wood (> 0 in), the cup's bore alone removed."""
    d = p - offset
    cs, sn = np.cos(theta), np.sin(theta)
    q = np.stack([cs * d[..., 0] + sn * d[..., 1], -sn * d[..., 0] + cs * d[..., 1]], -1)
    inside = np.minimum(np.minimum(q[..., 1], t - q[..., 1]), q[..., 0] + overlay)
    in_cup = np.minimum(L.chord(half_width) - np.abs(q[..., 0] - cup_x), L.CUP_DEPTH - q[..., 1])
    return np.minimum(inside, -in_cup)


def corners_in_bar(arm_pin, boss_pin, offset, cup_x, half_width):
    """How deep the bore's edges at the door's face reach into a link's bar
    (as long as pin to pin plus half its thickness past each, as Stave draws
    it), at every angle; > 0 is in."""
    direction = boss_pin - arm_pin
    length = np.linalg.norm(direction, axis=1)
    unit = direction / length[:, None]
    normal = np.stack([-unit[:, 1], unit[:, 0]], 1)
    half = L.LINK_THICKNESS / 2.0
    deepest = -np.inf
    for x in (cup_x - L.chord(half_width), cup_x + L.chord(half_width)):
        corner = L.door_point([x, 0.0], THETA, offset)
        rel = corner - arm_pin
        along = (rel * unit).sum(1)
        across = (rel * normal).sum(1)
        depth = np.minimum(np.minimum(along + half, length + half - along), half - np.abs(across))
        deepest = max(deepest, depth.max())
    return deepest


def evaluate(v):
    """(violation, error): violation 0 when feasible."""
    arm_rel, boss_rel = unpack(v)
    violation = 0.0
    # Assembles to 125 degrees, 1mm from a dead point.
    arm, boss = arm_rel + [L.CUP_X, 0], boss_rel + [L.CUP_X, 0]
    lengths = np.linalg.norm(boss - arm, axis=1)
    if lengths.min() < 8.0:
        return 1e3, 0.0
    with np.errstate(all="ignore"):
        _, margin, _ = L.solve(arm, boss, WIDE)
    if not np.isfinite(margin).all():
        return 1e3, 0.0
    violation += max(0.0, 1.0 - margin.min()) * 10
    worst = 0.0
    for screw in FIT_SCREWS:
        cup_x = L.CUP_X + screw
        arm, boss = arm_rel + [cup_x, 0], boss_rel + [cup_x, 0]
        with np.errstate(all="ignore"):
            offset, margin, _ = L.solve(arm, boss, THETA)
        if not np.isfinite(offset).all():
            return 1e3, 0.0
        b = [L.rot(boss[i], THETA) + offset for i in (0, 1)]
        for c in FIT_C:
            overlay = c + L.BASE_OVERLAY - screw
            for t in FIT_T:
                depth = -np.inf
                steel = -np.inf
                # The door against the side and the arm.
                for q in ([-overlay, 0], [-overlay, t], [-overlay + 5, 0], [0, 0],
                          [cup_x - L.CUP_RADIUS, 0], [cup_x + L.CUP_RADIUS, 0], [40, 0], [400, 0]):
                    p = L.door_point(q, THETA, offset)
                    side = np.minimum(np.minimum(-p[:, 0], -p[:, 1]), p[:, 0] + L.SIDE)
                    lo, hi = cup_x + L.ARM_X[0], cup_x + L.ARM_X[1]
                    in_arm = np.minimum(np.minimum(p[:, 0] - lo, hi - p[:, 0]),
                                        np.minimum(p[:, 1] - L.ARM_BACK, -p[:, 1]))
                    depth = max(depth, side.max(), in_arm.max())
                # Each link's bar, both faces, against the side and the door.
                for i in (0, 1):
                    direction = b[i] - arm[i]
                    normal = np.stack([-direction[:, 1], direction[:, 0]], 1)
                    normal /= np.linalg.norm(normal, axis=1)[:, None]
                    u = np.linspace(-0.04, 1.04, 55)[:, None, None]
                    for s in (-0.5, 0.5):
                        p = arm[i] + direction[None] * u + normal[None] * (s * L.LINK_THICKNESS)
                        side = np.minimum(np.minimum(-p[..., 0], -p[..., 1]), p[..., 0] + L.SIDE)
                        door = in_door_strict(p, offset[None], THETA[None], t, overlay, cup_x,
                                              L.LINK_HALF_WIDTH[i])
                        steel = max(steel, side.max(), door.max())
                    # The bore's edges at the door's face (the wood's
                    # corners nearest the link) inside the bar: sampling
                    # the bar's faces alone misses a corner poking in.
                    steel = max(steel, corners_in_bar(arm[i], b[i], offset, cup_x, L.LINK_HALF_WIDTH[i]))
                # The arm's body and knuckle against the door.
                for (lo, hi), (bottom, top) in ((L.ARM_X, (-8.0, 0.0)), (L.KNUCKLE_X, (0.0, L.ARM_FRONT))):
                    xs, ys = np.meshgrid(np.linspace(cup_x + lo, cup_x + hi, 9), np.linspace(bottom, top, 7))
                    p = np.stack([xs.ravel(), ys.ravel()], 1)[:, None, :] + 0 * offset[None]
                    door = in_door_strict(p, offset[None], THETA[None], t, overlay, cup_x, L.ARM_HALF_WIDTH)
                    steel = max(steel, door.max())
                depth = max(depth, -(cup_x + L.ARM_X[0]))
                # Wood against wood may touch (the door rests on the side);
                # steel keeps STEEL_CLEAR off the wood, so rounding the pins
                # and the viewer's own geometry never put it in.
                worst = max(worst, depth, steel + STEEL_CLEAR)
    violation += max(0.0, worst)
    if violation > 0:
        return violation, 0.0
    # The fit: minimum reveal for 18-20mm doors, and the protrusion.
    arm, boss = arm_rel + [L.CUP_X, 0], boss_rel + [L.CUP_X, 0]
    offset, _, _ = L.solve(arm, boss, THETA)
    error = 0.0
    for c, row in L.REVEAL.items():
        overlay = c + L.BASE_OVERLAY
        for t, published in zip(L.REVEAL_T, row):
            if t not in (18, 19, 20):
                continue
            corner = np.array([-overlay + 1.0, t - 1.0])
            x = L.door_point(corner, THETA, offset)[:, 0] - 1.0
            error += (-overlay - x.min() - published) ** 2
    overlay = 3.0 + L.BASE_OVERLAY
    protrusion = L.door_point([-overlay, 0.0], THETA[-1:], offset[-1:])[0, 0]
    error += 0.1 * (protrusion - L.PROTRUSION_T) ** 2
    return 0.0, error


def better(a, b):
    return a[0] < b[0] if (a[0] > 0 or b[0] > 0) else a[1] < b[1]


def main() -> int:
    generations = int(sys.argv[1]) if len(sys.argv) > 1 else 300
    rng = np.random.default_rng(int(sys.argv[2]) if len(sys.argv) > 2 else 1)
    start = np.concatenate([L.ARM_PINS.ravel(), L.BOSS_PINS.ravel()])
    size = 48
    population = np.clip(start + rng.normal(0, 1.5, (size, 8)), LOW, HIGH)
    population[0] = np.clip(start, LOW, HIGH)
    scores = [evaluate(v) for v in population]
    for generation in range(generations):
        for i in range(size):
            a, b, c = population[rng.choice([j for j in range(size) if j != i], 3, replace=False)]
            mutant = np.clip(a + rng.uniform(0.5, 0.9) * (b - c), LOW, HIGH)
            cross = rng.random(8) < 0.8
            cross[rng.integers(8)] = True
            trial = np.where(cross, mutant, population[i])
            score = evaluate(trial)
            if better(score, scores[i]) or score == scores[i]:
                population[i], scores[i] = trial, score
        best = min(range(size), key=lambda k: (scores[k][0], scores[k][1]))
        if generation % 20 == 0 or generation == generations - 1:
            print(f"generation {generation}: violation {scores[best][0]:.3f}, error {scores[best][1]:.3f}",
                  flush=True)
    best = min(range(size), key=lambda k: (scores[k][0], scores[k][1]))
    arm, boss = unpack(population[best])
    print("ARM_PINS =", np.round(arm, 2).tolist())
    print("BOSS_PINS =", np.round(boss, 2).tolist())
    return 0 if scores[best][0] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
