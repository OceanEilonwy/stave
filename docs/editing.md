# Editing in the 3D view

How the user builds and changes a design with the mouse, decided with the
owner on 2026-11-18. This is the specification the editor is built
against: `stave-app` holds the behaviour (headless, tested without a
window), `stave-gtk` draws it and forwards what people do, as everywhere
else (see [`gui.md`](gui.md)). The language it writes is
[`grammar.md`](grammar.md); where this file names a language rule, the
grammar is the authority.

The contract types are in `crates/stave-app/src/edit.rs` (`stave_app::edit`):
the typed selection, a candidate, a verdict, a fix and a grip, with what
the card shows (see [The card](#the-card)) and a hand-built fixture for
the clothing cabinet. The behaviour is built on them: signature matching
and applicability in `stave_app::candidates`, the scratch check in
`stave_app::scratch`, the edit in progress, which joins them, in
`stave_app::editing`, and the changes of the design that don't come from
the menu in `stave_app::change` (see [The edit in
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
made it check, fits. Each of a refusal's problems carries the fixes its
diagnostic holds, as patches (`Problem::fixes`), and its text as `stave
check` prints it, source excerpt included (`Problem::rendered`). Warnings never block: a candidate with warnings and
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

The edit in progress is session state, held by
`stave_app::session::Session`: either an **edit** from a selection and
its menu (`stave_app::editing::Edit`, `Session::edit`) or a **change**
of the design that doesn't come from the menu (`stave_app::change::Change`,
`Session::change`): a repeat, the shares grip, a mirror, a held distance,
a removal, a rename. Never both: starting one drops the other. The window
drives them with `Command::Edit(EditCommand)` and redraws what the
returned `Event::Edit(EditEvent)`s name, as for every other command.

| Command | Does |
|---|---|
| `Start(Picked)` | starts an edit from a selection (dropping what is in progress) and lists its menu |
| `CheckMenu` | works out each entry's applicability (`Session::check_edit_menu` does the same and can be stopped) |
| `Choose(index)` | chooses a menu entry |
| `Bind(Picked)` | binds the next place param still to point at with a further pick |
| `Set { name, value }` | sets a setting to a value typed in the card (an `Expr`; `editing::parse_value` reads one) |
| `Rename(name)` | names the new item, in place of the generated name |
| `Repeat(Option<Repeat>)` | makes the chosen entry `count` pieces filling its region, split along `axis` (`candidates::Repeat { count: RepeatCount, axis: Axis }`), or one again (`None`) |
| `RepeatItem { item, count, axis }` | starts a change: repeats an assembly of the project that fills a region `count` times, or changes the count of one already repeated (`axis: Option<Axis>`, needed for a first repeat only) |
| `MoveShares { region, lower, boundary }` | starts a change: moves the boundary between pieces `lower` and `lower + 1` (from 0, in label order) of a split, so piece `lower` is `boundary` (a `Length`) long along the split |
| `Mirror(item)` | starts a change: adds the mirror image of an assembly, renaming it with its side |
| `Hold(HoldRequest)` | starts a change: holds a measured distance as a `require` (see [Hold this](#hold-this)) |
| `Remove(item)` | starts a change: removes an item; while something depends on it the change *needs a choice*, naming what does |
| `IncludeDependants` | a removal waiting on its dependants removes them too |
| `RenameItem { item, to }` | starts a change: renames an item of the project everywhere it is used |
| `ApplyFix { problem, fix }` | adds one of the refusal's fixes to the candidate or the change (both counted from 0) |
| `Add` | writes the candidate or the change into the text, only when it fits |
| `Cancel` | drops the edit or the change, writing nothing |

`Session::mirror_offer(item)` says, without trying anything, whether an
assembly has a mirror image and what adding it would name (a
`MirrorOffer { source, renamed, image, about }`): the card offers
**Mirror** when it is `Some`.

| Event | Says |
|---|---|
| `Started` | an edit began; the menu is listed |
| `MenuChanged` | entries' applicability, or their order, changed |
| `Chosen`, `PlaceBound`, `SettingChanged`, `Renamed`, `RepeatChanged`, `FixApplied` | the candidate changed in that way |
| `ChangeStarted` | a change began and has been tried; `VerdictChanged` follows |
| `DependantsIncluded` | a removal now takes what depends on the item too |
| `VerdictChanged` | the candidate's or the change's verdict, or what is still to choose, changed |
| `Committed` | the candidate or change was written; the committed design is the session's result (with `ResultChanged`); `Session::last_commit` lists the files written |
| `Cancelled` | the edit or change was dropped |
| `Rejected(why)` | the command couldn't be done (a name already used, a pick that fills nothing the card asks for, Add on something that doesn't fit, a design that doesn't place, a change that can't be written for this design, `IncludeDependants` with no removal waiting: `NothingToInclude`); nothing changed |

Each change to the candidate writes its patch again (the candidate's
`add`, then each fix applied) and re-runs the scratch check, so the
verdict is always that of the candidate as it now is. A name typed for
the new item before it is added is part of the patch; one that is
already used in the project, or is a word of the language, is refused.

**Add** writes the patch as text edits per file (the smallest edits, as
the patch gives them) through the open buffers when the window has the
file open, so the editor shows the change unsaved and its undo can take
it back; on the command line, to disk. The session then shows the
committed design. **One writer**: every commit (an edit's, a change's,
the command line's `--patch --commit`, a new project's file) goes
through `editing::write_all` and a `WriteEdits`, which checks every file
before it writes any. When one no longer holds the text the edit started
from (a change saved in the editor or elsewhere, or a file the patch
would create that now exists) nothing is written and Add is rejected
(`CantWrite`), the change is kept, and the edit stays open to be
cancelled or started again. **Cancel** leaves no trace: nothing was
written, so every file is as it was, byte for byte.

### Changes of the design

A change is written by its pure function and judged by the same scratch
check as a menu pick: `repeat::repeat_assembly` (repeat),
`repeat::move_shares` (the shares grip), `repeat::mirror_assembly`
(mirror), `hold::hold_patch` (hold, given the opened design, which says
which faces a part and a region are measured between), `remove::removal`
and `remove::patch_for` (remove), and the navigation layer's rename
(`stave_ide::Analysis::rename`). The patcher doesn't apply `rename` (it
needs every use), so a rename, and the rename a mirror makes, comes as
whole-file texts: the change is tried on a scratch copy with those texts
in place, then its patch on top. A rename alone fits when the renamed
design places and has no more errors than before. What a commit writes
is, per file, the one edit from the text the change began from to the
final text.

`Change::summary` says what it does ("Make 3 of drawer"),
`Change::detail` what it found (`ChangeDetail::Mirror { renamed, image,
about }`, `Hold { name, condition }`, `Remove { removal, dependants }`,
`Rename { from, to }`, or `Repeat`), `Change::progress` where it stands
(the same `Progress` as an edit's chosen candidate), `Change::patch` the
patch tried, `Change::renamed_files` the files a rename rewrites, and
`Change::ghost` the design it would make.

### The card

The contract (`stave_app::edit`) carries everything the card shows, each
field filled where candidates and verdicts are made
(`EditScope::describe`, run when the menu is listed and after every
change to the chosen candidate; `default_value` is filled after each try,
by `candidates::describe::fill_default_values`):

| Field | Holds |
|---|---|
| `Candidate::generated_name` | the name the new item takes unless one is typed (`drawer_2`) |
| `PlaceParam::prompt` | the sentence the card asks for an unbound place param with: the first sentence of the param's own `///` doc comment, or "Point at the hinge side." from its name |
| `PlaceParam::text` | what fills it, as written (`door_bay[left]`, `side_l.right`, `[side_l, side_r]`) |
| `SettingParam::text` | the value it has now as `.stave` text: chosen, copied, or the default as written |
| `SettingParam::default_text` | the param's default as the declaration writes it (`15mm`, `opening.up - bottom_gap - top_gap`) |
| `SettingParam::default_value` | the value the default stands for, while the setting is left at its default: worked out by the solver in the scratch copy the chosen candidate was tried in (`opening.up - bottom_gap - top_gap` gives `220mm` for a 250mm drawer bay), or, for a default that names a value (`ply_18`, `butt_hinge(length: 40mm)`), that text. `None` on a menu entry not yet chosen, before the candidate is tried (a place param still to point at), when the copy could not be solved, and for a chosen or copied value |
| `SettingParam::copied_from` | the assembly a copied value was read from (`drawer[top]`) |
| `SettingParam::options` | what it can be set to, by its type, each a `SettingOption { text, hint }`: the stocks in scope for a `Stock`; the mounts the project's assemblies write, then `overlay()` and `inset(reveal: 2mm)`, for a `Mount`; the kinds of that class in scope for a kind type (`side_mount_slide`, `hw.brass_knob_26`); the values of a `within [..]` list for a quantity; `true` and `false` for a `Bool` |
| `SettingParam::range` | the two ends of a `within low..high`, as written (`ValueRange { low, high }`) |
| `Problem::rendered` | the error as `stave check` prints it: message, place, source excerpt, notes, help and fixes |
| `Problem::fixes` | the fixes the diagnostic holds, as patches (`Fix::patch`, labelled by `Fix::label`) |

Names in scope are written as the project would write them: bare for the
project's own declarations and the prelude's, through the import's name
for an imported module's (`casework.oak_19`). `edit::value_text` prints
any value expression as exact `.stave` text, and `edit::Names` gives the
names of the regions, parts and assemblies the contract's ids refer to
(`region`, `part`, `assembly`, `face`, `slot`, `grip_target`).

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
param, the verdict, the checker's words and the patch. `--patch
--commit` writes each patch that fits through the editor's one writer.

### Scripts

A script is a whole build with no window: every step is a command to
the same session a window drives. It holds one step a line; `#` starts a
comment (except inside a quoted string). Each step is echoed (`> list`)
before what it prints.

| Step | Does |
|---|---|
| `new "<title>" <width> <height> <depth>` | begins a new project from its overall sizes |
| `import <source>` | the new project imports a module (`import std:casework`) |
| `stock <name> <species> solid\|plywood\|mdf <thickness> [width <w>] [max_width <w>] max_length <l>` | a stock item of the new project |
| `stock <name> <species> dowel <diameter>` | a dowel stock of the new project |
| `tools <tool> ...` | the new project's tools (left out: every tool) |
| `create` | writes the new project to `<folder>/<title_in_lower_case>.stave` (never over a file that is there) and checks it; fails on an error |
| `select <selection>` | starts an edit: `region <name>`, `face <part>.<face>`, `between <part> <part>`, `parts <part> ...` |
| `list` | the menu, each entry's applicability worked out |
| `choose <entry>` | chooses an entry by its label or `group.label` |
| `bind <selection>` | points at what the card asks for |
| `set <name> = <value>` | sets a setting |
| `name <name>` | names the new item |
| `repeat <count> <axis>` | makes the chosen entry `count` pieces split along `across`, `up` or `deep`; `repeat 1` makes it one again |
| `repeat <assembly> <count> [<axis>]` | change: repeats an assembly, or changes its count |
| `shares <region> <piece> <length>` | change: moves the boundary above piece `<piece>` (from 1) so that piece is `<length>` long |
| `offer mirror <assembly>` | says whether it has a mirror image and what adding it would name |
| `mirror <assembly>` | change: adds its mirror image |
| `hold <a> <b> min\|max <length> "<reason>"` | change: holds the distance between two things, each a part or region by its whole name (`drawer[top].front`), else a face of one (`case.top.bottom`) |
| `remove <item>` | change: removes an item, or says what uses it |
| `include dependants` | the removal takes what uses the item too |
| `rename <item> <new name>` | change: renames an item everywhere |
| `fix <problem> <fix>` | applies a refusal's fix, both counted from 1 as `show` numbers them |
| `show` | the card: of the chosen entry, or of the change |
| `commit` or `add` | writes the candidate or the change |
| `cancel` | drops it, writing nothing |

```
# A small chest from nothing
new "Small chest" 700mm 1000mm 480mm
import std:casework
stock oak_19 white_oak solid 19mm max_width 200mm max_length 2700mm
stock maple_12 hard_maple solid 12mm max_width 150mm max_length 2400mm
stock ply_6 birch plywood 6mm max_length 2440mm
create

select region envelope
choose casework.Case
set panel_stock = oak_19
add

select region case.inside
choose casework.Divider
bind parts case.side[left] case.side[right]
set panel_stock = oak_19
add

select region divider.below
choose casework.Drawer
set mount = overlay(left: 18mm, right: 18mm, top: -1.5mm, bottom: 8mm, meeting: -0.75mm)
set front_stock = oak_19
set box_stock = maple_12
set bottom_stock = ply_6
set slide = side_mount_slide(length: 400mm)
repeat 2 up
add

repeat drawer 3          # changes of the design
add
shares drawer_bay 1 180mm
add
```

What each step prints:

- `new`: `new project "<title>": <w> wide, <h> high, <d> deep`; `import`,
  `stock` and `tools` print nothing; `create`: `created <path>` and `the
  project checks with <n> errors`, with the errors as `stave check`
  prints them.
- `select`: `<heading>: <selection>` and `<n> menu entries`; `list`: one
  entry a line, `  1. casework.Shelf  applies`.
- `choose`, `bind`, `set`, `name`, `repeat <count>`, `fix`, `include
  dependants`: where the candidate (or change) stands, in one line:
  `fits`, `fits, with 1 warning`, `refused: 2 errors`, `needs a choice:
  sides`.
- A change step: `<summary>: <where it stands>` (`Make 3 of drawer:
  fits`); a removal waiting on what uses the item: `Remove back: used by
  drawer_bay, drawer; 'include dependants' removes them too`.
- `show`: the card. For a change: its summary, what it found (indented),
  `verdict: ...`, each error as `stave check` prints it and `fix <p> <f>:
  <label>` for each fix, then `patch:` and the patch.
- `commit`/`add`: `wrote <path>` (or `created`) per file, then
  `committed: the design checks with <n> errors`; or `not committed:
  <why>` and the run stops.
- A rejected command prints `rejected: <why>`; a step that can't run
  prints `error: <why>`.

The run stops at the first step that fails (a name not in the design, a
command the edit rejects, a commit of something that doesn't fit, a new
project that doesn't check) and exits with 1; a script line that is not
a step exits with 2 before anything runs.

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
- Every generated name is shown on the card (`Candidate::generated_name`)
  and can be retyped there: before the commit with `Rename`, after it with
  `RenameItem`, which renames the item everywhere it is used.

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
their sum, so only those two pieces change size. The boundary is snapped
to whole millimetres, and the shares are written as whole numbers in the
same proportions (`100 / 181` and `262 / 181` become `50` and `131`), so
a later grip reads them back as plain numbers. When the pieces' sizes
are not whole millimetres themselves (a split of an odd size), those
whole numbers can be large; they are exact.

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

The request (`hold::HoldRequest`) is the two things measured (each a
`Measured::Item` by its path, `slat[0]`, `drawer[top].front`,
`divider.below`, or a `Measured::Face` of one, `apron_front.back`), the
bound (`Bound::Min` writes `>=` and `_min`, `Bound::Max` writes `<=` and
`_max`), the value and the reason. `hold_patch(scope, design, request)`
takes the opened design too, because the distance is written in a form
`between` accepts from where the design has the two things: opposite
faces of one part or region are its own size (`inside.deep`,
`extent(base, up)`); two parts are `between(a, b)`; a part and a region,
or two regions, apart along one axis are the two faces that look at each
other; a part inside a region is its face nearest the same side. It
refuses (`HoldError`) an empty reason (`NoReason`), the same thing twice
(`SameThing`), a name that is not an item of the project
(`UnknownItem`), a thing that is not a part or region of the design, an
assembly say (`NotMeasurable`), two faces that are not parallel
(`NotParallel`), and two things apart along more than one axis
(`NotOneAxis`: pick two faces instead).

## Remove

Removing an item lists what depends on it (every item whose text names
it, and what depends on those) and offers to remove those too, as one
patch. Otherwise it is refused: removing an item that something still
names would leave an error, which strict commit never writes. In the
session, `Remove(item)` starts the change; while anything depends on the
item it *needs a choice* naming each dependant, nearest first, and Add is
refused; `IncludeDependants` then removes them too (furthest first, the
item last), and the whole is tried as one patch.

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
