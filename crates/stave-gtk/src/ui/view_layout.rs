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
//!
//! Four of them are tool groups people can press, hold and drag to
//! another spot ([`Group`], [`Spot`]): the scene menu, the camera column,
//! the chips and the motion slider. [`Spots`] says which group is in
//! which spot (one per spot; dropping a group on another's spot swaps
//! them), [`Spots::free_for`] which spots a dragged group may land in and
//! [`nearest`] which one it lands in. Groups lie flat in the corners and
//! stand upright on the right edge.

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

/// One of the tool groups that can be pressed, held and dragged to
/// another spot on the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// The scene menu ("As built").
    Scene,
    /// The camera buttons, X-ray and Pull apart.
    Camera,
    /// The moving assemblies' chips.
    Moves,
    /// The selection's motion slider.
    Motion,
}

impl Group {
    /// Every group.
    pub const ALL: [Group; 4] = [Group::Scene, Group::Camera, Group::Moves, Group::Motion];

    /// The word the layout file uses.
    pub fn word(self) -> &'static str {
        match self {
            Group::Scene => "scene",
            Group::Camera => "camera",
            Group::Moves => "moves",
            Group::Motion => "motion",
        }
    }

    /// The group a word names.
    pub fn from_word(word: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|group| group.word() == word)
    }

    /// The control the group is.
    pub fn floating(self) -> Floating {
        match self {
            Group::Scene => Floating::Scene,
            Group::Camera => Floating::Camera,
            Group::Moves => Floating::Moves,
            Group::Motion => Floating::Motion,
        }
    }

    /// The group a control is, if it is one.
    pub fn of(item: Floating) -> Option<Group> {
        Group::ALL
            .into_iter()
            .find(|group| group.floating() == item)
    }
}

/// A spot a tool group can be docked in: a corner, where it lies flat, or
/// the right edge, where it stands upright.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Spot {
    /// The top left corner.
    TopLeft,
    /// The bottom left corner.
    BottomLeft,
    /// The bottom right corner.
    BottomRight,
    /// Down the right edge, from the top.
    Right,
}

impl Spot {
    /// Every spot.
    pub const ALL: [Spot; 4] = [
        Spot::TopLeft,
        Spot::BottomLeft,
        Spot::BottomRight,
        Spot::Right,
    ];

    /// The word the layout file uses.
    pub fn word(self) -> &'static str {
        match self {
            Spot::TopLeft => "top-left",
            Spot::BottomLeft => "bottom-left",
            Spot::BottomRight => "bottom-right",
            Spot::Right => "right",
        }
    }

    /// The spot a word names.
    pub fn from_word(word: &str) -> Option<Spot> {
        Spot::ALL.into_iter().find(|spot| spot.word() == word)
    }

    /// `true` for the right edge, where a group stands upright.
    pub fn is_upright(self) -> bool {
        match self {
            Spot::Right => true,
            Spot::TopLeft | Spot::BottomLeft | Spot::BottomRight => false,
        }
    }
}

/// Which spot each tool group is docked in: always one group per spot,
/// so two groups can never be docked on top of each other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spots {
    /// Indexed like [`Spot::ALL`]: the group in each spot.
    groups: [Group; 4],
}

impl Default for Spots {
    /// The scene menu at the top left, the camera column down the right,
    /// the chips at the bottom left and the motion slider at the bottom
    /// right.
    fn default() -> Spots {
        Spots {
            groups: [Group::Scene, Group::Moves, Group::Motion, Group::Camera],
        }
    }
}

impl Spots {
    fn index(spot: Spot) -> usize {
        match spot {
            Spot::TopLeft => 0,
            Spot::BottomLeft => 1,
            Spot::BottomRight => 2,
            Spot::Right => 3,
        }
    }

    /// The group docked in a spot.
    pub fn group_at(&self, spot: Spot) -> Group {
        self.groups
            .get(Spots::index(spot))
            .copied()
            .unwrap_or(Group::Scene)
    }

    /// The spot a group is docked in.
    pub fn spot_of(&self, group: Group) -> Spot {
        Spot::ALL
            .into_iter()
            .find(|spot| self.group_at(*spot) == group)
            .unwrap_or(Spot::TopLeft)
    }

    /// Docks a group in a spot; the group that was there takes the spot
    /// the moved one left, so every spot still holds one group.
    pub fn dock(&mut self, group: Group, spot: Spot) {
        let from = self.spot_of(group);
        let other = self.group_at(spot);
        if let Some(slot) = self.groups.get_mut(Spots::index(from)) {
            *slot = other;
        }
        if let Some(slot) = self.groups.get_mut(Spots::index(spot)) {
            *slot = group;
        }
    }

