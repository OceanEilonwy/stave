//! An orbit camera: a target, a direction to look from and a distance.
//! Every operation is a pure function returning a new [`Orbit`], so the
//! viewer only stores the current one.

use glam::{Mat4, Vec3, Vec4Swizzles};
use stave_geom::Aabb;

/// The vertical field of view, radians (30°: a natural, gently
/// foreshortened view of furniture).
pub const FIELD_OF_VIEW: f32 = std::f32::consts::PI / 6.0;

/// How far one pixel of mouse movement turns the camera, radians.
const RADIANS_PER_PIXEL: f32 = 0.008;

/// The lowest and highest the camera may look from, radians: a little
/// below the floor up to straight down.
const PITCH_RANGE: (f32, f32) = (-0.5, 1.55);

/// Distances the camera keeps to its target, millimetres.
const DISTANCE_RANGE: (f32, f32) = (20.0, 200_000.0);

/// The direction a design is first seen from: in front, to the right and
/// above, like standing in a showroom.
const DEFAULT_YAW: f32 = 0.58;
const DEFAULT_PITCH: f32 = 0.30;

/// Room left around a framed design, as a fraction of its size.
const FRAME_MARGIN: f32 = 1.12;

/// A ray in the render frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    /// Where it starts.
    pub origin: Vec3,
    /// Its direction, unit length.
    pub direction: Vec3,
}

/// How the camera projects: in perspective, or flat (orthographic), the
/// way a drawing shows a plan or an elevation: sizes don't shrink with
/// distance, so parallel edges stay parallel and lengths can be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Projection {
    /// A natural view, gently foreshortened.
    #[default]
    Perspective,
    /// A flat view: as wide as the perspective view is at the target.
    Flat,
}

/// Where the camera is: it looks at `target` from `distance` away, from
/// the direction given by `yaw` (about the up axis, 0 = straight in front,
/// positive toward the right) and `pitch` (positive = from above).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    /// The point looked at.
    pub target: Vec3,
    /// Radians about `+y`, from `+z` (the front) toward `+x` (the right).
    pub yaw: f32,
    /// Radians above the horizontal.
    pub pitch: f32,
    /// Millimetres from the target.
    pub distance: f32,
    /// Perspective or flat.
    pub projection: Projection,
}

impl Orbit {
    /// A camera that shows the whole box, from the default direction.
    pub fn framing(bounds: Aabb, aspect: f32) -> Orbit {
        Orbit {
            target: bounds.centre(),
            yaw: DEFAULT_YAW,
            pitch: DEFAULT_PITCH,
            distance: 1.0,
            projection: Projection::Perspective,
        }
        .framed(bounds, aspect)
    }

    /// A flat view straight down onto the box, its `yaw` the same as a
    /// view from the front turned that far (0: the front at the bottom of
    /// the picture), framed to show it all: a plan.
    pub fn plan(bounds: Aabb, aspect: f32, yaw: f32) -> Orbit {
        Orbit {
            target: bounds.centre(),
            yaw,
            pitch: std::f32::consts::FRAC_PI_2,
            distance: 1.0,
            projection: Projection::Flat,
        }
        .framed(bounds, aspect)
    }

