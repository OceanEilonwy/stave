#!/usr/bin/env python3
"""Design check for the chidori joint used in examples/japanese_joinery.

Three square sticks of side S cross at right angles in one node:

  first  (along x)  keeps the LOWER half of the node; a round hole (diameter D)
                    is bored through it at the node, and an open slot (width d)
                    runs from the hole out to the node's front-right corner,
                    on the diagonal.
  second (along y)  keeps the UPPER half of the node, with the same hole and
                    the same diagonal slot.
  lock   (along z)  is rounded to diameter D where it passes through the node,
                    with two flats d apart. With the flats along the diagonal it
                    slides into the node through the two slots; a quarter turn
                    puts its full diameter across the slots and locks the node.

The script checks, by sampling solid geometry:
  1. the assembled node has no wood passing through wood;
  2. every part is locked: any small move of any part collides;
  3. the node comes apart (the assembly sequence run backwards): turn the lock
     a quarter turn back (checked through the whole turn), slide it out along
     the diagonal, then lift the second stick off the first;
  4. the design depends on its features: without the flats, or without the
     rounding, the lock can't get in.

Exit code 0 when every check passes. Pure numpy; no other dependencies.
"""

from __future__ import annotations

import sys
from dataclasses import dataclass

import numpy as np

# ---- design (mm) -------------------------------------------------------------
S = 30.0              # stick section
L = 300.0             # stick length
D = 0.75 * S          # diameter of the rounded section and of the bored holes
FLATS = 0.45 * S      # distance across the flats on the rounded section = slot width
H = S / 2

EPS = 0.05            # contact tolerance: surfaces may touch within this
GRID = 0.4            # sampling step
REGION = 36.0         # half-size of the sampled region around the node

U = np.array([1.0, 1.0, 0.0]) / np.sqrt(2.0)   # the diagonal the lock slides along
V = np.array([-1.0, 1.0, 0.0]) / np.sqrt(2.0)  # across the diagonal


# ---- solids as membership tests, in each part's own frame ---------------------
# inside(p, m) is True where p lies inside the part shrunk by m (m >= 0), so a
# point within m of the surface does not count: that is the contact tolerance.

def _slot(p: np.ndarray, m: float) -> np.ndarray:
    """The open diagonal slot, grown by m (so the part is shrunk by m)."""
    along = p @ U
    across = np.abs(p @ V)
    return (across < FLATS / 2 + m) & (along > -m)


def _hole(p: np.ndarray, m: float) -> np.ndarray:
    return p[:, 0] ** 2 + p[:, 1] ** 2 < (D / 2 + m) ** 2


def inside_first(p: np.ndarray, m: float) -> np.ndarray:
    x, y, z = p[:, 0], p[:, 1], p[:, 2]
    box = (np.abs(x) <= L / 2 - m) & (np.abs(y) <= H - m) & (np.abs(z) <= H - m)
    upper_half_of_node = (np.abs(x) < H + m) & (z > -m)
    return box & ~upper_half_of_node & ~_hole(p, m) & ~_slot(p, m)


def inside_second(p: np.ndarray, m: float) -> np.ndarray:
    x, y, z = p[:, 0], p[:, 1], p[:, 2]
    box = (np.abs(y) <= L / 2 - m) & (np.abs(x) <= H - m) & (np.abs(z) <= H - m)
    lower_half_of_node = (np.abs(y) < H + m) & (z < m)
    return box & ~lower_half_of_node & ~_hole(p, m) & ~_slot(p, m)


@dataclass(frozen=True)
class LockShape:
    rounded: bool = True
    flats: bool = True


def inside_lock(p: np.ndarray, m: float, shape: LockShape = LockShape()) -> np.ndarray:
    """The lock in its own frame. At angle 0 the flats face along U (locked)."""
    x, y, z = p[:, 0], p[:, 1], p[:, 2]
    box = (np.abs(x) <= H - m) & (np.abs(y) <= H - m) & (np.abs(z) <= L / 2 - m)
    in_node = np.abs(z) <= H + m
    keep = np.ones(len(p), dtype=bool)
    if shape.rounded:
        keep &= x ** 2 + y ** 2 <= (D / 2 - m) ** 2
    if shape.flats:
        keep &= np.abs(p @ U) <= FLATS / 2 - m
    return box & (~in_node | keep)


# ---- poses --------------------------------------------------------------------

def rot_z(degrees: float) -> np.ndarray:
    a = np.radians(degrees)
    c, s = np.cos(a), np.sin(a)
    return np.array([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]])


@dataclass
class Pose:
    angle: float = 0.0                       # about z, degrees
    shift: np.ndarray = None                 # world translation

    def __post_init__(self) -> None:
        if self.shift is None:
            self.shift = np.zeros(3)

    def to_world(self, local: np.ndarray) -> np.ndarray:
        return local @ rot_z(self.angle).T + self.shift

    def to_local(self, world: np.ndarray) -> np.ndarray:
        return (world - self.shift) @ rot_z(self.angle)