    /// The spots a dragged group may be dropped in: every spot but its
    /// own that no showing group holds (`showing` says which show).
    pub fn free_for(&self, group: Group, showing: impl Fn(Group) -> bool) -> Vec<Spot> {
        let own = self.spot_of(group);
        Spot::ALL
            .into_iter()
            .filter(|spot| *spot != own && !showing(self.group_at(*spot)))
            .collect()
    }
}

/// Where a group of `size` (as it would be in that spot) goes in a spot
/// of a view `width` by `height`: the dashed outline shown while
/// dragging.
pub fn target(width: i32, height: i32, spot: Spot, size: (i32, i32)) -> Rect {
    let (w, h) = size;
    let (x, y) = match spot {
        Spot::TopLeft => (EDGE, EDGE),
        Spot::BottomLeft => (EDGE, height - EDGE - h),
        Spot::BottomRight => (width - EDGE - w, height - EDGE - h),
        Spot::Right => (width - EDGE - w, EDGE),
    };
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// The target nearest a point (by the distance to its middle), where a
/// dragged group lands when let go.
pub fn nearest(point: (i32, i32), targets: &[(Spot, Rect)]) -> Option<Spot> {
    let (x, y) = (i64::from(point.0), i64::from(point.1));
    targets
        .iter()
        .min_by_key(|(_, rect)| {
            let middle_x = i64::from(rect.x) + i64::from(rect.width) / 2;
            let middle_y = i64::from(rect.y) + i64::from(rect.height) / 2;
            (middle_x - x).pow(2) + (middle_y - y).pow(2)
        })
        .map(|(spot, _)| *spot)
}

/// Places Pull apart's slider beside the camera group: out to the left of
/// its foot when the group stands on the right edge (or under the column
/// when there is no room), under it in the top corner, above it in a
/// bottom corner. Returns the slider's place.
fn place_slider(
    placement: &mut Placement,
    measure: &impl Measure,
    camera: Rect,
    spot: Spot,
    room: i32,
) -> Option<Rect> {
    let (smallest, natural) = measure.widths(Floating::PullSlider)?;
    match spot {
        Spot::Right => {
            let beside = camera.x - EDGE;
            let slider_width = natural.max(smallest);
            if beside >= slider_width.min(NARROWEST_SLIDER.max(smallest)) {
                let w = slider_width.min(beside);
                let h = measure.height(Floating::PullSlider, w);
                Some(placement.put(
                    Floating::PullSlider,
                    Rect {
                        x: camera.x - w,
                        y: (camera.bottom() - h).max(EDGE),
                        width: w,
                        height: h,
                    },
                ))
            } else {
                let (w, h) = size(measure, Floating::PullSlider, room)?;
                Some(placement.put(
                    Floating::PullSlider,
                    Rect {
                        x: camera.right() - w,
                        y: camera.bottom() + GAP,
                        width: w,
                        height: h,
                    },
                ))
            }
        }
        Spot::TopLeft | Spot::BottomLeft | Spot::BottomRight => {
            let (w, h) = size(measure, Floating::PullSlider, room)?;
            let x = match spot {
                Spot::BottomRight => camera.right() - w,
                Spot::TopLeft | Spot::BottomLeft | Spot::Right => camera.x,
            };
            let y = match spot {
                Spot::TopLeft => camera.bottom() + GAP,
                Spot::BottomLeft | Spot::BottomRight | Spot::Right => camera.y - GAP - h,
            };
            Some(placement.put(
                Floating::PullSlider,
                Rect {
                    x,
                    y,
                    width: w,
                    height: h,
                },
            ))
        }
    }
}

/// The smallest rectangle round two.
fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    }
}

