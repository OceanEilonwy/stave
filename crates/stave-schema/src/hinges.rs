//! The built-in mechanism of a cup hinge: where its four pins are, and the
//! sizes of its arm and links.
//!
//! A hinge kind in a `.stave` file is woodworking-level catalogue data
//! (cup size, boring distance, plate setback, overlay); it never gives pin
//! coordinates. Every cup hinge moves on this one linkage, the common
//! 110° full-overlay clip-on hinge, anchored to its cup: the pins are fixed
//! relative to the cup's centre and the face it is bored into, so a
//! different boring distance, plate height or overlay moves the whole
//! mechanism with the cup, as it does in the real hinge.
//!
//! The numbers are derived in `docs/hardware/cup-hinge-linkage.md` and
//! checked by `tools/hinge-sim/linkage.py`: closed, the door rests exactly
//! where it is placed; it opens to 110° without the links crossing or
//! reaching a dead point; its hinge edge swings out no more than the
//! catalogue's minimum reveal; and nothing passes through wood.
//!
//! Coordinates are nanometres in the hinge's plane, at rest:
//!
//! - `x` from the cup's centre toward the middle of the door (away from the
//!   door's hinge edge);
//! - `y` from the face the cup is bored into (the back of the door) toward
//!   the door's front: positive inside the cup, negative inside the
//!   cabinet.

/// A point in the hinge's plane relative to the cup (see the module docs),
/// in nanometres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CupPoint {
    /// From the cup's centre toward the middle of the door.
    pub x: i64,
    /// From the door's back face into the door.
    pub y: i64,
}

/// One link: its pin in the arm and its pin in the boss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CupLink {
    /// The pin in the arm (fixed to the cabinet).
    pub arm: CupPoint,
    /// The pin in the boss (in the cup, moving with the door).
    pub boss: CupPoint,
    /// How wide the link is along the pins, in nanometres.
    pub width: i64,
    /// The gap down its middle, along the pins: a strap link is two
    /// flanges with the other link between them (zero for a solid bar).
    pub slot: i64,
}

/// A cup hinge's mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CupMechanism {
    /// The two links: the inner link (deeper in the cup when closed, a
    /// solid bar) first, then the outer link (a strap whose flanges pass
    /// either side of the inner link).
    pub links: [CupLink; 2],
    /// The arm's sides across the door (`x`, relative to the cup's
    /// centre), behind the door.
    pub arm_x: (i64, i64),
    /// The sides of the arm's front end (its knuckle), which reaches into
    /// the cup and carries the arm pins.
    pub knuckle_x: (i64, i64),
    /// How far the knuckle reaches into the cup (`y`).
    pub knuckle_front: i64,
    /// How wide the arm is along the pins.
    pub arm_width: i64,
    /// How thick each link is, across it in the plane.
    pub link_thickness: i64,
    /// The pins' diameter.
    pub pin_diameter: i64,
    /// The widest angle the hinge opens to, in micro-degrees: the
    /// mechanism is checked free of wood up to here (its links come to
    /// rest on the cup's rim, the hinge's stop).
    pub max_opening: i64,
}

/// A hundredth of a millimetre, in nanometres.
const HUNDREDTH: i64 = 10_000;

/// The 110° full-overlay clip-on cup hinge (fitted to Hettich Sensys
/// 8645i catalogue data; see the module docs).
pub const CUP_HINGE_110: CupMechanism = CupMechanism {
    links: [
        CupLink {
            arm: CupPoint {
                x: 1199 * HUNDREDTH,
                y: 1081 * HUNDREDTH,
            },
            boss: CupPoint {
                x: -1033 * HUNDREDTH,
                y: 775 * HUNDREDTH,
            },
            width: 900 * HUNDREDTH,
            slot: 0,
        },
        CupLink {
            arm: CupPoint {
                x: 1087 * HUNDREDTH,
                y: 224 * HUNDREDTH,
            },
            boss: CupPoint {
                x: -1147 * HUNDREDTH,
                y: 151 * HUNDREDTH,
            },
            width: 1400 * HUNDREDTH,
            slot: 1100 * HUNDREDTH,
        },
    ],
    arm_x: (-290 * HUNDREDTH, 1380 * HUNDREDTH),
    knuckle_x: (800 * HUNDREDTH, 1380 * HUNDREDTH),
    knuckle_front: 1240 * HUNDREDTH,
    arm_width: 1600 * HUNDREDTH,
    link_thickness: 150 * HUNDREDTH,
    pin_diameter: 300 * HUNDREDTH,
    max_opening: 110_000_000,
};
