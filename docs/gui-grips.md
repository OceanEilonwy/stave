### Grips

*(A section for [`gui.md`](gui.md), "Editing in the 3D view"; kept in a
file of its own until the two are merged.)*

[Grips](editing.md#grips) are drawn on the model by
`ui/edit_view/grips.rs` from what the session lists
(`EditCommand::ListGrips`, `Session::grips`) and moved with the
session's drag commands (`BeginDrag`, `DragTo`, `PickParam`,
`TypeGripValue`, `MakeSolved`, `MakePlain`, `Add`, `Cancel`). The window
decides nothing: what a grip moves, where it snaps, what stops it, the
verdict and the text written are the session's. It turns the pointer's
travel along the handle on screen into a value to send, and draws what
the session holds.

- **What has grips.** Selecting an assembly (in the tree, by its chip
  in the dock, with the finder, or by clicking one of its parts on the
  model) shows the grips of its top-level assembly. A click in the
  envelope shows the envelope's three sizes beside the menu "In this
  space"; a click in one piece of a split (`drawer_bay[bottom]`) shows
  the boundaries between its pieces. Selecting something else, or
  letting the space's menu go, takes them away. While an assembly's
  grips show, its card steps aside (the tree still says what is
  selected); a part's card stays, to the right of the grips.
