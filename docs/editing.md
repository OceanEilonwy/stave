# Editing in the 3D view

How the user builds and changes a design with the mouse, decided with the
owner on 2026-11-18. This is the specification the editor is built
against: `stave-app` holds the behaviour (headless, tested without a
window), `stave-gtk` draws it and forwards what people do, as everywhere
else (see [`gui.md`](gui.md)). The language it writes is
[`grammar.md`](grammar.md); where this file names a language rule, the
grammar is the authority.

The contract types are in `crates/stave-app/src/edit.rs` (`stave_app::edit`):
the typed selection, a candidate, a verdict, a fix and a grip, with a
hand-built fixture for the clothing cabinet. The behaviour is built on
them: signature matching and applicability in `stave_app::candidates`,
the scratch check in `stave_app::scratch`, and the edit in progress, which
joins them, in `stave_app::editing` (see [The edit in
progress](#the-edit-in-progress)).

## The idea

Every gesture produces an ordinary [patch](grammar.md#patches) in the
`.stave` grammar. The patch is applied to a scratch copy of the project,
solved and checked; the result is shown as a ghost with a verdict; it is
committed to the text only if it checks. Text stays the source of truth:
the editor never keeps a design of its own, and what the 3D view shows
is always what the text says, or a ghost of what a patch would make it
say.

The app holds no list of furniture pieces and no rule about what goes
where. What can go in a space, on a face or between two parts comes from
the components and kinds in scope and their own `require` lines
(see [Signature matching](#signature-matching)); what a size may change
comes from the param kinds (see [Smart edits](grammar.md#smart-edits)).

## The edit loop

1. **Point.** The user points at the design. What is under the pointer,
   or selected, becomes typed values ([The selection](#the-selection)).
2. **Propose.** A menu pick, a grip drag or a command makes a
   **candidate**: a patch, with what is still to be filled in.
3. **Try.** The patch is applied to a scratch copy, which is solved and
   checked. The 3D view shows the result as a ghost over the design, and
   the card beside it shows the **verdict**.
4. **Commit or refuse.** If the verdict is *fits*, Add (or letting go of a
   grip) writes the patch into the text through the lossless tree, keeping
   every comment and line break, and the text is checked again as after
   any save. Otherwise nothing is written.

Escape, or moving to another place, drops the candidate. Each commit is
one step of undo, and undo is an edit of the text like any other.

**A design that doesn't place can't be edited by pointing.** When the
project's own design has an error that stops the solver writing a design
(a failing `require`, an unknown name), there is nothing to check a
candidate against, so the edit doesn't start; the editor says so plainly
("fix the errors `stave check` shows first") rather than guessing.

## Verdicts and strict commit

A candidate's verdict is one of three, and nothing else:

| Verdict | Means | The card shows |
|---|---|---|
| Checking | the check for this candidate has not finished | the ghost so far, and a spinner |
| Fits | the candidate checks with no error | the ghost, its warnings if any, and Add |
| Refused | the candidate would cause one or more errors | each error as `stave check` words it, and the checker's fixes |

**Strict commit.** A mouse edit that would cause any checker error is
refused. The card shows the error and the checker's
[fixes](grammar.md#fixes) before anything is written; choosing a fix adds
its patch to the candidate, which is tried again and, once the fix has
made it check, fits. Warnings never block: a candidate with warnings and
no error fits, and the warnings are shown on the card. There is no third
"unfinished" verdict: a candidate that is not yet complete (a place param
still to point at, a setting still to choose) is not tried until it is.

An error the design already had before the edit does not refuse the
edit; only an error the patch causes does. (The editor compares the
candidate's diagnostics with the design's by code, message and place, as
the session carries problems across checks.)

## Verdict speed

Checks are split by cost, as in [Tooling and editors](grammar.md#tooling-and-editors),
with one tier added for the ghost:

| Tier | What | When |
|---|---|---|
| Fast | layers 1 to 3 (syntax, names and types, placement and sizes) and box overlap: two parts whose cut boxes overlap, other than parts that a joint or a piece of hardware joins | on every pointer move, while the pointer moves |
| Slow | the rest of the checks (layers 4 to 7) | when the pointer comes to rest; cancelled by the next move |

While the pointer moves the ghost shows the fast tier: an error there
refuses the candidate at once; otherwise the verdict is *checking*. At
rest the slow tier runs and gives *fits* or *refused*. **The Add button
waits for the slow tier**: it is enabled only on *fits*. Letting go of a
grip commits only once the slow tier says *fits*; until then the grip
stays where it was let go with the card's spinner, and a refusal puts it
back.

## The selection

A selection becomes typed values, the same types as component params:

| What the user points at | Typed value | Card heading |
|---|---|---|
| a click in a space (an opening, a bay, the box) | a `Region` | In this space |
| a click on a face of a part | an `Anchor` (the face) and its `Part` | On this face |
| a click between two facing parts | `Parts`: the two | Between these parts |
| a group selection (several parts) | `Parts`: every part of the group | With these parts |

What is under the pointer is decided by the scene, on what it shows:

- **A part in front of a region wins.** A click goes to a space only
  when the ray passes through empty space inside the region before it
  reaches any part (by more than a hundredth of a millimetre); a part in
  front of the region, or in its front face, is what is clicked. A part
  that stands inside the region (a rail across a bay) is clicked too; a
  part that is the region's wall, or lies beyond it, is not: the space
  is.
- **The innermost region is chosen.** Of the regions the ray looks into
  through empty space, the one it is still inside where that run ends,
  and of those the smallest (ties go to the one the design lists first). A region
  with no room inside (no size on some axis) is never picked. A region
  stays where the design puts it: a scene's motions don't move it.
- **Facing pairs consider only shown parts at rest.** "Between two facing
  parts" is a point in the gap between two parallel faces of two parts
  that face each other, with nothing between them: along each axis, the
  nearest part on either side whose box spans the point on the other two
  axes, and of the axes that have a part on both sides, the one with the
  narrowest gap (ties in the order up, across, deep). Only parts the scene
  shows, and shows where the design puts them, count: a door a scene
  swings open, or a part pulled apart, no longer faces anything. The pair
  is given low side first (left, bottom, front).

A click on a hardware's body or a joint's member gives no selection to
edit from: they are not places to put something.

## Signature matching

The menu for a selection is worked out from the declarations in scope,
never from a list in the app.

1. **Params split by type.** A component's params, and a kind's roles,
   are of two sorts. **Place params** (`Region`, `Anchor`, `Part`,
   `Parts`) are filled by pointing. **Setting params** (`Stock`, `Mount`,
   the kind types, lengths and other quantities) are filled in the card.
2. **Candidates.** A `component` or `kind` in scope is a candidate when
   the selection can fill its place params. Each typed value of the
   selection fills the first place param of its own type in declaration
   order that no other value fills. A `Region` must find a `Region`
   param, and `Parts` (two facing parts, a group) a `Parts` param.
   **A face gives both an `Anchor` and its `Part`, and a declaration is a
   candidate when its place params can be filled from either**: a
   component with an `Anchor` param, a `Part` param or both is offered on
   a face, each param that has a value of its type taking it. For a kind,
   its roles play that part: a role of type `Part` takes one part of the
   selection, a role of type `Parts` takes one or more, and the part
   pointed at first goes to the first role. A kind has no `Anchor` role,
   so on a face its first part role takes the face's part: a knob (a
   `pull` kind whose first role is `piece: Part`) is offered when a door
   stile's face is clicked. A `fastener` or `tool` kind is used by a joint
   or a tool list and never placed by itself, so it is never a candidate.
   Place params the selection doesn't fill are asked for by pointing
   (step 5).
3. **Applicability.** Each candidate is instantiated in a scratch copy
   with its defaults (and, for a setting with no default, the value it
   would get by step 5) and checked. Its own `require` lines are its
   applicability rules. A candidate that fails is still listed, greyed,
   with the checker's message, so the user learns why it doesn't fit. A
   candidate that still has a place param to point at, or a setting with
   no default and nothing to copy, is not tried: it **needs a choice**,
   which is not a refusal, and the entry is listed as usual. Trying never
   changes the design the menu was made for, and the menu can be checked
   a little at a time: a check that is stopped part way keeps the answers
   it reached, and the rest wait.
4. **The menu entry** is drawn from the declaration:

   | Part of the entry | From |
   |---|---|
   | label | its name (`Drawer`, `dado`) |
   | hint | the first sentence of its `///` doc comment |
   | group | its module: the project's own first, then imported modules by name, then `std` |
   | heading | the type of the selection (the table above) |

   Within a group, entries are in name order, greyed ones after the rest.
5. **Filling the rest.** A place param still unbound is asked for by
   pointing: the card names it ("Point at the side the hinges go on") and
   the next click fills it. Setting params with no default are asked for in
   the card: on first use the user chooses; later the value is copied from
   the project's most recent assembly of that component, read from the
   text (the last in file order of the files as the folder lists them).

A pick writes an assembly (see below); a kind writes a `joint` or a
`hardware` with the kind call.

## The edit in progress

The edit in progress is session state (`stave_app::editing::Edit`, held
by `stave_app::session::Session`). The window drives it with
`Command::Edit(EditCommand)` and redraws what the returned
`Event::Edit(EditEvent)`s name, as for every other command.

| Command | Does |
|---|---|
| `Start(Picked)` | starts an edit from a selection (dropping one in progress) and lists its menu |
| `CheckMenu` | works out each entry's applicability (`Session::check_edit_menu` does the same and can be stopped) |
| `Choose(index)` | chooses a menu entry |
| `Bind(Picked)` | binds the next place param still to point at with a further pick |
| `Set { name, value }` | sets a setting to a value typed in the card |
| `Rename(name)` | names the new item, in place of the generated name |
| `ApplyFix { problem, fix }` | adds one of the refusal's fixes to the candidate |
| `Add` | writes the candidate into the text, only when it fits |
| `Cancel` | drops the edit, writing nothing |

| Event | Says |
|---|---|
| `Started` | an edit began; the menu is listed |
| `MenuChanged` | entries' applicability, or their order, changed |
| `Chosen`, `PlaceBound`, `SettingChanged`, `Renamed`, `FixApplied` | the candidate changed in that way |
| `VerdictChanged` | the candidate's verdict, or what is still to choose, changed |
| `Committed` | the candidate was written; the committed design is the session's result (with `ResultChanged`) |
| `Cancelled` | the edit was dropped |
| `Rejected(why)` | the command couldn't be done (a name already used, a pick that fills nothing the card asks for, Add on a candidate that doesn't fit, a design that doesn't place); nothing changed |

Each change to the candidate writes its patch again (the candidate's
`add`, then each fix applied) and re-runs the scratch check, so the
verdict is always that of the candidate as it now is. A name typed for
the new item before it is added is part of the patch; one that is
already used in the project, or is a word of the language, is refused.

**Add** writes the patch as text edits per file (the smallest edits, as
the patch gives them) through the open buffers when the window has the
file open, so the editor shows the change unsaved and its undo can take
it back; on the command line, to disk. The session then shows the
committed design. Add never overwrites a change made since the edit
began: every file is checked before any is written, and when one no
longer holds the text the edit started from (a change saved in the
editor or elsewhere, or a file the patch would create that now exists)
Add writes nothing and is rejected (`CantWrite`), the change is kept, and
the edit stays open to be cancelled or started again. **Cancel** leaves
no trace: nothing was written, so every file is as it was, byte for
byte.

## The command line

`stave edit` runs the whole loop with no window. Its output is the
editor's test harness, so it is stable: entries in menu order, paths as
given on the command line, diagnostics as `stave check` prints them.

```
stave edit cabinet --patch shelf.stave [--commit]        # try a patch file
stave edit cabinet --region divider.above                # the menu
stave edit cabinet --face 'door_l.stile[hinge].front'    # the menu for a face
stave edit cabinet --between side_l side_r               # the menu for a gap
stave edit cabinet --region divider.above --choose Shelf \
    --bind 'parts side_l side_r' --set headroom=400mm --name hat_shelf --commit
stave edit cabinet --script steps.txt
```

The menu lists each entry as `casework.Shelf` (the bare label for the
project's own declarations) with `applies`, `needs a choice: <names>`
or `refused` and the checker's errors under it. `--choose` takes the
label or the label with its group, and prints the card: what fills each
param, the verdict, the checker's words and the patch.

A script holds one step a line (`#` starts a comment); each is echoed
(`> list`) before what it prints:

```
select region divider.above      # or: face <part>.<face>, between <a> <b>, parts <a> <b> ...
list
choose casework.Shelf
bind parts side_l side_r         # a selection, as for select
set headroom = 400mm
name hat_shelf
fix 1 1                          # a refusal's fix: problem, fix, from 1
show
commit                           # or: cancel
```

The run stops at the first step that fails (a name not in the design, a
command the edit rejects, a commit of a candidate that doesn't fit) and
exits with 1; a script line that is not a step exits with 2 before
anything runs.

## Generated text

**Generated text uses assemblies.** A menu pick of a component writes
`assembly <name> : <Component> { ... }` with its place params bound to
what was pointed at and its setting params as chosen, never a bare
`part`. A pick of a kind writes `joint <name> : <kind>(...)` or
`hardware <name> : <kind>(...)`. Grips edit component params
(`set apron_front.setback = 5mm`).

**Naming rules.**

- **Envelope.** `overall_width`, `overall_height` and `overall_depth` are
  the only params the tool creates by itself, with `region envelope =
  box(across: overall_width, up: overall_height, deep: overall_depth)`.
  They are written when a project is started from a box, as plain params.
- **Sizes.** A drag writes a plain number. "Name this size" turns a number
  into a `param` with a name the user types (`add param shelf_gap = 300mm`
  and a `set` that uses it). The tool never invents a param name.
- **Assemblies.** The component's name in lower case, words joined by `_`
  (`Drawer` gives `drawer`, `FramePanelDoor` gives `frame_panel_door`),
  then `_2`, `_3` for the next ones. A joint or a piece of hardware is
  named the same way from its kind's name (`dado`, `dado_2`).
- **Repeats.** A repeat of 2 or 3 is a labelled array, its labels from the
  direction of the split: up `[bottom, top]` or `[bottom, middle, top]`;
  across `[left, right]` or `[left, middle, right]`; deep `[front, back]`
  (and `[front, middle, back]`). A repeat of 4 or more is a counted array.
- **Bays.** The region a repeated component fills is the component's name
  plus `_bay`: `drawer_bay[bottom, top]`.
- **Mirrored pairs.** The name plus its side: `door_left`, and
  `door_right = mirror(door_left, about: ...)`.
- **Held rules.** Named from the two things measured, plus `_min` or
  `_max`: `slat_to_apron_front_min`. The reason the user types is its
  message.
- **Files.** One file per project until the user splits it. Once split,
  an added item goes to the file of the first item it refers to, as a
  patch's `add` does.
- Every generated name is shown on the card and can be retyped there,
  before or after the commit; retyping is a `rename` patch.

A generated name that is already taken in its scope takes the next
number (`drawer_3`); one that is a keyword or a built-in function's name
takes `_2` from the start.

## Repeat and mirror

**Repeat** turns one assembly into an array. It writes the array with
`split` (equal pieces, or `shares:` once a grip has moved a boundary) for
pieces that fill a region, or with `spaced` for pieces placed along it,
renames the region the component fills to `<name>_bay` and binds each
element with `[same]`:

```
region drawer_bay[bottom, top] = split(bay, up, shares: [1, 1])
assembly drawer[bottom, top] : Drawer {
  opening: drawer_bay[same]
}
```

A grip between two neighbouring pieces edits their two shares, keeping
their sum, so only those two pieces change size.

**Mirror.** When the selection has a mirror image in the design (a
component placed against one of two parts that are mirror images, or in
one of two regions of a split across), the tool offers it, and writes the
pair with `mirror(...)` about the mid-plane between them, renaming the
source with its side (`door_left`).

## Hold this

The user measures a distance between two things and pins it with a
minimum or a maximum. It becomes a `require`:

```
require slat_to_apron_front_min : between(slat[0], apron_front) >= 12mm "room for a finger"
```

The value pinned is the distance as it is now, and the user may type
another; the message is the reason the user types. Like any edit it is
tried first: a rule that already fails is refused.

## Remove

Removing an item lists what depends on it (every item whose text names
it, and what depends on those) and offers to remove those too, as one
patch. Otherwise it is refused: removing an item that something still
names would leave an error, which strict commit never writes.

## Grips

A grip is a handle on a size in the 3D view.

- **Grips in the first build move plain numbers only**: the envelope's
  params, component params (`set drawer[top].top_gap = 20mm`) and shares.
  A grip shows its value, the range it may move in (the param's `within`,
  or for a share, up to the sum of the two it shares with) and its stops:
  the values of a `within` list, or a step of the project's `resolution:`.
- **A size that is an expression** shows what it follows ("follows
  `cabinet_width`") and has no grip of its own. Single-param inversion
  ([Smart edits](grammar.md#smart-edits), rule 2) comes with the grips:
  dragging such a size offers its plain params.
- z3 over solved params is a later stage. No crate depends on z3 for
  editing today.

## What is not in the first build

Do not add these; each is a later decision:

- component warnings ("Case wants a back");
- a `fill` size;
- turned or tapered legs;
- automatic linking of repeated numbers;
- applicability guards beyond `require`;
- z3 (solved params are edited as plain ones until it comes).

## Libraries this is meant for

Written later, as ordinary libraries: `std:casework` (`Case`, `Back`,
`Divider`, `Shelf`, `HangingRail`, `Feet`, `Drawer`, `FramePanelDoor`) and
`std:frames` (`Top`, `Legs`, `Apron`, `Stretcher`, `Cleat`, `Slats`). Each
exposes its place params, a default for each setting where sensible, each
size a user would drag as a param, and the regions a later pick fills
(`case.inside`, `divider.above`, `divider.below`). Nothing in the editor
names them.