/// Places the controls in a view `width` by `height` pixels, each tool
/// group in the spot `spots` gives it.
pub fn place(width: i32, height: i32, measure: &impl Measure, spots: &Spots) -> Placement {
    let mut placement = Placement::default();
    let room = width.saturating_sub(2 * EDGE).max(0);
    let camera_spot = spots.spot_of(Group::Camera);
    let in_spot = |spot: Spot| spots.group_at(spot).floating();

    // Down the right edge: its group, Pull apart's slider when that is
    // the camera column, and the plan switch.
    let mut right_column: Vec<Rect> = Vec::new();
    let mut right_y = EDGE;
    let right_item = in_spot(Spot::Right);
    if let Some((w, h)) = size(measure, right_item, room) {
        let rect = placement.put(
            right_item,
            Rect {
                x: width - EDGE - w,
                y: EDGE,
                width: w,
                height: h,
            },
        );
        right_y = rect.bottom() + GAP;
        right_column.push(rect);
        if camera_spot == Spot::Right
            && let Some(slider) = place_slider(&mut placement, measure, rect, Spot::Right, room)
        {
            right_y = right_y.max(slider.bottom() + GAP);
            right_column.push(slider);
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

    // The top left corner's group (with Pull apart's slider under it when
    // it is the camera group).
    let top_left_item = in_spot(Spot::TopLeft);
    let top_left = size(measure, top_left_item, room).map(|(w, h)| {
        let rect = placement.put(
            top_left_item,
            Rect {
                x: EDGE,
                y: EDGE,
                width: w,
                height: h,
            },
        );
        if camera_spot == Spot::TopLeft
            && let Some(slider) = place_slider(&mut placement, measure, rect, Spot::TopLeft, room)
        {
            union(rect, slider)
        } else {
            rect
        }
    });

    // Along the top, in the middle: the line, then the key.
    let mut top_y = EDGE;
    for item in [Floating::Hint, Floating::ContactKey] {
        let Some((smallest, natural)) = measure.widths(item) else {
            continue;
        };
        let natural = natural.max(smallest);
        // Beside the top left group when it fits, else under it.
        let beside_corner = top_left.filter(|corner| corner.shares_rows(top_y, top_y + 1));
        let mut left = beside_corner.map_or(EDGE, |corner| corner.right() + GAP);
        let mut right = free_right(top_y, top_y + 1);
        if let Some(corner) = beside_corner
            && natural > right - left
        {
            top_y = corner.bottom() + GAP;
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

    // The foot, from the bottom up: the bottom right group, then the
    // bottom left one (beside it when there is room, else above it).
    let mut foot = height - EDGE;
    let mut row_top = foot;
    let right_foot_item = in_spot(Spot::BottomRight);
    let right_foot = size(measure, right_foot_item, room).map(|(w, h)| {
        let right = free_right(foot - h, foot);
        let smallest = measure.widths(right_foot_item).map_or(0, |(s, _)| s);
        let w = w.min(right - EDGE).max(smallest);
        placement.put(
            right_foot_item,
            Rect {
                x: right - w,
                y: foot - h,
                width: w,
                height: h,
            },
        )
    });
    if let Some(rect) = right_foot {
        row_top = rect.y;
    }
    let left_foot_item = in_spot(Spot::BottomLeft);
    let mut left_foot = None;
    if let Some((smallest, natural)) = measure.widths(left_foot_item) {
        let natural = natural.max(smallest);
        let beside = right_foot.map_or(free_right(foot - 1, foot), |m| m.x - GAP) - EDGE;
        let rect = if natural <= beside {
            let h = measure.height(left_foot_item, natural);
            Rect {
                x: EDGE,
                y: foot - h,
                width: natural,
                height: h,
            }
        } else {
            let bottom = right_foot.map_or(foot, |m| m.y - GAP);
            // Wraps onto more rows when it must: its height depends on
            // its width, and its width on the column beside it.
            let mut w = natural.min(room).max(smallest);
            let mut h = measure.height(left_foot_item, w);
            let right = free_right(bottom - h, bottom);
            if right - EDGE < w {
                w = (right - EDGE).max(smallest);
                h = measure.height(left_foot_item, w);
            }
            Rect {
                x: EDGE,
                y: bottom - h,
                width: w,
                height: h,
            }
        };
        if rect.width > 0 && rect.height > 0 {
            let rect = placement.put(left_foot_item, rect);
            row_top = row_top.min(rect.y);
            left_foot = Some(rect);
        }
    }
    // Pull apart's slider above the camera group in a bottom corner.
    let camera_foot = match camera_spot {
        Spot::BottomLeft => left_foot,
        Spot::BottomRight => right_foot,
        Spot::TopLeft | Spot::Right => None,
    };
    if let Some(camera) = camera_foot
        && let Some(slider) = place_slider(&mut placement, measure, camera, camera_spot, room)
    {
        row_top = row_top.min(slider.y);
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
        let placement = place(1000, 700, &controls(&EVERYDAY), &Spots::default());
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
            let placement = place(480, 420, &controls(&items), &Spots::default());
            assert_apart(&placement);
        }
    }

    #[test]
    fn the_chips_go_above_the_motion_slider_when_they_cannot_sit_beside_it() {
        let placement = place(600, 700, &controls(&EVERYDAY), &Spots::default());
        assert_apart(&placement);
        let moves = placement.get(Floating::Moves).unwrap();
        let motion = placement.get(Floating::Motion).unwrap();
        assert!(moves.bottom() <= motion.y);
    }

    #[test]
    fn the_slider_goes_under_the_camera_buttons_when_there_is_no_room_beside_them() {
        let placement = place(200, 600, &controls(&EVERYDAY), &Spots::default());
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
            &Spots::default(),
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
        let placement = place(
            800,
            600,
            &controls(&[(Floating::Camera, 40, 132)]),
            &Spots::default(),
        );
        assert_eq!(placement.get(Floating::Motion), None);
        assert_eq!(placement.get(Floating::Scene), None);
    }

    #[test]
    fn docking_a_group_swaps_it_with_the_one_in_that_spot() {
        let mut spots = Spots::default();
        spots.dock(Group::Camera, Spot::BottomLeft);
        assert_eq!(spots.spot_of(Group::Camera), Spot::BottomLeft);
        assert_eq!(spots.spot_of(Group::Moves), Spot::Right);
        // Every spot still holds exactly one group.
        for group in Group::ALL {
            assert_eq!(spots.group_at(spots.spot_of(group)), group);
        }
        // Docking where it already is changes nothing.
        let before = spots.clone();
        spots.dock(Group::Camera, Spot::BottomLeft);
        assert_eq!(spots, before);
    }

    #[test]
    fn a_dragged_group_may_land_only_where_no_showing_group_is() {
        let spots = Spots::default();
        // The chips step aside while the motion slider shows.
        let showing = |group: Group| group != Group::Moves;
        assert_eq!(
            spots.free_for(Group::Camera, showing),
            vec![Spot::BottomLeft]
        );
        // With only the scene menu and the camera showing, both bottom
        // corners are free.
        let showing = |group: Group| matches!(group, Group::Scene | Group::Camera);
        assert_eq!(
            spots.free_for(Group::Scene, showing),
            vec![Spot::BottomLeft, Spot::BottomRight]
        );
    }

    #[test]
    fn a_group_lands_in_the_target_nearest_the_pointer() {
        let targets: Vec<(Spot, Rect)> = [Spot::TopLeft, Spot::BottomLeft, Spot::Right]
            .into_iter()
            .map(|spot| {
                let size = if spot.is_upright() {
                    (40, 200)
                } else {
                    (200, 40)
                };
                (spot, target(1000, 700, spot, size))
            })
            .collect();
        assert_eq!(nearest((60, 650), &targets), Some(Spot::BottomLeft));
        assert_eq!(nearest((900, 120), &targets), Some(Spot::Right));
        assert_eq!(nearest((300, 80), &targets), Some(Spot::TopLeft));
        assert_eq!(nearest((300, 80), &[]), None);
        let right = target(1000, 700, Spot::Right, (40, 200));
        assert_eq!((right.right(), right.y), (1000 - EDGE, EDGE));
    }

    #[test]
    fn a_group_docked_elsewhere_goes_to_its_new_spot() {
        let mut spots = Spots::default();
        spots.dock(Group::Camera, Spot::BottomLeft);
        spots.dock(Group::Scene, Spot::Right);
        // Lying flat now: the camera group is wide and short.
        let items = [
            (Floating::Scene, 120, 42),
            (Floating::Camera, 236, 40),
            (Floating::PullSlider, 300, 44),
            (Floating::Motion, 380, 40),
        ];
        let placement = place(1000, 700, &controls(&items), &spots);
        let camera = placement.get(Floating::Camera).unwrap();
        assert_eq!((camera.x, camera.bottom()), (EDGE, 700 - EDGE));
        // Pull apart's slider sits above it, at its left.
        let slider = placement.get(Floating::PullSlider).unwrap();
        assert_eq!(slider.x, camera.x);
        assert!(slider.bottom() <= camera.y);
        let scene = placement.get(Floating::Scene).unwrap();
        assert_eq!((scene.right(), scene.y), (1000 - EDGE, EDGE));
        assert_apart(&placement);
        // The camera group in the top left corner, the slider under it,
        // a line along the top beside them.
        let mut spots = Spots::default();
        spots.dock(Group::Camera, Spot::TopLeft);
        let mut items = items.to_vec();
        items.push((Floating::Hint, 300, 20));
        for width in [1000, 520] {
            let placement = place(width, 700, &controls(&items), &spots);
            let camera = placement.get(Floating::Camera).unwrap();
            assert_eq!((camera.x, camera.y), (EDGE, EDGE));
            let slider = placement.get(Floating::PullSlider).unwrap();
            assert!(slider.y >= camera.bottom());
            assert_apart(&placement);
        }
    }

    #[test]
    fn the_words_for_groups_and_spots_read_back() {
        for group in Group::ALL {
            assert_eq!(Group::from_word(group.word()), Some(group));
            assert_eq!(Group::of(group.floating()), Some(group));
        }
        for spot in Spot::ALL {
            assert_eq!(Spot::from_word(spot.word()), Some(spot));
        }
        assert_eq!(Spot::from_word("middle"), None);
    }
}
