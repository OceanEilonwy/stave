# Work pack: graphical editing from the 3D view (2026-11-18 to 2026-11-19)

A record, not a plan: what was built so a design can be made and changed
in the 3D view, the decisions it was built to, what was left out on
purpose, how it was accepted and what is still wrong. The specification
is [`docs/editing.md`](../editing.md) (the editor) and
[`docs/grammar.md`](../grammar.md) (the language it writes); the window
is [`docs/gui.md`](../gui.md), "Editing in the 3D view".

## Ground rules it was built under

- Parallel lanes in their own worktrees, merged wave by wave into
  `edit/integration` (wave 0, `22c9500`), `edit/integration-w1`
  (`6fe1fe1`), `-w2` (`fdfadf8`), `-w3` (`7655e9f`) and finally
  `edit/graphical-editing` (wave 4, then `main`'s docked window merged
  in at `0a4894e`). Never pushed; `main` never touched.
- Spec first: each language change landed in `docs/grammar.md` with its
  code. Headless first: every behaviour is in `stave-app` and tested
  without a window; `stave-gtk` draws and forwards.
- Gate before every commit: `cargo fmt`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`.
- Every `stock` in a test design has `max_length:`; tests build their own
  inputs or use files their lane owns.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## What was built

| Wave | Built | Where |
|---|---|---|
| 0 | The spec (`docs/editing.md`, the grammar's param kinds, `box`, `shares:`, place params, smart edits, fixes as patches); applying a patch to the lossless tree (`add`, `set`, `unset`, `remove`); the two check tiers (`check_fast`: layers 1 to 3 and box overlap; `check_slow` with cancellation); `Prepared::reopen`; the editor's contract types; `std:casework` begun (Drawer, FramePanelDoor) | `docs/`, `stave-lang/src/patch/`, `stave-check`, `stave-app/src/edit.rs`, `std/casework/` |
| 1 | `locked param` / `solved param` as a prefix; `box(across:, up:, deep:)` as the root; `split(..., shares: [...])`; component params of type `Part`, `Parts`, `Anchor`; `replace` patches and fixes carried on a `Diagnostic`; Back, Divider, Shelf, HangingRail, Feet; listing candidates by signature, patch synthesis and the new-project writer; naming; the scratch copy with verdict and commit; `stave edit --patch`; face, region and facing-pair picks in the scene | `stave-lang`, `stave-model`, `stave-solve`, `std/casework/`, `stave-app/src/candidates/`, `naming.rs`, `scratch.rs`, `stave-scene` |
| 2 | The param-kind checks; motion and out-of-range checks that offer fixes as patches; `Case` and `std:frames` (Top, Legs, Apron, Stretcher, Cleat, Slats); `examples/*_from_box` proved equal to the hand-written pieces; the edit in progress in the session; repeat, shares, mirror, hold, remove, rename as pure functions; ghosts with a verdict; how a size depends on params (`trace.rs`) and single-param inversion; `stave-smt`, an exact deterministic solver over z3; the window's menu and card widgets | `stave-check`, `std/`, `examples/`, `stave-app/src/editing.rs`, `repeat.rs`, `hold.rs`, `remove.rs`, `stave-model/src/trace.rs`, `stave-solve/src/trace.rs`, `crates/stave-smt/`, `stave-gtk/src/ui/edit_menu.rs`, `edit_card.rs` |
| 3 | Changes of the design in the session through one safe writer; refusals carry fixes; the contract carries names, value texts, options, ranges, prompts and rendered problems; `stave edit --script`, with the cabinet and the side table built by script from an empty project and pinned (`tests/edit-scripts/`); grips and stops (`grips.rs`, `stops.rs`); solving for solved params (`solve_edit.rs`); the window on the real session: lit spaces, menu and card, ghosts, fixes, Add and Cancel, the New Project dialog, `--edit-script` | `stave-app/src/change.rs`, `stave-cli/src/script.rs`, `stave-app/src/grips.rs`, `stops.rs`, `solve_edit.rs`, `stave-gtk/src/ui/edit_view/`, `new_project.rs` |
| 4 | Grips and the solver reachable from the session; the wave 3 defects (a kind on a face placed, side names not doubled, shares as small numbers, the menu's greyed entries folded, defaults printed by value, a part spaced along a span, hold offering the measured distance, the empty sidebar, `Case` not offered inside a case); in the window: grips (drag, type, stops, follows, "Let the solver move this"), repeat (stepper, count handle, boundary handles), mirror, hold, remove with dependants, rename, the finder's verbs, pending lines in the editor | `stave-app/src/gripping.rs`, `stave-gtk/src/ui/edit_view/` |
| Acceptance | Both pieces built from an empty project in the window (below); three defects fixed and two script steps added on the way | `tools/window-edits/acceptance/`, `docs/screenshots/window/accept_*.png` |

The branch is `main` plus 96 commits, 350 files, about 62,600 lines added.

## Decisions taken

From the owner (2026-11-18), all in force:

1. **Param kinds are a prefix**: `param`, `locked param`, `solved param`;
   `private` first; a solved param needs `within`; a kind only on a
   quantity param with a default. Locked: smart edits never change it.
   Plain: changed only when it is the single candidate or the user picks
   it. Solved: z3 may solve for it.
2. **The envelope**: `box(across:, up:, deep:)` makes a region from three
   lengths; exactly one root, a `ground` part or a `box`; the box's bottom
   is the floor.
3. **`split` takes `shares:`**, one per element, positive, not with `at:`.
4. **Component params may be `Part`, `Parts` or `Anchor`**, given in an
   assembly as a part, a list of parts and a face.
5. **Smart edits** rewritten for the three kinds: plain params are offered
   to pick from; only solved params are solved for, with every `require`
   and `within` kept, minimising (in order) how many change, how far, and
   how far from the thing dragged.
6. **Fixes are patches** a diagnostic's help can carry.
7. **Strict commit**: an edit that causes a checker error is refused, with
   the error and its fixes on the card; warnings never block.
8. **Generated text uses assemblies**; grips edit component params.
9. **Signature matching**: the selection becomes typed values; place
   params are filled by pointing, setting params on the card; each
   candidate is tried in a scratch copy and its own `require` lines are
   its applicability; labels, hints and groups come from the declaration.
10. **Naming rules**: only `overall_width`, `overall_height`,
    `overall_depth` and `region envelope` are made unasked; a drag writes
    a number; assemblies by component name with `_2`, `_3`; labelled
    arrays for 2 or 3, counted for 4 or more; `<name>_bay`; mirrored
    pairs `<name>_left`/`_right`; held rules `<a>_to_<b>_min|max`; one
    file until the user splits it; every name can be retyped.
11. **Repeat, mirror, hold, remove** as `docs/editing.md` says.
12. **Grips** move plain numbers first; a size that is an expression shows
    what it follows; the solver over solved params comes after.
13. **Verdict speed**: the fast tier while the pointer moves, the slow
    tier at rest (cancelled by the next move); Add waits for the slow tier.

Taken by the waves and kept by the owner:

- No new diagnostic codes: new messages reuse existing ones.
- Parts may lie outside the envelope; the box is the root and the floor,
  not a limit.
- An error the design already had does not refuse an edit; only errors
  the edit causes do.
- One element of an array cannot be set by a patch (`drawer[top].slide`).
- The fast tier is layers 1 to 3 plus box overlap.
- `std:casework` and `std:frames` are standard modules outside the
  prelude (`import "std:casework"`, `casework.Drawer`); library
  declarations are numbered before the project's.
- A click on a face offers kinds as well as components.
- A project whose base design does not solve cannot be edited by mouse.

Taken in the acceptance (this record):

- A click fills the first open place param it can **in declaration
  order** (the order the card lists them), so a face fills a `Part` that
  comes before an `Anchor` (`editing::bind_further`; the spec already
  said "the next click fills it").
- "And the other side" on a single piece filling a whole space, with no
  mirror image in the design, makes it a mirrored pair across
  (`MirrorSplit`).
- A check that can't open the folder (an empty folder) is told to the
  window as a changed result, so the view leaves "Checking…".

## Left out on purpose

Each is a later decision, not an oversight:

- component warnings ("Case wants a back");
- a `fill` size;
- turned or tapered legs (the cabinet's `Feet` are turned by a recipe of
  the library, not a new shape);
- automatic linking of repeated numbers;
- applicability guards beyond a component's own `require` lines.

The brief also left z3 out of the first build; the orchestrated waves
built it anyway (`stave-smt` in wave 2, the solver in the session and the
window in waves 3 and 4), as each wave's task said.

## Acceptance

The plan's test: both pieces built from an empty project **in the
window**. Done through the window's scripted input (`tools/window-shot`
with `--edit-script`: the same commands the pointer, the card and the
keys send, on the real GTK window drawn by a private Broadway server, no
screen needed), one run a stage, a picture before and after each Add:

```
cargo build -p stave-gtk -p stave-cli
tools/window-edits/acceptance/run            # both; or: run cabinet | run table
```

Result: the projects the window writes are **byte-identical** to the
pinned `tests/edit-scripts/clothing_cabinet.stave` and
`tests/edit-scripts/side_table.stave` (so the same parts by stock and cut
size), each checking with 0 errors and 0 warnings. The pictures are
`docs/screenshots/window/accept_cabinet_*.png`, `accept_table_*.png` and
`accept_new_project_dialog.png`.

Not done in the window:

- **The New Project dialog** opens and is pictured, but no script step
  fills its fields, and it takes **one wood**: the cabinet needs six
  stocks, the table two. Both projects were created by `stave edit
  --script` (`new`/`stock`/`create`), which uses the dialog's writer
  (`change::new_project_file`). By hand, the extra stocks would be typed
  into the text.
- **The cleat's `start`** (`apron_front.apron.left`, the apron's end) is
  inside the leg's mortise and cannot be clicked from any side; it was
  filled with the script's `bind`, which stands for a click without
  making one.
- Faces out of sight were reached by orbiting the view (a new `turn`
  step, or `--turn`/`--pitch` for a stage), as a person would; the top's
  underside was not reached from the camera and its top face was clicked
  instead (any face of a part gives the part).

Fixed on the way:

1. An empty folder left the window on "Checking…" for ever (and
   `window-shot` timed out with no picture): `Session::finish_check` now
   reports a changed result when the folder can't be opened (asserted in
   `stave-app/tests/session.rs`, `a_folder_that_cant_be_read_says_so`).
2. A face click filled the first open `Anchor` before an earlier `Part`:
   the apron's top went into `outside` and the apron was refused
   (`axis-set-twice`). Now declaration order (unit test
   `editing::tests::a_face_fills_the_first_place_the_card_asks_for`).
3. A door filling a space had no gesture to become a mirrored pair: "And
   the other side" was hidden for it and, when sent, refused. Now it
   sends `MirrorSplit` across (no automated test: it is window code; the
   acceptance run is its proof).

Added to the window's script steps: `save` (Ctrl+S) and `turn <degrees>
[<pitch>]` (orbit).

## Known defects

Seen in the acceptance run; none blocks building either piece.

- **Add into an open file is unsaved.** Selecting an item opens its file
  in the Code panel; a change made then (mirror, a grip, hold) goes into
  that buffer unsaved (the tab shows "•") and is written only on Ctrl+S,
  while an Add from the menu with no file open writes to disk at once.
  Documented, but it surprises.
- **A mirror is offered again** on every item that already has one:
  `door_left` offers "And the other side · adds door_2_right"
  (`offer mirror door_left` in `stave edit` names `door_2_left` and
  `door_2_right`), and so do `apron_front` (`apron_2_back`), `apron_left`
  (`apron_2_right`), `cleat_front` and `stretcher_front`.
- **The mirrored-pair switch is offered too widely** (a side effect of
  fix 3): any single piece that fills a whole space and has no mirror
  image shows "And the other side · splits its space across into a
  mirrored pair", so the Top, the Case, the Divider and the Shelf offer
  it, where a mirrored pair makes no sense. Turned on there, it would
  split the space and pair them; nothing stops it but the check.
- **Grip labels pile up**: after Add, the selected assembly's grip
  labels overlap each other ("80mm · locked: follows numbers only" eight
  times on the feet; "follows" labels wrapped into tall narrow boxes) and
  are drawn over the cards (the hold card's title is covered).
- **A typed grip value** opens a small panel ("3mm (was 5mm)", Apply)
  drawn over the change card's title, with a second Set button below.
- **Details panel**: the "CHANGE BACK" heading is drawn over the
  "Component" row of the account above it.
- **Prompts**: library place params have `//` comments, not `///`, so the
  card says "Point at the under." / "Point at the start." The card shows
  raw param names (`stands_on`, `panel_stock`).
- **Tree**: joints' pins read "base_joints's pin[2]".
- After the drawers' Add nothing is selected; after other Adds the new
  item is.
- With the New Project dialog over an empty folder, the Parts panel says
  "No folder open".
- The menu "In this space" on the empty envelope offers Divider, Drawer
  and FramePanelDoor (needing a choice), and a space inside the case
  offers `hardware.side_mount_slide` ("needs a choice: piece").
- A mirrored pair split up or deep has no gesture (only across).
- The dialog takes one wood and has no script step (above).

## Lines `CLAUDE.md` needs

`CLAUDE.md` belongs to the owner and was not edited. It would need:

In "Crate boundaries", after the `stave-ide` paragraph:

```
`stave-smt` (the exact solver for smart edits, over z3) sits beside that
chain at the bottom: it depends on no other Stave crate, is the only crate
that may depend on z3, and only `stave-app` depends on it.
```

In "Sources of truth", after the `examples/`, `std/` line:

```
- `std/casework/` (`std:casework`: Case, Back, Divider, Shelf,
  HangingRail, Feet, Drawer, FramePanelDoor) and `std/frames/`
  (`std:frames`: Top, Legs, Apron, Stretcher, Cleat, Slats) — the
  libraries the 3D editor offers. They are standard modules outside the
  prelude (`import "std:casework"`); nothing in the editor names them.
- `docs/editing.md` — the 3D editor's specification.
```
