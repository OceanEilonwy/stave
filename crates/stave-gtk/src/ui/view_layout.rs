//! Where the 3D view's floating controls go, so that none covers another
//! however small the 3D View panel is docked.
//!
//! The corners: the scene menu at the top left; the camera buttons (with
//! X-ray and Pull apart) at the top right, Pull apart's slider popping
//! out to the left of its button and focus mode's From above / In 3D
//! switch under them; a short line and the shared-wood key along the top
//! between them; the moving assemblies' chips at the bottom left and the
//! selection's motion slider at the bottom right; a joint's step player,
//! a problem's motion card and the chip naming a problem off screen
//! above them, in the middle. Anything that doesn't fit beside its
//! neighbour moves above or below it instead.

use std::collections::BTreeMap;

/// How far the controls sit from the view's edges, in pixels.
const EDGE: i32 = 14;
/// The room between two controls, in pixels.
const GAP: i32 = 8;
/// The widest a card in the middle of the foot gets, in pixels.
const CARD_WIDTH: i32 = 620;
/// The narrowest Pull apart's slider is squeezed to beside its button
/// before it goes below the camera buttons instead, in pixels.
const NARROWEST_SLIDER: i32 = 180;

/// One of the 3D view's floating controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Floating {
    /// The scene menu ("As built").
    Scene,
    /// The camera buttons, X-ray and Pull apart, in one column.
    Camera,
    /// Pull apart's play button and slider, beside its button.
    PullSlider,
    /// Focus mode's From above / In 3D switch.
    PlanSwitch,
    /// A short line about what the view shows.
    Hint,
    /// The shared-wood key.
    ContactKey,
    /// The moving assemblies' chips ("Move: door_l …").
    Moves,
    /// The selection's motion slider ("door_l closed ——o—— 110°").
    Motion,
    /// A joint's step player.
    StepCard,
    /// A problem's motion card.
    ProblemCard,
    /// The chip naming a problem off screen.
    EdgePin,
}

/// A control's place in the view, in pixels from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// From the left.
    pub x: i32,
    /// From the top.
    pub y: i32,
    /// How wide.
    pub width: i32,
    /// How tall.
    pub height: i32,
}

impl Rect {
    fn right(self) -> i32 {
        self.x.saturating_add(self.width)
    }

    fn bottom(self) -> i32 {
        self.y.saturating_add(self.height)
    }

    /// `true` if the two share any rows.
    fn shares_rows(self, top: i32, bottom: i32) -> bool {
        self.y < bottom && top < self.bottom()
    }
}

/// How big the controls want to be.
pub trait Measure {
    /// The control's smallest and natural widths, or `None` while it is
    /// hidden.
    fn widths(&self, item: Floating) -> Option<(i32, i32)>;
    /// Its height at a width.
    fn height(&self, item: Floating, width: i32) -> i32;
}

/// Where every showing control goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    places: BTreeMap<Floating, Rect>,
}

impl Placement {
    /// Where a control goes, or `None` when it isn't showing.
    pub fn get(&self, item: Floating) -> Option<Rect> {
        self.places.get(&item).copied()
    }

    fn put(&mut self, item: Floating, rect: Rect) -> Rect {
        self.places.insert(item, rect);
        rect
    }
}

/// A control's width and height when it may be at most `widest` wide,
/// or `None` while it is hidden (or has no size).
fn size(measure: &impl Measure, item: Floating, widest: i32) -> Option<(i32, i32)> {
    let (smallest, natural) = measure.widths(item)?;
    let width = natural.min(widest).max(smallest);
    let height = measure.height(item, width);
    (width > 0 && height > 0).then_some((width, height))
}

