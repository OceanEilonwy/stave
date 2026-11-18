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
hand-built fixture for the clothing cabinet. They carry no behaviour yet.

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

A click in a space picks the smallest region that holds the point.
"Between two facing parts" is a point in the gap between two parallel
faces of two parts that face each other, with nothing between them.

## Signature matching

The menu for a selection is worked out from the declarations in scope,
never from a list in the app.

1. **Params split by type.** A component's params, and a kind's roles,
   are of two sorts. **Place params** (`Region`, `Anchor`, `Part`,
   `Parts`) are filled by pointing. **Setting params** (`Stock`, `Mount`,
   the kind types, lengths and other quantities) are filled in the card.
2. **Candidates.** A `component` or `kind` in scope is a candidate when
   the selection can fill its place params: every typed value of the
   selection fills a place param of its own type that no other value
   fills (the first such param in declaration order), and at least one
   place param is filled. For a kind, its roles play that part: a role of
   type `Part` takes one part of the selection, a role of type `Parts`
   takes one or more, and the part pointed at first goes to the first
   role.
3. **Applicability.** Each candidate is instantiated in a scratch copy
   with its defaults (and, for a setting with no default, the value it
   would get by step 5) and checked. Its own `require` lines are its
   applicability rules. A candidate that fails is still listed, greyed,
   with the checker's message, so the user learns why it doesn't fit.
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
