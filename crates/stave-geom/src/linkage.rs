//! Solving a four-bar linkage (a cup hinge) at an angle: where the boss,
//! and the door with it, and each link are when the door has turned by
//! that much.
//!
//! The pins are exact ([`Linkage`]); here, at the geometry boundary, they
//! become `f64` nanometres in the linkage's plane. Turning the boss by `θ`
//! about the frame's origin puts its pins at `R(θ)·b`; it must then move by
//! the offset `d` that brings each pin back to its link's length from its
//! arm pin: `d` lies on the circle about `a₁ − R(θ)·b₁` of radius `L₁` and
//! on the circle about `a₂ − R(θ)·b₂` of radius `L₂`. Of their two
//! meeting points the linkage's [`Branch`] picks one, the same at every
//! angle; at `θ = 0` the offset is zero, exactly (rest is returned
//! without solving anything).
//!
//! [`LinkagePath::speed_bound`] bounds how fast any point of the door can
//! move per radian of opening, from the offset's derivative (found by
//! differentiating the two length constraints), which the sweeps use to
//! step without missing a contact.

use glam::{DMat3, DMat4, DVec2, DVec3};
use stave_model::solved::{Branch, Linkage};

use crate::collide::{direction_vector, point_vector};
use crate::render;

/// A linkage ready to solve, in `f64` nanometres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkagePath {
    origin: DVec3,
    /// World unit vectors of the plane's `x`, `y` and of `x × y`.
    x: DVec3,
    y: DVec3,
    normal: DVec3,
    ground: [DVec2; 2],
    coupler: [DVec2; 2],
    lengths: [f64; 2],
    /// `+1` for [`Branch::Left`], `-1` for [`Branch::Right`].
    side: f64,
    /// Radians.
    opening: f64,
}

/// Where everything is at one angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkagePose {
    /// The boss (and the door): world nanometres to world nanometres.
    pub coupler: DMat4,
    /// Each link, turned about its arm pin.
    pub links: [DMat4; 2],
}

impl LinkagePath {
    /// The linkage in floats.
    pub fn new(linkage: &Linkage) -> LinkagePath {
        let frame = linkage.frame();
        let x = direction_vector(frame.free_side());
        let y = direction_vector(frame.swing_toward());
        let plane =
            |p: stave_model::solved::PlanePoint| DVec2::new(render::nm(p.x), render::nm(p.y));
        let [first, second] = *linkage.links();
        let ground = [plane(first.ground), plane(second.ground)];
        let coupler = [plane(first.coupler), plane(second.coupler)];
        let ([g1, g2], [b1, b2]) = (ground, coupler);
        LinkagePath {
            origin: point_vector(frame.origin()),
            x,
            y,
            normal: x.cross(y),
            ground,
            coupler,
            lengths: [(b1 - g1).length(), (b2 - g2).length()],
            side: match linkage.branch() {
                Branch::Left => 1.0,
                Branch::Right => -1.0,
            },
            opening: render::radians_f64(linkage.opening_angle()),
        }
    }

    /// How far it opens, in radians.
    pub fn opening(&self) -> f64 {
        self.opening
    }

    /// The boss's offset in the plane after turning by `theta` about the
    /// origin; `None` where the links can't reach (never inside the
    /// checked range).
    pub fn offset(&self, theta: f64) -> Option<DVec2> {
        if theta == 0.0 {
            return Some(DVec2::ZERO);
        }
        let rotation = DVec2::from_angle(theta);
        let ([g1, g2], [b1, b2]) = (self.ground, self.coupler);
        let c1 = g1 - rotation.rotate(b1);
        let c2 = g2 - rotation.rotate(b2);
        let between = c2 - c1;
        let distance = between.length();
        if distance < 1e-9 {
            return None;
        }
        let [l1, l2] = self.lengths;
        let along = (l1 * l1 - l2 * l2 + distance * distance) / (2.0 * distance);
        let across_squared = l1 * l1 - along * along;
        if across_squared < 0.0 {
            return None;
        }
        let unit = between / distance;
        Some(c1 + unit * along + unit.perp() * (self.side * across_squared.sqrt()))
    }