/// Places the controls in a view `width` by `height` pixels.
pub fn place(width: i32, height: i32, measure: &impl Measure) -> Placement {
    let mut placement = Placement::default();
    let room = width.saturating_sub(2 * EDGE).max(0);

    // The top right: the camera column, its slider and the plan switch.
    let mut right_column: Vec<Rect> = Vec::new();
    let mut right_y = EDGE;
    let camera = size(measure, Floating::Camera, room).map(|(w, h)| {
        let rect = placement.put(
            Floating::Camera,
            Rect {
                x: width - EDGE - w,
                y: EDGE,
                width: w,
                height: h,
            },
        );
        right_y = rect.bottom() + GAP;
        right_column.push(rect);
        rect
    });
    if let Some(camera) = camera
        && let Some((smallest, natural)) = measure.widths(Floating::PullSlider)
    {
        let beside = camera.x - EDGE;
        let slider_width = natural.max(smallest);
        if beside >= slider_width.min(NARROWEST_SLIDER.max(smallest)) {
            // Out to the left of the button, level with the column's foot.
            let w = slider_width.min(beside);
            let h = measure.height(Floating::PullSlider, w);
            let rect = placement.put(
                Floating::PullSlider,
                Rect {
                    x: camera.x - w,
                    y: (camera.bottom() - h).max(EDGE),
                    width: w,
                    height: h,
                },
            );
            right_column.push(rect);
        } else if let Some((w, h)) = size(measure, Floating::PullSlider, room) {
            let rect = placement.put(
                Floating::PullSlider,
                Rect {
                    x: width - EDGE - w,
                    y: right_y,
                    width: w,
                    height: h,
                },
            );
            right_y = rect.bottom() + GAP;
            right_column.push(rect);
        }
    }
    if let Some((w, h)) = size(measure, Floating::PlanSwitch, room) {
        let rect = placement.put(
            Floating::PlanSwitch,
            Rect {
                x: width - EDGE - w,
                y: right_y,
                width: w,
                height: h,
            },
        );
        right_column.push(rect);
    }
    // The right edge left free beside the right column between two rows.
    let free_right = |top: i32, bottom: i32| {
        right_column
            .iter()
            .filter(|rect| rect.shares_rows(top, bottom))
            .map(|rect| rect.x - GAP)
            .fold(width - EDGE, i32::min)
    };

    // The top left: the scene menu.
    let scene = size(measure, Floating::Scene, room).map(|(w, h)| {
        placement.put(
            Floating::Scene,
            Rect {
                x: EDGE,
                y: EDGE,
                width: w,
                height: h,
            },
        )
    });

    // Along the top, in the middle: the line, then the key.
    let mut top_y = EDGE;
    for item in [Floating::Hint, Floating::ContactKey] {
        let Some((smallest, natural)) = measure.widths(item) else {
            continue;
        };
        let natural = natural.max(smallest);
        // Beside the scene menu when it fits, else under it.
        let beside_scene = scene.filter(|scene| scene.shares_rows(top_y, top_y + 1));
        let mut left = beside_scene.map_or(EDGE, |scene| scene.right() + GAP);
        let mut right = free_right(top_y, top_y + 1);
        if let Some(scene) = beside_scene
            && natural > right - left
        {
            top_y = scene.bottom() + GAP;
            left = EDGE;
            right = free_right(top_y, top_y + 1);
        }
        let w = natural.min((right - left).max(0)).max(smallest);
        let h = measure.height(item, w);
        if w <= 0 || h <= 0 {
            continue;
        }
        let centred = (width - w) / 2;
        let x = centred.min(right - w).max(left);
        let rect = placement.put(
            item,
            Rect {
                x,
                y: top_y,
                width: w,
                height: h,
            },
        );
        top_y = rect.bottom() + GAP;
    }

    // The foot, from the bottom up: the motion slider at the right, the
    // chips at the left (beside it when there is room, else above it).
    let mut foot = height - EDGE;
    let mut row_top = foot;
    let motion = size(measure, Floating::Motion, room).map(|(w, h)| {
        let right = free_right(foot - h, foot);
        let smallest = measure.widths(Floating::Motion).map_or(0, |(s, _)| s);
        let w = w.min(right - EDGE).max(smallest);
        placement.put(
            Floating::Motion,
            Rect {
                x: right - w,
                y: foot - h,
                width: w,
                height: h,
            },
        )
    });
    if let Some(motion) = motion {
        row_top = motion.y;
    }
    if let Some((smallest, natural)) = measure.widths(Floating::Moves) {
        let natural = natural.max(smallest);
        let beside = motion.map_or(free_right(foot - 1, foot), |m| m.x - GAP) - EDGE;
        if natural <= beside {
            let h = measure.height(Floating::Moves, natural);
            let rect = placement.put(
                Floating::Moves,
                Rect {
                    x: EDGE,
                    y: foot - h,
                    width: natural,
                    height: h,
                },
            );
            row_top = row_top.min(rect.y);
        } else {
            let bottom = motion.map_or(foot, |m| m.y - GAP);
            // Wraps onto more rows when it must: its height depends on
            // its width, and its width on the column beside it.
            let mut w = natural.min(room).max(smallest);
            let mut h = measure.height(Floating::Moves, w);
            let right = free_right(bottom - h, bottom);
            if right - EDGE < w {
                w = (right - EDGE).max(smallest);
                h = measure.height(Floating::Moves, w);
            }
            let rect = placement.put(
                Floating::Moves,
                Rect {
                    x: EDGE,
                    y: bottom - h,
                    width: w,
                    height: h,
                },
            );
            row_top = row_top.min(rect.y);
        }
    }
    foot = if row_top < foot { row_top - GAP } else { foot };

    // Above them, in the middle: the cards, then the off-screen chip.
    for item in [Floating::StepCard, Floating::ProblemCard, Floating::EdgePin] {
        let widest = match item {
            Floating::StepCard | Floating::ProblemCard => room.min(CARD_WIDTH),
            _ => room,
        };
        let Some((smallest, natural)) = measure.widths(item) else {
            continue;
        };
        let w = match item {
            // The cards take the width they are given.
            Floating::StepCard | Floating::ProblemCard => widest.max(smallest),
            _ => natural.min(widest).max(smallest),
        };
        let h = measure.height(item, w);
        if w <= 0 || h <= 0 {
            continue;
        }
        let rect = placement.put(
            item,
            Rect {
                x: (width - w) / 2,
                y: foot - h,
                width: w,
                height: h,
            },
        );
        foot = rect.y - GAP;
    }
    placement
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Controls of fixed sizes; the chips wrap onto two rows when given
    /// less than their width.
    struct Fixed(BTreeMap<Floating, (i32, i32)>);

    impl Measure for Fixed {
        fn widths(&self, item: Floating) -> Option<(i32, i32)> {
            let (width, _) = self.0.get(&item)?;
            let smallest = match item {
                Floating::Moves => 120,
                Floating::PullSlider => 160,
                _ => *width,
            };
            Some((smallest.min(*width), *width))
        }

        fn height(&self, item: Floating, width: i32) -> i32 {
            let (natural, height) = self.0.get(&item).copied().unwrap_or((0, 0));
            if item == Floating::Moves && width < natural {
                height * 2 + 6
            } else {
                height
            }
        }
    }

    fn controls(items: &[(Floating, i32, i32)]) -> Fixed {
        Fixed(items.iter().map(|(item, w, h)| (*item, (*w, *h))).collect())
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    fn assert_apart(placement: &Placement) {
        let rects: Vec<(Floating, Rect)> = placement.places.iter().map(|(k, v)| (*k, *v)).collect();
        for (i, (a, ra)) in rects.iter().enumerate() {
            for (b, rb) in rects.iter().skip(i + 1) {
                assert!(!overlap(*ra, *rb), "{a:?} {ra:?} covers {b:?} {rb:?}");
            }
        }
    }

    const EVERYDAY: [(Floating, i32, i32); 5] = [
        (Floating::Scene, 120, 42),
        (Floating::Camera, 40, 236),
        (Floating::PullSlider, 300, 44),
        (Floating::Moves, 420, 38),
        (Floating::Motion, 380, 40),
    ];

    #[test]
    fn the_controls_go_to_their_corners_in_a_wide_view() {
        let placement = place(1000, 700, &controls(&EVERYDAY));
        let scene = placement.get(Floating::Scene).unwrap();
        assert_eq!((scene.x, scene.y), (EDGE, EDGE));
        let camera = placement.get(Floating::Camera).unwrap();
        assert_eq!((camera.right(), camera.y), (1000 - EDGE, EDGE));
        // The slider pops out to the left of Pull apart, at the column's foot.
        let slider = placement.get(Floating::PullSlider).unwrap();
        assert_eq!(slider.right(), camera.x);
        assert_eq!(slider.bottom(), camera.bottom());
        let motion = placement.get(Floating::Motion).unwrap();
        assert_eq!((motion.right(), motion.bottom()), (1000 - EDGE, 700 - EDGE));
        let moves = placement.get(Floating::Moves).unwrap();
        assert_eq!((moves.x, moves.bottom()), (EDGE, 700 - EDGE));
        assert_apart(&placement);
    }

    #[test]
    fn nothing_covers_anything_in_a_narrow_view() {
        // The chips and the motion slider never show together: while the
        // selection's slider shows, the chips step aside.
        for shown in [Floating::Moves, Floating::Motion] {
            let mut items: Vec<_> = EVERYDAY
                .iter()
                .copied()
                .filter(|(item, _, _)| {
                    !matches!(item, Floating::Moves | Floating::Motion) || *item == shown
                })
                .collect();
            items.push((Floating::Hint, 260, 20));
            items.push((Floating::EdgePin, 200, 32));
            let placement = place(480, 420, &controls(&items));
            assert_apart(&placement);
        }
    }

    #[test]
    fn the_chips_go_above_the_motion_slider_when_they_cannot_sit_beside_it() {
        let placement = place(600, 700, &controls(&EVERYDAY));
        assert_apart(&placement);
        let moves = placement.get(Floating::Moves).unwrap();
        let motion = placement.get(Floating::Motion).unwrap();
        assert!(moves.bottom() <= motion.y);
    }

    #[test]
    fn the_slider_goes_under_the_camera_buttons_when_there_is_no_room_beside_them() {
        let placement = place(200, 600, &controls(&EVERYDAY));
        let camera = placement.get(Floating::Camera).unwrap();
        let slider = placement.get(Floating::PullSlider).unwrap();
        assert!(slider.y >= camera.bottom());
        assert!(slider.width <= 200 - 2 * EDGE);
    }

    #[test]
    fn a_problem_card_sits_above_the_foot_in_the_middle() {
        let placement = place(
            1000,
            700,
            &controls(&[
                (Floating::Camera, 40, 132),
                (Floating::ProblemCard, 500, 90),
                (Floating::EdgePin, 200, 32),
                (Floating::ContactKey, 220, 28),
            ]),
        );
        let card = placement.get(Floating::ProblemCard).unwrap();
        assert_eq!(card.width, CARD_WIDTH);
        assert_eq!(card.bottom(), 700 - EDGE);
        assert_eq!(card.x, (1000 - CARD_WIDTH) / 2);
        let pin = placement.get(Floating::EdgePin).unwrap();
        assert!(pin.bottom() <= card.y);
        let key = placement.get(Floating::ContactKey).unwrap();
        assert_eq!(key.y, EDGE);
        assert_apart(&placement);
    }

    #[test]
    fn hidden_controls_are_not_placed() {
        let placement = place(800, 600, &controls(&[(Floating::Camera, 40, 132)]));
        assert_eq!(placement.get(Floating::Motion), None);
        assert_eq!(placement.get(Floating::Scene), None);
    }
}
