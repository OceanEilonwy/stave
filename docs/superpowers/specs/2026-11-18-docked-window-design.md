# The docked window

Design approved 2026-11-18 (canvas "Stave docked layouts", boards B and B2).

## Why

Trying the V2 spatial window, the user found that what floats over the 3D
view gets in the way: the card covers the model, the tree is an overlay,
the editor slides in and out awkwardly, and the rail of open files has a
"+" that makes no sense. The simplification was right in direction, but a
traditional docked window (as in Visual Studio) suits Stave better:
nothing covers the model, and every panel has a place.

## Goals

- Panels docked round the 3D view, never over it, with libpanel
  (`libpanel-1` 1.10, crate `libpanel` 0.7), so they can be dragged to
  another edge or into another panel's tabs, closed, and brought back.
- Default layout (board B): left column Parts (tabbed with Files) above
  Details; the 3D view in the centre with its tools in a fixed strip under
  it; a bottom panel holding the Code panel's tabs and Problems; the right
  edge empty and hidden.
- The **Code panel** decides where files open: a new file opens as a tab
  in whichever frame the Code panel sits in (B2: dragged to the right, new
  files open on the right). With no file open it shows a placeholder
  ("Open a file from Files or Ctrl+K") so it never vanishes.
- The status bar has a toggle per edge (left, bottom, right).
- The layout is remembered per user in `~/.config/stave/` (never in a
  `.stave` file); the main menu has "Reset layout".
- Behaviour fixes from the same review, done first:
  1. The header bar is solid.
  2. Esc closes the finder.
  6. Clicking a part in the 3D view selects it without moving the camera;
     clicking in the parts list (or the finder, or "show in 3D") frames it.
  7. The mouse's Back and Forward buttons (8 and 9) go back and forward.
  8. Clicking a lens ("2 uses", "show in 3D") above a line in the editor
     does not select text.
  9. X-ray changes only when its button is pressed: the window never
     switches it on or off by itself.

## Non-goals

- No change to `stave-app`'s session contract beyond what the fixes need.
- No new views (the cut list stays the header's switch).
- No floating windows torn off the main window.

## Anti-goals

- Anything drawn over the 3D view other than the problem pins on the model.
- Layout stored in project files.

## Constraints

- Only `stave-gtk` depends on libpanel (CLAUDE.md: only `stave-gtk` may
  depend on GTK, libadwaita or wgpu; libpanel is a GTK library).
- `tools/window-shot` must still produce deterministic screenshots, so the
  saved layout is not read when taking screenshots.

## Approaches

1. **Fixed docked layout with `gtk::Paned`**: small and solid, no new
   dependency, but panels cannot be moved. Rejected by the user: they want
   panels they can rearrange.
2. **libpanel** (chosen by the user): `PanelDock` with start, end, bottom
   areas and a centre; `PanelFrame`s holding `PanelWidget`s as tabs;
   drag-to-rearrange and placeholders built in.
3. **Hand-made docking** on `gtk::Paned` and drag-and-drop: all the work
   libpanel already did. Not considered further.

Three layouts were mocked (A Workbench, B Model first, C Activity bar); the
user chose B with an optional right panel (B2).

## Left to the builder

- How the saved layout is written (libpanel's `PanelSession`, or a small
  record of which area each panel is in if that proves unreliable).
- Exact panel sizes.