def grid_points() -> np.ndarray:
    axis = np.arange(-REGION, REGION + GRID / 2, GRID)
    gx, gy, gz = np.meshgrid(axis, axis, axis, indexing="ij")
    return np.stack([gx.ravel(), gy.ravel(), gz.ravel()], axis=1)


GRID_POINTS = grid_points()


@dataclass
class Part:
    name: str
    inside: callable

    def samples(self) -> np.ndarray:
        """Sample points filling the part near the node, in its own frame."""
        return GRID_POINTS[self.inside(GRID_POINTS, 0.0)]


def collides(a: Part, pose_a: Pose, b: Part, pose_b: Pose) -> bool:
    """True if a and b share volume deeper than the contact tolerance."""
    for mover, pm, other, po in ((a, pose_a, b, pose_b), (b, pose_b, a, pose_a)):
        world = pm.to_world(mover.samples())
        if np.any(other.inside(po.to_local(world), EPS)):
            return True
    return False


# ---- checks -------------------------------------------------------------------

FIRST = Part("first", inside_first)
SECOND = Part("second", inside_second)
LOCK = Part("lock", inside_lock)
PARTS = [FIRST, SECOND, LOCK]
REST = {p.name: Pose() for p in PARTS}

failures: list[str] = []


def check(ok: bool, what: str) -> None:
    print(("  ok    " if ok else "  FAIL  ") + what)
    if not ok:
        failures.append(what)


def any_collision(poses: dict[str, Pose], parts: list[Part]) -> str | None:
    for i, a in enumerate(parts):
        for b in parts[i + 1:]:
            if collides(a, poses[a.name], b, poses[b.name]):
                return f"{a.name} / {b.name}"
    return None


def directions() -> list[np.ndarray]:
    out = []
    for dx in (-1, 0, 1):
        for dy in (-1, 0, 1):
            for dz in (-1, 0, 1):
                if (dx, dy, dz) != (0, 0, 0):
                    v = np.array([dx, dy, dz], dtype=float)
                    out.append(v / np.linalg.norm(v))
    return out


def main() -> int:
    print(f"chidori: S={S}mm, rounded diameter D={D}mm, flats {FLATS}mm, "
          f"tolerance {EPS}mm, sampling {GRID}mm")

    print("1. assembled node")
    clash = any_collision(REST, PARTS)
    check(clash is None, "no wood passes through wood" + (f" (clash: {clash})" if clash else ""))

    print("2. every part is locked")
    step = 4 * GRID
    for part in PARTS:
        free = []
        for d in directions():
            poses = dict(REST)
            poses[part.name] = Pose(REST[part.name].angle, d * step)
            if any_collision(poses, PARTS) is None:
                free.append(np.round(d, 2).tolist())
        check(not free, f"{part.name} can't move {step:.1f}mm in any of 26 directions"
              + (f" (free: {free})" if free else ""))

    print("3. the node comes apart (assembly run backwards)")
    turn_ok = True
    for angle in np.arange(0.0, -90.0 - 0.5, -1.0):
        poses = dict(REST)
        poses["lock"] = Pose(angle)
        if any_collision(poses, PARTS) is not None:
            turn_ok = False
            print(f"        lock collides at {angle:.0f} degrees")
            break
    check(turn_ok, "turn the lock back a quarter turn, checked every degree")

    slide_ok = True
    for t in np.arange(0.0, 60.0 + GRID, GRID):
        poses = dict(REST)
        poses["lock"] = Pose(-90.0, U * t)
        if any_collision(poses, PARTS) is not None:
            slide_ok = False
            print(f"        lock collides after sliding {t:.1f}mm")
            break
    check(slide_ok, "slide the lock out along the diagonal until clear (60mm)")

    lift_ok = True
    for t in np.arange(0.0, S + 2 * GRID, GRID):
        poses = dict(REST)
        poses["second"] = Pose(0.0, np.array([0.0, 0.0, t]))
        if any_collision(poses, [FIRST, SECOND]) is not None:
            lift_ok = False
            print(f"        second collides after lifting {t:.1f}mm")
            break
    check(lift_ok, "lift the second stick off the first until clear")

    print("4. the features are necessary")
    for label, shape in (("without the flats", LockShape(flats=False)),
                         ("without the rounding", LockShape(rounded=False))):
        variant = Part("lock", lambda p, m, s=shape: inside_lock(p, m, s))
        blocked = False
        for t in np.arange(0.0, 60.0 + GRID, GRID):
            poses = {"first": Pose(), "second": Pose(), "lock": Pose(-90.0, U * t)}
            if any_collision(poses, [FIRST, SECOND, variant]) is not None:
                blocked = True
                break
        check(blocked, f"{label}, the lock can't slide in")

    print()
    if failures:
        print(f"{len(failures)} check(s) failed")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
