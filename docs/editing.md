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
`stave_app::editing`, the changes of the design that don't come from
the menu in `stave_app::change` (see [The edit in
progress](#the-edit-in-progress)), and grips and the solver in
`stave_app::gripping` (see [Grips in the session](#grips-in-the-session)).

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

Escape, or moving to another place, drops the candidate. **There is no
undo of a commit yet.** A commit into a file open in the Code panel goes
into its buffer as one step of that editor's own undo (Ctrl+Z with the
text focused). A commit with no file open in the editor, the default for
a menu Add, is written straight to disk, and the window can't take it
back: the only ways back are Remove, or editing the text. Undo of a
commit, as one step and an edit of the text like any other, is still to
be built (see [STATUS](STATUS.md#graphical-editing)).

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
   (step 6).

   Two rules leave out entries that could never apply, so the menu
   doesn't list them at all:

   - **A joint joins parts that meet.** Every joint kind's parts must
     touch (the checker's `no-contact` rule holds for the whole class),
     so a joint is not listed when the parts the selection gives its
     roles can't meet: the part of its first role touches or overlaps
     none of the parts of the others, as the design has them now. Two
     parts facing each other across a gap ("Between these parts") never
     meet, so no joint is offered between them. A joint with a role still
     to point at is listed as usual. Hardware is not left out this way:
     whether its members reach a part depends on the kind, so the
     checker says.
   - **A component is not put straight inside itself.** A component is
     not offered to fill a space that one of its own assemblies declares
     (a `Case` in `case.inside`): that would nest it in itself with
     nothing between. A space made from such
     a region in the project (`split(case.inside, up)`) is the project's,
     and offers it as usual. A component that is meant to nest says so
     in its `///` doc comment with the words "may nest inside itself"
     (any case): `/// A tray. It may nest inside itself.` The library's
     `Divider` says so, since the space under a divider may take another
     divider.
3. **Applicability.** Each candidate is instantiated in a scratch copy
   with its defaults (and, for a setting with no default, the value it
   would get by step 6) and checked. Its own `require` lines are its
   applicability rules. A candidate that fails is still listed, greyed,
   with the checker's message, so the user learns why it doesn't fit (an
   error said again with the same code and message, once for each
   element of an array, is shown once). A candidate that still has a
   place param to point at, or a setting with no default and nothing to
   copy, is not tried: it **needs a choice**,
   which is not a refusal, and the entry is listed as usual. Trying never
   changes the design the menu was made for, and the menu can be checked
   a little at a time: a check that is stopped part way keeps the answers
   it reached, and the rest wait.
4. **Hardware on a face is placed.** A kind of hardware (a slide, hinge
   or pull) chosen on a face is mounted on that face: its call writes
   `face: <the face>` when the kind's own `mount { }` puts it on another,
   and a `place { }` that fixes every axis the kind leaves `open`, each
   centred on the piece (`center(stile, across)`, `center(stile, up)`),
   so a knob on a door stile fits as soon as it is chosen. The axes are
   found as the solver finds them: `along` is the piece's length (its
   width when the mount face is an end), `across` the mount face's other
   axis. Hardware chosen on parts rather than a face is centred on its
   piece the same way, on its kind's own mount face. The user can move
   it after (a `set knob_2.place.up = ...`).
5. **The menu entry** is drawn from the declaration:

   | Part of the entry | From |
   |---|---|
   | label | its name (`Drawer`, `dado`) |
   | hint | the first sentence of its `///` doc comment |
   | group | its module: the project's own first, then imported modules by name, then `std` |
   | heading | the type of the selection (the table above) |

   The entries that apply, need a choice or aren't checked yet come
   first, then every refused (greyed) entry, so the window can fold the
   refused ones under one heading (`Edit::refused_from` is where they
   begin, `candidates::refused_from` for any menu). Each part is
   ordered by group, and within a group by name.
6. **Filling the rest.** A place param still unbound is asked for by
   pointing: the card names it ("Point at the side the hinges go on") and
   the next click fills it. The card lists them in declaration order,
   and a click fills the first one still open that it can: a face fills
   an `Anchor` with the face or a `Part` with its part, whichever comes
   first (so an apron's `under: Part` takes the top before its `outside:
   Anchor` takes a leg's face), else a `Parts` with its part; a click
   between two parts fills a `Parts`, else one `Part` each
   (`editing::bind_further`). Setting params with no default are asked for in
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
| `PutBack` | puts the setting a refusal blames back to the value it had before (see [Blame and the way back](#blame-and-the-way-back)) |
| `Rename(name)` | names the new item, in place of the generated name |
| `Repeat(Option<Repeat>)` | makes the chosen entry `count` pieces filling its region, split along `axis` (`candidates::Repeat { count: RepeatCount, axis: Axis }`), or one again (`None`) |
| `RepeatItem { item, count, axis }` | starts a change: repeats an assembly of the project that fills a region `count` times, or changes the count of one already repeated (`axis: Option<Axis>`, needed for a first repeat only) |
| `MoveShares { region, lower, boundary }` | starts a change: moves the boundary between pieces `lower` and `lower + 1` (from 0, in label order) of a split, so piece `lower` is `boundary` (a `Length`) long along the split |
| `Mirror(item)` | starts a change: adds the mirror image of an assembly, renaming it with its side |
| `MirrorSplit { item, axis }` | starts a change: makes an assembly that fills a whole space a mirrored pair, splitting the space in two along `axis` (see [Repeat and mirror](#repeat-and-mirror)) |
| `SetParam { item, param, value }` | starts a change: sets a param of an assembly of the project, as a grip does when it is let go (`set apron_front.setback = 3mm`) |
| `RepeatPart { part, count, axis, from, to }` | starts a change: repeats a part spaced evenly along a span from one anchor to another (`from: "rail_l.right"`), or changes how many there are ([`repeat::repeat_part`](#repeat-and-mirror)) |
| `Hold(HoldRequest)` | starts a change: holds a measured distance as a `require` (see [Hold this](#hold-this)) |
| `Remove(item)` | starts a change: removes an item; while something depends on it the change *needs a choice*, naming what does |
| `IncludeDependants` | a removal waiting on its dependants removes them too |
| `RenameItem { item, to }` | starts a change: renames an item of the project everywhere it is used |
| `ListGrips(GripSelection)` | lists the grips of an assembly, the envelope or a split's boundaries, without their stops (see [Grips in the session](#grips-in-the-session)) |
| `BeginDrag(index)` | starts a change: a drag on the listed grip at `index` |
| `DragTo(Amount)` | moves the grip being dragged to a value: its patch is written again and tried |
| `PickParam(ParamId)` | picks which plain param a dragged size changes, from the drag's choices |
| `TypeGripValue { grip, value }` | a drag on a listed grip begun and moved to a typed value in one |
| `MakeSolved { param, low, high }` | starts a change: makes a param of the project a `solved param` within `low..high` |
| `MakePlain(param)` | starts a change: makes a solved param of the project plain again, keeping its range |
| `ApplyFix { problem, fix }` | adds one of the refusal's fixes to the candidate or the change (both counted from 0) |
| `Add` | writes the candidate or the change into the text, only when it fits; ends a drag by writing it |
| `Cancel` | drops the edit or the change, writing nothing; ends a drag by dropping it |

`Session::mirror_offer(item)` says, without trying anything, whether an
assembly has a mirror image and what adding it would name (a
`MirrorOffer { source, renamed, image, about }`); `None` too when the
item already has its image (`door_right = mirror(door_left, ...)`) or is
one. `Session::item_offer(item)` (`stave_app::item_offer`) is everything
a selected item's card offers: how many pieces it is, whether the repeat
controls apply (it is pieces of a repeat or fills a space), and what
**And the other side** does, a `MirrorGesture`: `Image(MirrorOffer)`
(sends `Mirror`) or `Split(axis)` (sends `MirrorSplit`), or nothing.
`MirrorGesture::command(item)` is the command to send. The window shows
these answers and decides none of them. `stave_app::item_offer` also
says which item a selection changes (`selected_item`: its outermost
assembly or the part, without its index) and how many pieces an item is
(`piece_count`); `stave_app::grip_values` says where a drag snaps, how
far it may go, what an arrow key steps by and whether a typed value
would be written as typed.

| Event | Says |
|---|---|
| `Started` | an edit began; the menu is listed |
| `MenuChanged` | entries' applicability, or their order, changed |
| `OptionsChanged` | an option of the chosen candidate's settings was found to fit or not, or the options' order changed (see [Options are checked](#options-are-checked)) |
| `Chosen`, `PlaceBound`, `SettingChanged`, `Renamed`, `RepeatChanged`, `FixApplied` | the candidate changed in that way |
| `ChangeStarted` | a change began and has been tried; `VerdictChanged` follows |
| `DependantsIncluded` | a removal now takes what depends on the item too |
| `VerdictChanged` | the candidate's or the change's verdict, or what is still to choose, changed |
| `GripsChanged` | `Session::grips` changed: listed, or dropped after a commit (the design they were listed from is gone) |
| `GripStopsFound(index)` | the stops of the listed grip at `index` were found |
| `DragMoved`, `ParamPicked` | the grip being dragged moved, or a param was picked for it; `VerdictChanged` follows when the verdict did |
| `Committed` | the candidate or change was written; the committed design is the session's result (with `ResultChanged`); `Session::last_commit` lists the files written |
| `Cancelled` | the edit or change was dropped |
| `Rejected(why)` | the command couldn't be done (a name already used, a pick that fills nothing the card asks for, Add on something that doesn't fit, a design that doesn't place, a change that can't be written for this design, `IncludeDependants` with no removal waiting: `NothingToInclude`; `PutBack` with nothing blamed: `NothingToPutBack`; a grip command with no grips listed: `NoGrips`; a grip index the listing doesn't have: `NoSuchGrip`; a drag command with no drag: `NotDragging`; a pick that is not one of the choices: `NotAChoice`); nothing changed |

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
`Rename { from, to }`, `Grip(GripDetail)` for a drag, or `Repeat`), `Change::progress` where it stands
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
| `SettingOption::fit` | on the chosen candidate, whether it fits with the setting set to that option (`OptionFit`: `NotChecked`, `NeedsChoice(names)`, `Fits`, or `Refused(OptionRefusal { first, more })`, the checker's first error and how many more); the refused options come after the rest (see [Options are checked](#options-are-checked)) |
| `SettingParam::range` | the two ends of a `within low..high`, as written (`ValueRange { low, high }`) |
| `Chosen::blame` | the setting change that made the candidate stop fitting (`Blame { setting, from, from_text, to_text }`, with `Blame::text` and `Blame::put_back_label`; see [Blame and the way back](#blame-and-the-way-back)) |
| `Problem::rendered` | the error as `stave check` prints it: message, place, source excerpt, notes, help and fixes |
| `Problem::fixes` | the fixes the diagnostic holds, as patches (`Fix::patch`, labelled by `Fix::label`) |

Names in scope are written as the project would write them: bare for the
project's own declarations and the prelude's, through the import's name
for an imported module's (`casework.oak_19`). `edit::value_text` prints
any value expression as exact `.stave` text, and `edit::Names` gives the
names of the regions, parts and assemblies the contract's ids refer to
(`region`, `part`, `assembly`, `face`, `slot`, `grip_target`).

### Options are checked

A setting's options are checked the way menu entries are
(`stave_app::option_fits`). For the chosen candidate, every option of
every setting that has a list (`SettingParam::options`: kinds of any
class, stocks, mounts, a `within` list, `true`/`false`) carries an
`OptionFit`:

| Fit | Means | The card shows |
|---|---|---|
| `NotChecked` | its check hasn't run, or was thrown away | a spinner and "Checking whether it fits…"; it can be chosen |
| `NeedsChoice(names)` | the candidate would still wait on something else (a place to point at, another setting with no value), so it can't be tried | "Checked once you choose …"; it can be chosen |
| `Fits` | the candidate checks with no error with this option | a tick and "Fits" |
| `Refused(OptionRefusal)` | it would cause errors: `first`, the checker's first error, and `more`, how many after it | greyed, a "can't" sign and "Can't be used here: error[overlap]: …"; it can't be chosen from the list |

An option is tried by building the candidate in a scratch copy with that
setting set to that option and every other setting, the place params,
the repeat, the name typed and the fixes added as they stand now: the
same patch the candidate would write if that option were chosen, judged
as the candidate is (an error the design already had doesn't count). The
option the setting holds now takes the candidate's own verdict without a
second check. Options that fit (and those not checked yet, or waiting)
come first, in the order their type lists them; the refused ones after,
in the same order. None is hidden: what can't be chosen is shown greyed
with the reason.

**Without waiting.** The checks are tens of scratch checks for a setting
like a case's joinery (18 kinds), so no command runs them.
`Session::options_job()` gives an `OptionsJob` (it can be sent to another
thread: it prepares the project itself); `OptionsJob::run(cancel, found)`
tries each option in turn, setting by setting in card order, and hands
each answer (`FoundFit { ticket, setting, option, fit }`) to `found` as
soon as it is known; `OptionsJob::only(setting)` and `prefer(setting)`
narrow it or put one setting first (the one whose list is open). The
window hands each answer to `Session::option_fit_found`, which records it
and says `OptionsChanged`. Setting the cancel flag stops the job between
checks (and inside a slow one). `Session::check_setting_options(only,
cancel)` runs the job in place, for a script.

**Thrown away when the candidate changes.** Every change to the chosen
candidate (a setting, a place pointed at, a rename, a fix, a repeat)
takes a new ticket (`Chosen::options_ticket`) and sets every option back
to `NotChecked`, because whether an option fits can depend on the other
settings; an answer for an older ticket is dropped, and the job is run
again. The options of the setting just set keep their answers: they were
tried with the other settings, which did not change.

On the Case in a 940 x 1780 x 580mm envelope with `oak_19`, 6 of the
joinery's 18 options fit (cope_and_stick, dado, dowel, glued_face,
groove, screw) and 12 are refused; the four edge joints say why in
woodworking words, through `edge_to_edge`. Checking all 18 takes
about 0.2 seconds in a debug build on the machine it was written on.

### Blame and the way back

When a `Set` turns a candidate that fitted, or that was waiting for that
very setting, into one that is refused, the refusal is blamed on it:
`Chosen::blame` is a `Blame` with the setting, where its value came from
before (`from`: `Default`, `Copied`, `Chosen` or `Unset`), that value as
text (`from_text`, `None` for `Unset`) and the value it became
(`to_text`). The card puts it first, in plain words (`Blame::text`: "It
stopped fitting when joinery became chigiri.", or for a setting that had
no value yet "It doesn't fit now that panel_stock is oak_19."), keeps the
checker's own messages below, and offers one action
(`Blame::put_back_label`: "Put joinery back to dowel(count: 4), the
default", "Clear panel_stock again") that sends `PutBack`, which restores
exactly the old `SettingValue` (a default stays a default, so nothing is
written for it) and tries the candidate again.

- A candidate already refused before the change gets no blame: the
  setting changed is not why it doesn't fit.
- Setting the blamed setting again while it is still refused keeps the
  blame's `from` (it still stopped fitting when that setting left its
  first value) and updates `to_text`.
- The blame goes when the candidate fits again, when another setting is
  set, and on `PutBack`; a rename, a fix or a repeat that leaves the
  candidate refused keeps it.

A value can always be typed in the setting's entry, a refused one
included; a typed value that is refused is blamed as above.

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
or `refused` and the checker's errors under it, the refused entries
last. `--choose` takes the
label or the label with its group, and prints the card: what fills each
param (a setting left at its default shows the value it works out to:
`depth  9mm (default)`, `box_height  220mm (default: opening.up -
bottom_gap - top_gap)`), the verdict, the checker's words and the
patch. `--patch
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
| `options <setting>` | checks the chosen entry's setting's options and lists each with whether it fits |
| `put back` | puts back the setting a refusal blames to its value before |
| `name <name>` | names the new item |
| `repeat <count> <axis>` | makes the chosen entry `count` pieces split along `across`, `up` or `deep`; `repeat 1` makes it one again |
| `repeat <assembly> <count> [<axis>]` | change: repeats an assembly, or changes its count |
| `spaced <part> <count> <axis> from <anchor> to <anchor>` | change: repeats a part spaced evenly along a span (`spaced slat 5 across from apron_left.right to apron_right.left`), or changes how many there are |
| `shares <region> <piece> <length>` | change: moves the boundary above piece `<piece>` (from 1) so that piece is `<length>` long |
| `offer mirror <assembly>` | says whether it has a mirror image and what adding it would name |
| `mirror <assembly>` | change: adds its mirror image |
| `mirror <assembly> <axis>` | change: makes an assembly that fills a whole space a mirrored pair, the space split in two along `across`, `up` or `deep` |
| `set <assembly>.<param> = <value>` | change: sets a param of an assembly of the project, as a grip does (a `set` whose name has a dot) |
| `measure <a> <b>` | says the distance between two things as the design has it now |
| `hold <a> <b> min\|max [<length>] "<reason>"` | change: holds the distance between two things, each a part or region by its whole name (`drawer[top].front`), else a face of one (`case.top.bottom`); without a length, the distance as it is now |
| `remove <item>` | change: removes an item, or says what uses it |
| `include dependants` | the removal takes what uses the item too |
| `rename <item> <new name>` | change: renames an item everywhere |
| `grips envelope`, `grips assembly <name>`, `grips split <piece>` | lists the grips of the envelope, of an assembly, or of the boundaries of a split (`grips split drawer_bay[bottom]`), numbered from 1 |
| `stops [<grip>]` | searches the stops of one listed grip, or of all that move |
| `drag <grip> <value>` | change: drags a listed grip to a value (`drag 1 420mm`); a `drag` of the same grip goes on from where it is |
| `grip <grip> = <value>` | change: types a value for a listed grip |
| `pick <param>` | picks which param a dragged size changes, by its name as listed (`case.overhang`) or its own (`overhang`) |
| `solved <param> within <low>..<high>` | change: makes a param of the project a solved param |
| `plain <param>` | change: makes a solved param plain again |
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

grips envelope           # a grip, dragged or typed
stops 1
drag 1 720mm
add
measure divider.panel case.top
hold divider.panel case.top min "room for the drawers"   # the distance as it is now
add
```

What each step prints:

- `new`: `new project "<title>": <w> wide, <h> high, <d> deep`; `import`,
  `stock` and `tools` print nothing; `create`: `created <path>` and `the
  project checks with <n> errors`, with the errors as `stave check`
  prints them.
- `select`: `<heading>: <selection>` and `<n> menu entries`; `list`: one
  entry a line, `  1. casework.Shelf  applies`.
- `choose`, `bind`, `set`, `put back`, `name`, `repeat <count>`, `fix`,
  `include dependants`: where the candidate (or change) stands, in one
  line: `fits`, `fits, with 1 warning`, `refused: 2 errors`, `needs a
  choice: sides`. A `set` that turns the candidate refused adds, indented,
  the blame and what `put back` does:

  ```
  > set joinery = chigiri
  refused: 2 errors
    It stopped fitting when joinery became chigiri.
    'put back': Put joinery back to dowel(count: 4), the default
  ```
- `options <setting>`: `<setting>: <n> options, <k> fit`, then one option
  a line in the card's order (those that fit first, the refused after),
  numbered from 1: `  3. dowel  fits`, `  8. chigiri  refused:
  error[overlap]: case.base and case.base_joints.key[0] overlap (and 1
  more)`, `not tried: choose <names> first`. A setting that isn't the
  chosen entry's is an error.
- A change step: `<summary>: <where it stands>` (`Make 3 of drawer:
  fits`); a removal waiting on what uses the item: `Remove back: used by
  drawer_bay, drawer; 'include dependants' removes them too`.
- `grips`: one grip a line, `  1. divider.height  500mm  moves
  180mm..1329.5mm (up)` or `  4. apron_front.apron.length  360mm
  follows overall_width, legs.inset, oak_45_sq.width (a size of the
  stock) (across)`; `stops`: each searched grip with `down to <value>`
  and `up to <value>`, and what breaks it one step further (`179.5mm
  breaks it: error[require]: ...`), the end of its range, or as far as
  the search looks.
- `drag`, `grip`, `pick`: the change line (`Set divider.height to 420mm:
  fits`; `Drag apron_front.apron.length to 380mm: pick which size
  changes: overall_width, inset`; `Drag legs.leg[front_left].length to
  400mm: refused: can't reach 400mm: the rule 'knee_room' stops it
  (...)`), then, indented, `changes <path> <from> -> <to>` when the
  solver chose or a param was picked, and `could change <param>: <from>
  -> <to>` for each choice still to pick from.
- `measure`: `<a> to <b>: <distance>`; `hold` without a length says
  `the distance is <distance> now` first.
- `show`: the card. For an edit, the blame lines (as after `set`) come
  right under `verdict:`, before the checker's words. For a change: its
  summary, what it found (indented),
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
- **Assemblies.** The component's name in lower case when it is one
  word (`Drawer` gives `drawer`). **A name of two or more words gives
  its last word**, the thing it is (`FramePanelDoor` gives `door`,
  `HangingRail` gives `rail`), so a mirrored pair reads `door_left`, not
  `frame_panel_door_left`; a last word that is a number or a single
  letter keeps the word before it (`PanelA` gives `panel_a`,
  `WidePanel_2` gives `panel_2`). Then `_2`, `_3` for the next ones. A
  joint or a piece of hardware is named from its kind's whole name
  (`dado_2`, `side_mount_slide_2`).
- **Repeats.** A repeat of 2 or 3 is a labelled array, its labels from the
  direction of the split: up `[bottom, top]` or `[bottom, middle, top]`;
  across `[left, right]` or `[left, middle, right]`; deep `[front, back]`
  (and `[front, middle, back]`). A repeat of 4 or more is a counted array.
- **Bays.** The region a repeated component fills is the assembly's base
  name plus `_bay`: `drawer_bay[bottom, top]`, `door_bay[left, right]`.
- **Mirrored pairs.** The name plus its side: `door_left`, and
  `door_right = mirror(door_left, about: ...)`. A name that already ends
  in a side of the mirrored axis has it replaced, never doubled:
  mirroring `door_left` across keeps it and adds `door_right`. A side
  of another axis is part of the name and stays: mirroring `leg_front`
  across gives `leg_front_left` and `leg_front_right`.
- **Held rules.** Named from the two things measured, joined by `_to_`,
  plus `_min` or `_max`: `slat_to_apron_front_min`. A word already in
  the name is not written again (`rail.rail` to `divider.panel` gives
  `rail_to_divider_panel_min`; `drawer.front` to `drawer.back` gives
  `drawer_front_to_back_min`). The reason the user types is its
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
to the project's `resolution:`, and the shares written are small whole
numbers: every piece's size counted in steps of the resolution, divided
by what the counts have in common (pieces of 150mm, 225mm and 125mm at
0.5mm are 300, 450 and 250 steps: `shares: [6, 9, 5]`). A later grip
reads them back as plain numbers. A piece whose size is not a whole
number of steps (a split of an odd size) is counted to the nearest
step, which moves it by less than a step.

**Spaced.** A part placed along a span is repeated with `spaced`
(`EditCommand::RepeatPart`, `repeat::repeat_part`): the part becomes an
array, labelled from the axis for two or three, and its placement term
on that axis becomes `spaced(evenly, from: <anchor>, to: <anchor>)`. On
a part that is already an array it changes the count.

**Mirror.** When the selection has a mirror image in the design (a
component placed against one of two parts that are mirror images, or in
one of two regions of a split across), the tool offers it, and writes the
pair with `mirror(...)` about the mid-plane between them, renaming the
source with its side (`door_left`). Failing those, a component placed
against parts that each have a part of the same size at the same place
from the other side of one plane has a mirror image too: an apron
between the two front legs of a `Legs`, whose back legs are the front
ones reflected front to back, is mirrored `about: mid(top.board, deep)`,
the plane written as the mid-plane of a part it names (else of a region)
that the plane halves. A plane the component itself lies astride (the
apron's mid-plane across) is not offered: its image would be itself.

**A mirrored pair from one.** A component that fills a whole space and
is handed (a door filling the space above a divider, hinged to the left
side) is made a pair by splitting the space in two (`mirror door
across`). Handed means one of its place params that takes a single part
or face (`hinge_side : Part`) is bound to one off the middle of the
space along the split; a top, a case, a shelf or a divider, whose places
are the space itself or both sides at once, is the same either way round
and is not offered the pair (`stave edit` still makes it when asked, and
the check decides). An item that already has its mirror image is offered
neither. The split: the space becomes `<component>_bay`
as a repeat of two would make it, the original is renamed with the first
side and fills the first piece, and its mirror image fills the other,
about the space's mid-plane:

```
region door_bay[left, right] = split(divider.above, across)
assembly door_left : casework.FramePanelDoor { opening: door_bay[left] ... }
assembly door_right = mirror(door_left, about: mid(divider.above, across))
```

A plain repeat across would hang both doors on the side the first was
hinged to; the mirror hangs the second on the other side.

## Hold this

The user measures a distance between two things and pins it with a
minimum or a maximum. It becomes a `require`:

```
require slat_to_apron_front_min : between(slat[0], apron_front) >= 12mm "room for a finger"
```

The value pinned is the distance as it is now, and the user may type
another; the message is the reason the user types. The card offers the
distance as it is now (`Session::measure(first, second)`, worked out
from the solved design by `hold::measured_distance` in the same forms
the rule writes). Like any edit it is
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

- **A grip moves a plain number**: the envelope's params, component
  params (`set drawer.top_gap = 20mm`) and shares. A grip shows its
  value, the range it may move in (the param's `within`, or for a share,
  up to the sum of the two it shares with) and its stops: the values of
  a `within` list, or a step of the project's `resolution:`.
- **A size that is an expression** shows what it follows ("follows
  `overall_width`, `case.overhang`"), each with its kind (plain, solved,
  locked, a size of the stock, a size of the hardware or joint), and is
  dragged through what it follows ([Smart edits](grammar.md#smart-edits)):
  - **solved params**: when the size depends on `solved param`s, the
    solver chooses which change and by how much (rule 3), keeping every
    `require` and `within`;
  - **one plain param**: the drag inverts it exactly (rule 2);
  - **several plain params**: the card lists them, each with the value it
    would take, and waits for the user to pick one;
  - **none**: the card says what stops it ("nothing this size follows
    can change: maple_12.thickness (a size of the stock)").

The pure functions are `grips::grips` (the grips of a selection: an
assembly's params then its parts' lengths and widths, the envelope's
three sizes, or a split's boundaries), `stops::find_stops` (how far a
plain number can go each way before the design stops checking),
`grips::move_grip` and `move_grip_by` (the patch for a drag), and
`solve_edit::Drag` (the solver's drag).

### Grips in the session

The session lists grips and drives a drag with the same commands and
events as every other edit (see the tables in [The edit in
progress](#the-edit-in-progress)); `stave_app::gripping` joins the pure
functions.

- **Listing.** `ListGrips(selection)` lists the grips at once, without
  their stops (`GripSelection::Assembly(id)`, `Envelope` or
  `Split(piece)`); `Session::grips` gives the `GripList`. Each
  `ListedGrip` holds the grip (`SelectionGrip`: its size, its label
  `divider.height`, the world axis it drags along, `GripAction::Moves`
  with value, range, steps and kind, or `GripAction::Follows` with what
  it follows) and its stops (`GripStops::NotSearched`, `NotSearchable`
  for a follows grip or a boundary, `Found { below, above }`, or
  `Failed`). A length grip of an assembly carries what the scene's
  `grip_place` needs and can't know itself (`SelectionGrip::place`,
  `PlaceInput::Length`): every length param of the assembly with its
  value, and the two faces the length is measured between; the
  envelope's grips carry the box and the face that moves
  (`PlaceInput::Envelope`), a boundary its two pieces and the axis
  (`PlaceInput::Share`). `GripList::contract(index)` gives the
  contract's `Grip` where one can be made.
- **Stops, without waiting.** The stop search is tens of checks a grip,
  so it is never run by a command. `Session::grip_stops_job()` gives a
  `StopsJob` (it can be sent to another thread: it prepares the project
  itself); `StopsJob::run(cancel, found)` searches each movable grip in
  turn and hands each answer (`FoundStops`) to `found` as soon as it is
  known; the window hands each to `Session::grip_stops_found`, which
  narrows the grip's range to its two last good values and says
  `GripStopsFound(index)`. Setting the cancel flag stops the job between
  checks; the grips reached keep their answer. An answer for an older
  listing is dropped (`GripList::ticket`). `Session::find_grip_stops`
  runs the job in place, for a script.
- **A drag** is a change of the design: `BeginDrag(index)` starts it
  (dropping what was in progress, keeping the grips listed),
  `DragTo(value)` moves it, and each move writes the patch again and
  tries it as an edit in progress is tried: the verdict and the ghost
  (`Change::ghost`) are of the value as it now is. `Add` ends the drag
  by writing it through the one writer; `Cancel` drops it.
  `TypeGripValue { grip, value }` begins and moves in one, for a value
  typed in the card. `Change::detail` is `ChangeDetail::Grip(GripDetail)`:
  where it was moved (`target`), what changes and to what (`changes`,
  each a `ParamChange { param, path, from, to }`), the plain params to
  pick from (`choices`, each with the change it would make or why it
  can't) and the one picked (`picked`, by `PickParam(param)`), what stops
  it (`stopped`), what blocks the solver (`blockers`), whether the
  solver chose (`solved`), and the size the written values reach when
  snapping to the resolution made them miss (`reached`).
  `Change::progress` is *needs a choice* (the plain params by name)
  while a pick is waited for, and *can't be written* with the reason
  when nothing can change or the solver can't reach the value. A commit
  makes the listed grips stale, so the session drops them
  (`GripsChanged`).
- **The solver in a drag.** When the size depends on solved params, the
  first move starts the solver's `Drag`: it solves once, then gives the
  params' values by exact arithmetic along the drag and solves again only
  at a bound (a `within` end, a require's edge, a param back at its
  start). Each move reports the params that will change and to what
  (`GripDetail::changes`, on the resolution) before the drag ends. A
  value no answer reaches is refused naming what blocks it: "can't reach
  400mm: the rule 'knee_room' stops it (leave room under the aprons for
  your knees)" (`GripDetail::blockers`, `Blocker::Require { name,
  message }`).
- **Solved and plain.** `MakeSolved { param, low, high }` turns a plain
  param of the project into `solved param ... within low..high` (a solved
  param needs its range, which the user types); `MakePlain(param)` takes
  `solved` off again and keeps the range, which still limits it. A
  `locked param` is refused (no edit changes it), as is a param that
  isn't the project's own (`param_kind::param_kind_patch`).

## What is not in the first build

Do not add these; each is a later decision:

- component warnings ("Case wants a back");
- a `fill` size;
- turned or tapered legs;
- automatic linking of repeated numbers;
- applicability guards beyond `require`.

**z3 is in the build.** An early brief listed z3 as out of the first
build; the owner decided on 19 November 2026 that it is required.
`crates/stave-smt` (z3 built from source, about five minutes the first
time) is the only crate that depends on it, and `stave-app` uses it to
solve for `solved param`s (`solve_edit.rs`, the window's "Let the
solver move this"). Plain params are still moved by exact single-param
inversion, with no solver.

## Libraries this is meant for

Ordinary libraries, outside the prelude (`import "std:casework"`):
`std:casework` (`Case`, `Back`, `Divider`, `Shelf`, `HangingRail`,
`Feet`, `Drawer`, `FramePanelDoor`) and `std:frames` (`Top`, `Legs`,
`Apron`, `Stretcher`, `Cleat`, `Slats`). Each exposes its place params, a
default for each setting where sensible, each size a user would drag as
a param, and the regions a later pick fills (`case.inside`,
`divider.above`, `divider.below`). Nothing in the editor names them.
Their place params carry `//` comments, not `///` doc comments, so the
card's prompt for them is the fallback "Point at the under." rather than
a sentence.

## Both pieces from nothing, in the window

The acceptance of this feature: the clothing cabinet and the side table
built from an empty project in the real window, driven by its scripted
input (`tools/window-edits/acceptance/run`, see
[gui.md](gui.md#the-acceptance-run)). Not every step was a gesture: the
faces were clicked through the scene's real pick, but the steps that
open a menu in a space or between two parts, and those that fill a place
between two parts, find it by name and send the command the click would
send, so those two clicks were not exercised in the window.
Both projects the window writes are the same text, byte for byte, as
`tests/edit-scripts/clothing_cabinet.stave` and `side_table.stave`, which
`stave edit --script` writes from the session. What the window could not
do by gesture, and what it does badly, is listed in
[STATUS](STATUS.md#graphical-editing).