    /// The offset's derivative with respect to the angle: from
    /// `(R b_i + d − a_i) · (R' b_i + d') = 0` for both links.
    fn offset_rate(&self, theta: f64) -> Option<DVec2> {
        let d = self.offset(theta)?;
        let rotation = DVec2::from_angle(theta);
        // Each link: its arm (row) and the right-hand side; d/dθ of R b is
        // R b turned a quarter.
        let [(first, r1), (second, r2)] = [0, 1].map(|i| {
            let (coupler, ground) = (self.coupler_pin(i), self.ground_pin(i));
            let turned = rotation.rotate(coupler);
            let arm = turned + d - ground;
            (arm, -arm.dot(turned.perp()))
        });
        let determinant = first.perp_dot(second);
        if determinant.abs() < 1e-9 {
            return None;
        }
        // Solve [first; second] · d' = (r1, r2) (Cramer).
        Some(DVec2::new(
            (r1 * second.y - first.y * r2) / determinant,
            (first.x * r2 - r1 * second.x) / determinant,
        ))
    }

    /// The fastest the boss's offset changes anywhere in the range, in
    /// nanometres per radian: sampled finely (every 0.05deg, and a little
    /// past the range), with a tenth more for what lies between samples.
    pub fn offset_speed(&self) -> f64 {
        let steps = 2400;
        let fastest = (0..=steps)
            .filter_map(|k| self.offset_rate(self.opening * 1.05 * f64::from(k) / f64::from(steps)))
            .map(DVec2::length)
            .fold(0.0, f64::max);
        fastest * 1.1 + 1_000.0
    }

    /// The fastest any point of the door within `radius` of the frame's
    /// origin (in the plane) moves, in nanometres per radian of opening,
    /// given [`LinkagePath::offset_speed`]: it turns about the origin and
    /// moves with the offset.
    pub fn speed_bound(radius: f64, offset_speed: f64) -> f64 {
        radius + offset_speed
    }

    /// A world point in the plane's coordinates (`x`, `y`, and along the
    /// axis), relative to the frame's origin.
    pub fn to_plane(&self, point: DVec3) -> DVec3 {
        let relative = point - self.origin;
        DVec3::new(
            relative.dot(self.x),
            relative.dot(self.y),
            relative.dot(self.normal),
        )
    }

    /// The world transform of a motion in the plane: turn by `angle` about
    /// the plane point `about`, then move by `offset`.
    fn plane_motion(&self, about: DVec2, angle: f64, offset: DVec2) -> DMat4 {
        let basis = DMat3::from_cols(self.x, self.y, self.normal);
        let to_world = DMat4::from_translation(self.origin) * DMat4::from_mat3(basis);
        let to_plane = DMat4::from_mat3(basis.transpose()) * DMat4::from_translation(-self.origin);
        let about = DVec3::new(about.x, about.y, 0.0);
        let local = DMat4::from_translation(DVec3::new(offset.x, offset.y, 0.0) + about)
            * DMat4::from_rotation_z(angle)
            * DMat4::from_translation(-about);
        to_world * local * to_plane
    }

    /// The boss's (the door's) world transform at `theta`; `None` where the
    /// links can't reach.
    pub fn coupler(&self, theta: f64) -> Option<DMat4> {
        if theta == 0.0 {
            return Some(DMat4::IDENTITY);
        }
        let offset = self.offset(theta)?;
        Some(self.plane_motion(DVec2::ZERO, theta, offset))
    }

    /// Everything's world transform at `theta`.
    pub fn pose(&self, theta: f64) -> Option<LinkagePose> {
        if theta == 0.0 {
            return Some(LinkagePose {
                coupler: DMat4::IDENTITY,
                links: [DMat4::IDENTITY; 2],
            });
        }
        let offset = self.offset(theta)?;
        let rotation = DVec2::from_angle(theta);
        let link = |i: usize| {
            let (coupler, ground) = (self.coupler_pin(i), self.ground_pin(i));
            let rest = coupler - ground;
            let now = rotation.rotate(coupler) + offset - ground;
            self.plane_motion(ground, rest.angle_to(now), DVec2::ZERO)
        };
        Some(LinkagePose {
            coupler: self.plane_motion(DVec2::ZERO, theta, offset),
            links: [link(0), link(1)],
        })
    }

    fn coupler_pin(&self, link: usize) -> DVec2 {
        self.coupler.get(link).copied().unwrap_or(DVec2::ZERO)
    }

    fn ground_pin(&self, link: usize) -> DVec2 {
        self.ground.get(link).copied().unwrap_or(DVec2::ZERO)
    }