    /// The same direction, moved to show the whole box: the target at its
    /// centre and the camera just far enough back for every corner to be
    /// in view.
    pub fn framed(self, bounds: Aabb, aspect: f32) -> Orbit {
        let tan_vertical = (FIELD_OF_VIEW * 0.5).tan();
        let tan_horizontal = tan_vertical * aspect.max(0.05);
        let target = bounds.centre();
        let backward = self.backward();
        let (right, up) = Orbit { target, ..self }.screen_axes();
        // A corner at depth `z` toward the camera fits when the camera is
        // at least `z + |x| / tan` back, across and up.
        let needed = bounds
            .corners()
            .iter()
            .map(|corner| {
                let offset = *corner - target;
                let toward = offset.dot(backward);
                let across = offset.dot(right).abs() * FRAME_MARGIN / tan_horizontal;
                let upward = offset.dot(up).abs() * FRAME_MARGIN / tan_vertical;
                match self.projection {
                    Projection::Perspective => toward + across.max(upward),
                    // A flat view is as wide as the perspective one at the
                    // target: depth doesn't shrink anything.
                    Projection::Flat => across.max(upward),
                }
            })
            .fold(0.0_f32, f32::max);
        Orbit {
            target,
            distance: needed.clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1),
            ..self
        }
    }

    /// From where the camera looks: a unit vector from the target toward
    /// the eye.
    pub fn backward(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
    }

    /// The eye position.
    pub fn eye(&self) -> Vec3 {
        self.target + self.backward() * self.distance
    }

    /// The camera's right and up directions on screen. Right is level
    /// and set by the yaw alone, so looking straight down still knows
    /// which way is which.
    fn screen_axes(&self) -> (Vec3, Vec3) {
        let backward = self.backward();
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = backward.cross(right).normalize_or(Vec3::Y);
        (right, up)
    }

    /// Turned by a mouse drag of `(dx, dy)` pixels: right turns the design
    /// to the right, down looks from higher up.
    pub fn orbited(self, dx: f32, dy: f32) -> Orbit {
        Orbit {
            yaw: (self.yaw - dx * RADIANS_PER_PIXEL).rem_euclid(std::f32::consts::TAU),
            pitch: (self.pitch + dy * RADIANS_PER_PIXEL).clamp(PITCH_RANGE.0, PITCH_RANGE.1),
            ..self
        }
    }

    /// Moved sideways by a drag of `(dx, dy)` pixels in a view
    /// `viewport_height` pixels high, so the point under the mouse follows
    /// it.
    pub fn panned(self, dx: f32, dy: f32, viewport_height: f32) -> Orbit {
        let per_pixel =
            2.0 * self.distance * (FIELD_OF_VIEW * 0.5).tan() / viewport_height.max(1.0);
        let (right, up) = self.screen_axes();
        Orbit {
            target: self.target + (up * dy - right * dx) * per_pixel,
            ..self
        }
    }

    /// Closer (`steps > 0`) or further away, by 12% a step.
    pub fn zoomed(self, steps: f32) -> Orbit {
        Orbit {
            distance: (self.distance * 0.88_f32.powf(steps))
                .clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1),
            ..self
        }
    }

    /// World to camera.
    pub fn view(&self) -> Mat4 {
        let (_, up) = self.screen_axes();
        Mat4::look_at_rh(self.eye(), self.target, up)
    }

    /// Camera to clip space (depth 0 to 1), for a view `aspect` wide per
    /// unit of height.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        let near = (self.distance * 0.01).max(1.0);
        let far = self.distance * 50.0 + 10_000.0;
        match self.projection {
            Projection::Perspective => {
                Mat4::perspective_rh(FIELD_OF_VIEW, aspect.max(0.01), near, far)
            }
            Projection::Flat => {
                let half_height = self.distance * (FIELD_OF_VIEW * 0.5).tan();
                let half_width = half_height * aspect.max(0.01);
                Mat4::orthographic_rh(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    near,
                    far,
                )
            }
        }
    }

    /// World to clip space.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// The ray through a pixel of a `width` × `height` view (pixels from
    /// the top left).
    pub fn ray(&self, x: f32, y: f32, width: f32, height: f32) -> Ray {
        let (width, height) = (width.max(1.0), height.max(1.0));
        let inverse = self.view_projection(width / height).inverse();
        let ndc_x = 2.0 * x / width - 1.0;
        let ndc_y = 1.0 - 2.0 * y / height;
        let unproject = |depth: f32| {
            let point = inverse * glam::Vec4::new(ndc_x, ndc_y, depth, 1.0);
            point.xyz() / point.w
        };
        let near = unproject(0.0);
        let far = unproject(1.0);
        Ray {
            origin: near,
            direction: (far - near).normalize_or(-self.backward()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cabinet_bounds() -> Aabb {
        Aabb {
            min: Vec3::new(-20.0, -80.0, -560.0),
            max: Vec3::new(920.0, 1700.0, 54.0),
        }
    }

    #[test]
    fn framing_shows_the_whole_box() {
        let bounds = cabinet_bounds();
        let aspect = 1.6;
        let orbit = Orbit::framing(bounds, aspect);
        assert_eq!(orbit.target, bounds.centre());
        let clip = orbit.view_projection(aspect);
        for corner in bounds.corners() {
            let p = clip.project_point3(corner);
            assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0, "{corner} at {p}");
            assert!((0.0..=1.0).contains(&p.z));
        }
        // Not absurdly far: some corner comes near the edge of the view.
        let widest = bounds
            .corners()
            .iter()
            .map(|c| {
                let p = clip.project_point3(*c);
                p.x.abs().max(p.y.abs())
            })
            .fold(0.0_f32, f32::max);
        assert!(widest > 0.6, "{widest}");
    }

    #[test]
    fn the_centre_ray_hits_the_target() {
        let orbit = Orbit::framing(cabinet_bounds(), 1.0);
        let ray = orbit.ray(400.0, 300.0, 800.0, 600.0);
        let to_target = (orbit.target - ray.origin).normalize();
        assert!(ray.direction.dot(to_target) > 0.9999);
    }

    #[test]
    fn orbit_pan_and_zoom() {
        let orbit = Orbit::framing(cabinet_bounds(), 1.0);
        let turned = orbit.orbited(100.0, 0.0);
        assert!((turned.yaw - (orbit.yaw - 0.8).rem_euclid(std::f32::consts::TAU)).abs() < 1e-5);
        assert_eq!(turned.distance, orbit.distance);
        let high = orbit.orbited(0.0, 10_000.0);
        assert!(high.pitch <= PITCH_RANGE.1);
        // Panning right by the whole view moves the target left by the
        // view's width at the target.
        let panned = orbit.panned(600.0, 0.0, 600.0);
        let moved = (panned.target - orbit.target).length();
        let expected = 2.0 * orbit.distance * (FIELD_OF_VIEW * 0.5).tan();
        assert!((moved - expected).abs() / expected < 1e-4);
        let (right, _) = orbit.screen_axes();
        assert!((panned.target - orbit.target).dot(right) < 0.0);
        assert!(orbit.zoomed(1.0).distance < orbit.distance);
        assert!(orbit.zoomed(-1.0).distance > orbit.distance);
    }

    #[test]
    fn a_plan_looks_straight_down_flat_with_the_front_at_the_bottom() {
        let bounds = cabinet_bounds();
        let orbit = Orbit::plan(bounds, 1.6, 0.0);
        assert_eq!(orbit.projection, Projection::Flat);
        let clip = orbit.view_projection(1.6);
        for corner in bounds.corners() {
            let p = clip.project_point3(corner);
            assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0, "{corner} at {p}");
            assert!((0.0..=1.0).contains(&p.z));
        }
        // The front (+z) is at the bottom of the picture, the right on the
        // right, and height doesn't move anything on screen.
        let front = clip.project_point3(Vec3::new(0.0, 0.0, 50.0));
        let back = clip.project_point3(Vec3::new(0.0, 0.0, -500.0));
        assert!(front.y < back.y);
        let right = clip.project_point3(Vec3::new(900.0, 0.0, 0.0));
        let left = clip.project_point3(Vec3::new(0.0, 0.0, 0.0));
        assert!(right.x > left.x);
        let high = clip.project_point3(Vec3::new(0.0, 1500.0, 0.0));
        assert!((high.x - left.x).abs() < 1e-5 && (high.y - left.y).abs() < 1e-5);
    }
}