- **How a grip is drawn** (`grip_text::grip_look`), never by colour
  alone: each kind has its own handle, line and words.
  - A plain number that moves: a round handle on the face that moves,
    a solid dimension line with arrows from the face it is measured
    from, and its value on the line ("500mm"; "1780mm · solved" for a
    solved param).
  - A size that follows params: a square handle, a dashed line, and
    what it follows ("862mm · follows overall_width, case.overhang"),
    each param that stops the chain named with its kind ("oak_19.thickness
    (a size of the stock)").
  - A size nothing can move (it follows only locked params, stock or
    hardware sizes, or its stops are both at its value): a padlock in
    place of the handle, a dotted line, and why ("locked: follows
    oak_45_sq.width (a size of the stock)").

  Where the dimension sits comes from the scene's `grip_place`: a
  component's length between the two faces it is measured between (or,
  when another length of the assembly has the same value, the faces its
  placement term names); a part's length or width and the envelope's
  sizes along the box's edge nearest the eye whose end no other handle
  already holds (so the envelope's width, height and depth end on three
  different corners and each handle can be grabbed); a split's boundary
  at the boundary. A grip with no direction in space (a count, an angle,
  a length that is no gap) has no place on the model. Labels are moved
  along their line, to either side of it or on past the handle, so they
  cover no handle, no other label and no floating card (the space's
  menu, the edit's card), and kept inside the view.
- **Stops.** Once listed, every movable grip's stops are searched on a
  worker thread (`Session::grip_stops_job`, cancelled when the grips
  change), each answer handed to `Session::grip_stops_found` as it
  comes; a grip's range narrows to its two last good values.
- **A drag.** Pressing a handle starts a drag (`BeginDrag`); a drag
  already open on that grip goes on from where it is. Moving it sends
  `DragTo` with the value under the pointer: snapped to the grip's steps
  (the project's `resolution:`, or a `within` list's values), or to
  round numbers (10mm, 5°) while **Ctrl** is held, and held inside its
  range. Only the latest value waits to be sent while one is tried. The
  handle and its number follow the value, the other grips step aside,
  the ghost shows the design as it would be (tinted by the verdict, with
  "✓ It fits" beside it), and beside the dimension its range is drawn:
  a pale track between the two last good values, a rule's stop as a hard
  red end hatched beyond it (as the door's swing slider draws where the
  door first hits), the end of a `within` as a hard grey end. The panel
  beside the handle (`GripPanel`) says what is moved and to what
  ("700mm (was 500mm)"), the verdict in words, and, held at a stop, the
  rule that stops it ("stops at 180mm, because at 179.5mm
  'drawer[bottom].box_is_useful' fails: the drawer box is too shallow;
  make the opening taller"). Letting go writes it (`Add`) when it fits;
  otherwise the panel keeps Apply (enabled only on a fit) and Cancel.
  **Escape** cancels. With the keyboard, a focused handle moves a step
  with the arrow keys (Ctrl: round numbers) and Enter writes it.
- **A typed value.** Clicking the number turns it into an entry. Enter
  tries the value through the same check (`TypeGripValue`) and writes it
  when it fits. What is written is always the number typed, never
  another: a value outside the grip's range (its stops once found;
  before, its `within`, or how far a grip reaches, 1000mm either way),
  or off the project's resolution, is refused before it is tried ("Can't
  be 2900mm: this grip moves it between 780mm and 2780mm", "Can't be
  912.3mm: it goes in steps of 0.5mm; the nearest is 912.5mm"), and a
  try the session would write as another number is refused and dropped.
  A value the check refuses is not written either. Each time the entry
  stays open, marked, holding what was typed, with why under it ("Can't
  be 120mm: it stops at 180mm, because …"). Escape puts the number back
  and drops what was tried.
- **A size that follows params.** Dragging its square handle changes
  what it follows. With one plain param the panel says which and to
  what before the drag ends ("Changes overall_width 500mm to 560mm")
  and the ghost moves everything that follows it. With several, pressing
  the handle lists them at once, and each shows the value it would take
  as the drag goes ("overall_width 500mm to 520mm", "legs.inset 25mm to
  15mm"); nothing is tried until one is picked (`PickParam`), then the
  drag goes on with that one. When solved params are involved
  the solver chooses ("The solver changes overall_height 600mm to
  579mm"), and a value it can't reach is refused naming the rule that
  blocks it.
- **Letting the solver move it.** The ⋮ button beside a number opens
  "Let the solver move this", for a param of the project (the grip's
  own, or each one a size follows; a component's param can't change
  kind). Turning it on asks for the range the solver keeps it within
  (a fifth either side of the value, to retype) and **Make it solved**
  sends `MakeSolved`; turning it off on a solved param sends
  `MakePlain`. The panel shows the change's verdict, and Apply writes
  it. After any commit the grips are listed again from the new design.

The edit script steps for grips (`ui/edit_view/grip_script.rs`) do the
same things without a pointer, grips numbered from 1 as `stave edit`
numbers them:

| Step | What it stands for |
|---|---|
| `grips assembly <name>` | the assembly selected: its grips on the model |
| `grips envelope`, `grips split <piece>` | the envelope's grips, or a split's boundaries (as a click in the space shows them) |
| `stops` | every grip's stops found now |
| `drag <grip> <value>` | the handle dragged to a value, still held |
| `release` | the handle let go: written when it fits |
| `type <grip> <value>` | the number clicked, the value typed, Enter pressed |
| `pick <param>` | a size from the list a follows grip offers |
| `menu <grip>` | the grip's ⋮ menu opened (a popover: not in a screenshot) |
| `solver <grip> <low> <high>`, `plain <grip>` | the solver's switch turned on with that range, or off |

Pictures (`docs/screenshots/window/`), each from an edit script in
`tools/window-edits/` on a copy of `examples/clothing_cabinet_from_box`
or `examples/side_table_from_box`:

| Picture | Script | Shows |
|---|---|---|
| `grip_divider.png` | `grip_divider.edit` (`--select divider`) | the divider's grips: its height (round handle, "500mm"), its panel's length and width (square handles, what they follow) |
| `grip_drag.png` | `grip_drag.edit` (`--select divider`) | the height dragged to 700mm and held: the ghost, "✓ It fits", the range with its red stop at 180mm, the panel |
| `grip_stop.png` | `grip_stop.edit` (`--select divider`) | dragged down past its stop: held at 180mm, the panel naming the rule |
| `grip_typed_refused.png` | `grip_typed_refused.edit` (`--select divider`) | 120mm typed: the entry stays open with why |
| `grip_follows.png` | `grip_follows.edit` (`--select top`) | the table top's length dragged: "Changes overall_width 500mm to 560mm", everything that follows it in the ghost |
| `grip_follows_pick.png` | `grip_follows_pick.edit` (`--select apron_front`) | the apron's length, which follows two plain params: the list to pick from |
| `grip_envelope.png` | `grip_envelope.edit` | a click in the envelope: the menu and the three sizes along the box's edges, each handle on its own corner, each label clear of the menu |
| `grip_solver.png` | `grip_solver.edit` | the height made solved within 1500mm..2000mm, waiting for Apply |

```
tools/window-shot grip_drag.png <copy of examples/clothing_cabinet_from_box> --select divider \
    --edit-script tools/window-edits/grip_drag.edit
```

`release` and `type` write when the value fits: point them at a copy.

**Not in the window yet.** A grip with no direction in space (a count,
an angle, a length that is no gap, `divider.depth`) is listed by the
session but not drawn, so it can't be dragged or typed in the window;
the command line can (`grip <n> = <value>`). Grips of a nested assembly
on their own (only the top-level assembly's are listed), and of a whole
group of parts, are not offered. While pulled apart, in focus mode, on
another view, or while a menu entry's card is open, grips are hidden. The
⋮ menu is a popover, which the screenshot tool can't draw. The solver's
switch is offered for the project's own params only.