    /// A pin of a link at `theta`, in the plane: the arm pin (`coupler:
    /// false`) or the boss pin.
    pub fn pin(&self, link: usize, coupler: bool, theta: f64) -> Option<DVec2> {
        let ground = *self.ground.get(link)?;
        if !coupler {
            return Some(ground);
        }
        let rest = *self.coupler.get(link)?;
        Some(DVec2::from_angle(theta).rotate(rest) + self.offset(theta)?)
    }

    /// The boss's turn and offset at `theta`, in the plane.
    pub fn plane_pose(&self, theta: f64) -> Option<(DVec2, DVec2)> {
        Some((DVec2::from_angle(theta), self.offset(theta)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::placement::{Axis, Direction};
    use stave_lang::units::{Angle, Length};
    use stave_model::solved::{Link, LinkageFrame, PlanePoint, Point};

    /// The pins of `euro_hinge` (`std/hardware/hinges.stave`), in
    /// nanometres across from the cup's centre and in from the door's
    /// back: the arm's pin, then the cup's, for the inner link and then the
    /// outer link.
    const PINS: [[(i64, i64); 2]; 2] = [
        [(13_500_000, 10_900_000), (-8_800_000, 7_330_000)],
        [(13_280_000, 3_560_000), (-9_540_000, 1_390_000)],
    ];

    /// The cup hinge on a 19mm door with its cup 5mm from the edge, in a
    /// frame at the door's front hinge corner.
    fn cup_hinge(opening: Angle) -> Linkage {
        let place = |(across, depth): (i64, i64)| {
            PlanePoint::new(
                Length::from_nm(22_500_000 + across),
                Length::from_nm(depth - 19_000_000),
            )
        };
        let links = PINS.map(|[arm, cup]| Link {
            ground: place(arm),
            coupler: place(cup),
        });
        let frame = LinkageFrame::new(
            Axis::Up,
            Point::default(),
            Direction::Right,
            Direction::Front,
        )
        .unwrap();
        Linkage::new(frame, links, opening).unwrap()
    }

    #[test]
    fn the_cup_hinge_linkage_rests_exactly_and_opens_to_its_angle() {
        let linkage = cup_hinge(Angle::from_deg(110));
        let path = LinkagePath::new(&linkage);
        assert_eq!(path.offset(0.0), Some(DVec2::ZERO));
        assert_eq!(path.coupler(0.0), Some(DMat4::IDENTITY));
        // Every angle to the full opening has a pose, and both links keep
        // their lengths.
        for k in 0..=1100 {
            let theta = (f64::from(k) / 10.0).to_radians();
            for link in 0..2 {
                let arm = path.pin(link, false, theta).unwrap();
                let boss = path.pin(link, true, theta).unwrap();
                let rest = path.pin(link, true, 0.0).unwrap() - arm;
                assert!(((boss - arm).length() - rest.length()).abs() < 1e-3);
            }
        }
        // At 90deg the door's front hinge corner (the frame's origin) is
        // 5.54mm further across and 9.27mm further back than closed, as
        // docs/hardware/cup-hinge-linkage.md has it; its back corner, which rested
        // on the cabinet's front (deep 19mm here), is 10mm in front of it:
        // the door was pushed out as it turned.
        let door = path.coupler(90f64.to_radians()).unwrap();
        let corner = door.transform_point3(DVec3::ZERO);
        assert!((corner.x / 1e6 - 5.54).abs() < 0.02, "{corner}");
        assert!((corner.z / 1e6 - 9.27).abs() < 0.02, "{corner}");
        let back = door.transform_point3(DVec3::new(0.0, 0.0, 19e6));
        assert!((back.z / 1e6 - 9.27).abs() < 0.02, "{back}");
        assert!((back.x / 1e6 - 24.54).abs() < 0.02, "{back}");
        // The offset changes no faster than the bound the sweeps use.
        let speed = path.offset_speed();
        for k in 0..1100 {
            let (a, b) = (
                (f64::from(k) / 10.0).to_radians(),
                (f64::from(k + 1) / 10.0).to_radians(),
            );
            let step = (path.offset(b).unwrap() - path.offset(a).unwrap()).length();
            assert!(step <= speed * (b - a) * 1.000_001, "{k}");
        }
        // The exact check agrees: the links never reach a dead point.
        assert!(Linkage::new(*linkage.frame(), *linkage.links(), Angle::from_deg(180)).is_ok());
    }
}
