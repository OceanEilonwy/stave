# Special cases audit (2026-11-17)

An audit of `docs/grammar.md` and the crates below `stave-gtk` for behaviour
that names one joint or hardware kind, reads one kind's catalogue fields, or
lets one pairing off a check. Phase 1 of the "generalise kinds" push: this
file is the audit and the approved design; the staged plan is
`docs/workpacks/2026-11-17-generalise-kinds.md`. No code was changed.

**Why.** Joint and hardware kinds are meant to be `.stave` definitions that
anyone can add; the checker should handle any kind out of the box. Joints
already work that way. Hardware does not: a hinge, a slide and a pull are
each a block of Rust. The cost showed in commit `cc1243c`: cup-hinge links
went 0.8mm into the door's wood and no check said so, because a
hinge-only allowance in the simulation and in the swing check called it
"the stop". The geometry is fixed. The structure that hid it is still there
and is what this audit lists.

Every line number was read in the worktree at commit `9070b46`.

## Summary

| Where | Findings | GENERALISE | JUSTIFIED |
|---|---|---|---|
| Spec (`docs/grammar.md`) | 25 | 19 | 6 |
| `stave-lang` | 6 | 4 | 2 |
| `stave-schema` | 12 | 7 | 5 |
| `stave-model` | 11 | 9 | 2 |
| `stave-solve` | 21 | 17 | 4 |
| `stave-geom` | 13 | 10 | 3 |
| `stave-check` | 14 | 10 | 4 |
| `stave-scene` | 4 | 3 | 1 |
| `stave-app` (UI lane: hand-offs) | 8 | 7 | 1 |
| `stave-cli`, `stave-gtk` (out of scope: hand-offs) | 2 | 1 | 1 |
| `tools/` (Python) | 3 | 3 | 0 |
| **Total** | **119** | **90** | **29** |

The five known examples are all confirmed (S6/SC1/SO1, GE1 to GE3, S8/SC5,
CH1, S22/SO17). The pattern behind the other findings:

1. **Joints are clean; hardware is not.** No core crate names a joint kind
   in code, with two small exceptions (CH13, AP6: `"dado"`, `"rabbet"`,
   `"groove"` matched as text to pick a noun). Every piece of hardware
   behaviour, though, is a `match` on `BaseKind::Hinge | Slide | Pull` or on
   `HardwareData::Hinge | ButtHinge | Slide | Pull`. 157 lines under
   `src/` in eight crates name one of those variants, `Motion::Linkage` or
   a `MemberName` (schema 47, model 17, solve 27, geom 3, check 12,
   scene 15, app 27, cli 9).
2. **A cup hinge is not a kind, it is a field-name test.** A hinge kind is
   a cup hinge when it has an entry called `cup_diameter` (and four more),
   a butt hinge when it has one called `leaf` (SC4, M1, SO3). Everything
   after that branch is a second code path.
3. **Hardware has exactly two solids, by construction.** `Hardware` holds
   one `envelope` (moves with the piece) and one optional `fixed` member.
   A cup hinge's arm, knuckle, links and pins hang off a side field
   (`mechanism`) that only hinge code reads. The links are in no collision
   list at all (CH3); they are reached by one hinge-only sweep (GE2).
4. **Checks that are really `require`s.** `slide-gap` and `hinge-overlay`
   are catalogue rules written in Rust with their own diagnostic codes and
   their own help text (CH1, CH4).

### Surprises

- **Steel is still drawn in wood, twice, and no check can see it.** The
  viewer draws the cup's rim 0.8mm thick on the door's back face, 2.5mm
  past the bore (`stave-geom/src/hinge.rs:22-23`). On every overlay door in
  the examples that puts the rim 0.8mm into the front edge of the cabinet
  side, where the door closes against it. The Ø3mm pins are drawn too; the
  outer boss pin is 1.39mm from the door's back face, so it stands 0.11mm
  proud of the door, over the side's front edge (the clothing cabinet's
  dump gives its pin at `(12.96, -17.61)` on a 19mm door with 18mm of
  overlay). Both are mesh-only numbers: no solid, no check. This is the
  same class of fault as the one just fixed, and the generic collision
  model will report both. See gap 1.
- **`hinge-overlay`'s help can suggest a value the kind rejects.** The help
  searches `cup_inset` over `0..8mm` and `plate_height` over `0..9mm`
  (`MAX_INSET_NM`, `MAX_PLATE_NM`), not over the kind's own `within` lists
  (`[3mm, 4mm, 4.5mm, 5mm, 6mm, 7mm]`), so it can print `cup_inset: 2.5mm`.
- **A drawer always travels toward the front.** The spec says a slide
  travels along its piece's length, out through the front of its `in`
  region; the code hardwires `Direction::Front` (SO8), and the default
  mount face only ever looks left or right (SO4).
- **An assembly's motion is its first hardware's motion.** The spec rule
  that all hinges of an assembly share one pivot line is not checked (SO16;
  `docs/STATUS.md` says so). Two different hinges on one door move on the
  first one's linkage.
- **Swept wood has no holes.** The motion sweeps use each part's boxes and
  the boxes around its rods; cuts are left out (GE4). That is safe for wood
  against wood (more wood than there is) and is the reason the hinge needed
  its own sweep that knows about one bore.
- **Glue-ups do not exist.** `max_width` is stored and readable; nothing
  splits a part. The only test asserts that a too-wide part gives no error
  (SO17).
- **`stave-lang` has three unused per-base placement types**
  (`HardwareTerms::{Fixed, One, Two}`, L3).
- **The acceptance test for "no steel in wood" lives in the UI lane's
  crate** (`crates/stave-app/tests/focus.rs:737`) and calls the hinge-only
  functions directly. Routing it through the generic path means editing
  that file.

---

## Findings by crate

Verdicts: **GENERALISE (X)** names the design element (A to K, below) that
replaces it; **JUSTIFIED** says why it stays.

### Spec: `docs/grammar.md`

| # | Lines | What it says | Why it is there | Verdict |
|---|---|---|---|---|
| S1 | 30 | `base_kind` is a closed list: `joint`, `slide`, `hinge`, `pull`, `fastener`, `tool` | Review 2 decided bases are closed | JUSTIFIED: kept as vocabulary. After G a base only supplies words ("door opens", "drawer pulls out") and a param type; it decides no behaviour |
| S2 | 31-36, 772-774 | `member` is a bare word; hardware kinds "can't have `roles`, `cut`, `makes` or the joint fields" | Hardware was catalogue data only; the base did the rest | GENERALISE (B, C, F, G) |
| S3 | 738-742 | Fixed roles and required/optional entries per base (`slide`, `hinge`, `pull`) | The Rust for each base reads these names | GENERALISE (G: `roles { }`; B: member shapes replace `envelope`, `plate`, `leaf`; other entries become ordinary fields and params the kind chooses) |
| S4 | 743-744 | `fastener` needs `diameter`, `length`; `tool` has `title`, `powered` | `uses:` and the hardware list read a fastener's size; recipes read tools | JUSTIFIED: a two-field contract with the hardware list and the work plan, the same for every fastener and tool. *Checks gaps stage 4:* still justified, and the hardware list now reads the same two entries for a joint member's fastener (`member pin : dowel_pin(...)`, once per copy placed) as for `uses:`. A fastener may also write `species:` (a species word, as a stock's), which only the viewer reads, to draw every placed copy in that wood (steel without it); the core names no fastener kind for it |
| S5 | 746-750 | A hinge is a cup hinge or a butt hinge by which entries it has | One base, two mechanisms | GENERALISE (C: the kind's `mechanism { }` says how it moves) |
| S6 | 752-761 | "Every cup hinge moves on Stave's one built-in linkage"; `opening_angle` at most `110deg` | The pins were kept out of `.stave` files as "coordinates" | GENERALISE (C, D). The pins are measurements within the hardware, like a profile's stations |
| S7 | 763-770 | `base_overlay`, `plate_height`, `overlay_adjustment` and the overlay formula | Catalogue rule for one family | GENERALISE (H) |
| S8 | 776-808 | "Hardware members": words fixed per base | The viewer knew which mesh each word was | GENERALISE (C: any name; B: its shapes say what it is) |
| S9 | 907-916 | Default mount face, what the base fixes, which axes `place { }` sets, per base | Placement written once per base in Rust | GENERALISE (G) |
| S10 | 918-928 | How an envelope lies on the mount face; "front ends level" for slides | One envelope per hardware | GENERALISE (B, G) |
| S11 | 933-955 | Where a cup hinge's cup, plate, arm and links go; what must stay out of wood | Describes SO10 and GE2 | GENERALISE (A, C, G, H). The sentence "nothing is excused as a stop" becomes the general rule for everything |
| S12 | 956-968 | Butt hinge: pin edge rule, two leaf mortises | Describes SO9 | GENERALISE (F, G: `edge(piece, flush_with: on)`) |
| S13 | 969-972 | A slide with a box envelope is two members, each half its thickness | Describes SO8 | GENERALISE (B, C) |
| S14 | 1190-1191 | Fasteners "are never placed and take no part in collision checks" | A joint knows a count, not positions | JUSTIFIED, and restated by A as the one declared "not modelled" case |
| S15 | 1348-1351 | `slide`: checks `clearance` (`slide-gap`), travels toward the front | Base behaviour | GENERALISE (H: a `require`; C: a rail and a drive) |
| S16 | 1352-1365 | `hinge`: the base bores the cup, picks the linkage's plane, cuts leaf mortises; "a hinge has no `envelope:`" | Base behaviour | GENERALISE (F, C) |
| S17 | 1366-1368 | `pull`: the base bores one or two screw holes | Base behaviour | GENERALISE (F) |
| S18 | 1372-1375 | Envelopes; "an envelope may overlap the hardware's own role parts" | A cup must sit in its door, and nothing cut the room for it in the model | GENERALISE (A: the kind's cut makes the room; the exemption goes) |
| S19 | 1377-1387 | "Models are for looks only"; `align: bounds` within 0.5mm | Checks must not depend on a mesh | GENERALISE (E: still for looks, but checked against the text) |
| S20 | 1617-1634 | Motions come from "the slides, or the hinges"; "an assembly with both hinges and slides has no motion"; a sentence each for cup and butt hinges | Two motion types, by base | GENERALISE (C, D: one mechanism per assembly, from whatever hardware it holds) |
| S21 | 1722-1723 | Diagnostic codes `slide-gap`, `hinge-overlay` | One code per catalogue rule | GENERALISE (H) |
| S22 | 653-654 | `max_width`: "wider parts are planned as glue-ups"; `max_length`: longer is an error | The cut-list mockup showed glue-ups | GENERALISE (I) |
| S23 | 1915-1916 | Types `HardwareKind`, `SlideKind`, `HingeKind`, `PullKind`, `FastenerKind` | A component asks for "a slide" | JUSTIFIED: the type of a preset, used for params |
| S24 | 1268-1300, 1449-1469, 1929-1932 | Closed tables: cut operations, their features, assembly steps, faces, axes, units, species, stock forms | Geometry and recipes are written per operation | JUSTIFIED: these are the language's primitives. A new kind is built from them; a new primitive is a spec change (rule J) |
| S25 | 726-729 | Every joint call accepts `glue:` and `methods:` | Apply to any joint | JUSTIFIED: uniform, not per kind |

### `stave-lang`

| # | Where | What it does | Why | Verdict |
|---|---|---|---|---|
| L1 | `ast.rs:596-643` | `BaseKind`, `is_mounted_hardware` | S1 | JUSTIFIED (S1) |
| L2 | `ast.rs:693`, `703-709` | `KindEntry::Member(KindMember { name })`: a member is only a name | S8 | GENERALISE (B, C: a member has a body) |
| L3 | `placement.rs:719-814` | `HardwareTerms::{Fixed, One, Two}`, `HardwarePlacement`, its error: zero, one or two open axes, "a slide", "a hinge", "a pull". Used nowhere outside this file | S9 | GENERALISE (G); delete |
| L4 | `diagnostics.rs:284`, `286` | Codes `SlideGap`, `HingeOverlay` | S21 | GENERALISE (H) |
| L5 | `parser.rs:1623-1628`, `1670` | `member` parsed as a bare word; kind entries expect `param`, `require`, `makes`, `member` | S2 | GENERALISE (B, C, F) |
| L6 | `lib.rs:216-217` | Contextual words `slide`, `hinge`, `pull`, `fastener`, `tool`, `member`, ... | Lexing | JUSTIFIED |

### `stave-schema`

| # | Where | What it does | Why | Verdict |
|---|---|---|---|---|
| SC1 | `hinges.rs:1-117` | `CupPoint`, `CupLink`, `CupMechanism`, `CUP_HINGE_110`: four pins, link widths, arm and knuckle sizes, pin diameter, `max_opening` | S6 | GENERALISE (C). Moves to `std/hardware/hinges.stave` verbatim |
| SC2 | `kinds.rs:45-65`, `79-82` | `MountRule::{NearerRegionSide, Face}`, `OpenAxes::{None, AlongPieceLength, Both}`, and the two `BaseSpec` fields | S9 | GENERALISE (G) |
| SC3 | `kinds.rs:136-157` | `SLIDE`: roles `piece`, `in`; `length`, `clearance`, `travel`, `envelope` | S3 | GENERALISE (G, B) |
| SC4 | `kinds.rs:159-174` | `CUP_HINGE`/`BUTT_HINGE` conditional requirements; `CUP_HINGE_ENTRIES`, `BUTT_HINGE_ENTRIES` | S5 | GENERALISE (C) |
| SC5 | `kinds.rs:176-283` | `MemberName` (11 words), the four per-base member tables, `members_of(base, cup)` | S8 | GENERALISE (C) |
| SC6 | `kinds.rs:285-320` | `HINGE`: roles `piece`, `on`; the cup, plate and leaf entries; `base_overlay`, `plate_height`, `overlay_adjustment` | S3, S7 | GENERALISE (G, H) |
| SC7 | `kinds.rs:322-338`, `380-381` | `PULL`: `envelope`, `centres`, `diameter`, `screw_diameter`; `DEFAULT_SCREW_DIAMETER_NM` | S17 | GENERALISE (F: a param default in the kind) |
| SC8 | `kinds.rs:340-367` | `FASTENER`, `TOOL` | S4 | JUSTIFIED |
| SC9 | `kinds.rs:384`, `388` | `FACE_SETTING`, `RESERVED_JOINT_SETTINGS` | Settings every call of that sort accepts | JUSTIFIED |
| SC10 | `fields.rs:113-134` | Stock fields that depend on `form: dowel` | A dowel has a diameter, not a thickness | JUSTIFIED: a stock form, not a kind |
| SC11 | `operations.rs`, `recipes.rs`, `words.rs` | Cut operations, recipe steps, closed word lists | S24 | JUSTIFIED |
| SC12 | `functions.rs:25-26`, `189` | `overlay(...)`, `inset(...)` mounts | Region vocabulary | JUSTIFIED: about doors and openings, not about a hinge |

### `stave-model`

| # | Where | What it does | Why | Verdict |
|---|---|---|---|---|
| M1 | `resolve/kinds.rs:305-352` | Decides cup or butt from entry names; three hinge-only errors | S5 | GENERALISE (C) |
| M2 | `resolve/kinds.rs:371-374`, `388-430` | `lower_members`: member words checked against the base's table; "a cup hinge has no member ..." | S8 | GENERALISE (C) |
| M3 | `defs.rs:735-745` | `MemberDef { name: MemberName, .. }` | S8 | GENERALISE (C) |
| M4 | `solved.rs:1082-1101` | `HardwareBase::{Slide, Hinge, Pull}` | Which words a viewer uses | JUSTIFIED (S1) |
| M5 | `solved.rs:1106-1155` | `HardwareData::{Slide, ButtHinge, Hinge, Pull}` with `cup_diameter`, `cup_depth`, `cup_inset`, `overlay`, `centres`, ... | One variant per code path | GENERALISE (B, C, G): members, roles and readable entries |
| M6 | `solved.rs:1159-1180` | `OverlayRange` and `nominal()` | S7 | GENERALISE (H) |
| M7 | `solved.rs:1193-1232`, `1240-1287` | `Hardware { envelope, fixed, mechanism }`; `CupMechanism`, `LinkBar`, `HardwareMember`; `is_butt_hinge()` | Two solids, plus a cup hinge's extras | GENERALISE (B, C): a list of members, each with shapes |
| M8 | `solved.rs:1291-1362` | `Pivot`; `Motion::{Turn, Linkage, Travel}`; `turns()`, `swing()` | One variant per hardware type | GENERALISE (C, D): one solved mechanism. A single pivot and a single rail are its simplest cases |
| M9 | `linkage.rs` (whole file) | `Linkage`: exactly two links, four pins, exact dead-point check | The cup hinge | GENERALISE (D). Its exact circle-circle check is kept as the solver's kernel |
| M10 | `solved.rs:636-640`, `805-818` | `CutSource::Hardware`; `CutPlace::Recess` and `CutPlace::Mount`, cut places only base code can make | S16, S17 | GENERALISE (F: `crossing(piece, member)`) |
| M11 | `types.rs:119-127`, `212-217` | Kind types per base, and their names in messages | S23 | JUSTIFIED |

### `stave-solve`

| # | Where | What it does | Why | Verdict |
|---|---|---|---|---|
| SO1 | `hardware.rs:16`, `837-853`, `946-1018`, `1188-1299` | Reads `CUP_HINGE_110`: places the pins (`cup_linkage`), builds the arm and knuckle boxes (`cup_arm`), caps the opening at 110° (`989-1002`, `out-of-range`) and reports "a cup hinge's links lock" (`1003-1016`, `motion`) | S6 | GENERALISE (C, D) |
| SO2 | `hardware.rs:26-72`, `99-108` | `Data::{Slide, ButtHinge, Hinge, Pull}`, `OverlayRangeExact`, `Mechanism`, `HingeStyle::{Cup, Butt}` | M5 | GENERALISE (B, C) |
| SO3 | `hardware.rs:329-378` | Picks the hinge style by looking for `"leaf"`, then `"cup_diameter"`, `"cup_depth"`, `"cup_inset"`; makes the envelope from them | S5 | GENERALISE (B, C) |
| SO4 | `hardware.rs:384-406` | Default mount face: `MountRule`; for slides compares left and right only | S9 | GENERALISE (G: `face(piece, nearer: in)`) |
| SO5 | `hardware.rs:409-414` | An envelope stands off the face, "except a hinge cup's or leaf's, which runs into the piece" | Sunk hardware | GENERALISE (B: a shape's `depth` range says which way; F cuts the room) |
| SO6 | `hardware.rs:453-509` | Per base: a slide starts at the piece's low end, centred on its width; a cup sits `cup_inset` from the edge nearer `on`; a butt leaf starts at the flush edge | S9, S12 | GENERALISE (G) |
| SO7 | `hardware.rs:522-526` | Which axes `place { }` may set, from `OpenAxes` | S9 | GENERALISE (G) |
| SO8 | `hardware.rs:607-666` | Slide: splits the envelope into two half-thickness members; reads `travel`, `length`, `clearance`; motion is `Travel { toward: Direction::Front }` (`662`) | S13, S15 | GENERALISE (B, C) |
| SO9 | `hardware.rs:667-756`, `1026-1066` | Butt hinge: reflects the leaf into `on`, checks the faces meet (its own `no-contact`), makes two recesses, a `Turn` about the edge | S12 | GENERALISE (C, F, G) |
| SO10 | `hardware.rs:757-899`, `1072-1173` | Cup hinge: bore, plate from `"plate_setback"` and `"plate"` (`hinge_plate`), linkage frame, overlay, and `"base_overlay"`, `"plate_height"`, `"overlay_adjustment"` (`868-880`) | S11, S7 | GENERALISE (C, F, G, H) |
| SO11 | `hardware.rs:901-926` | Pull: reads `"centres"`, `"diameter"`, `"screw_diameter"`; one or two through holes | S17 | GENERALISE (F) |
| SO12 | `eval.rs:751-753` | `screw_diameter` defaults to 4mm when the kind is a pull | S3 | GENERALISE (a param default in the kind) |
| SO13 | `build.rs:184-196`, `301-369` | `bore_cut`, `recess_cut`: turn a base's bore and recesses into cuts | S16, S17 | GENERALISE (F) |
| SO14 | `build.rs:782-996` | Mirroring hardware: re-binds `on` or the region per `Data` variant, reflects each motion variant and the cup mechanism by hand | Mirror needs every outside reference | GENERALISE (G: re-bind every role; C: reflect one mechanism) |
| SO15 | `build.rs:1298-1365` | Writes `HardwareData` and `CupMechanism` per variant | M5, M7 | GENERALISE (B, C) |
| SO16 | `build.rs:1439-1458` | An assembly's motion is its first hardware's; mixed turns and others give none; nothing checks that hinges agree | S20 | GENERALISE (D: the rule for several items) |
| SO17 | `build.rs:141`, `1110-1111`; `joints.rs:668` | `max_length` is checked; `max_width` is only stored | S22 | GENERALISE (I) |
| SO18 | `build.rs:527-585` | Reads a fastener's `diameter` and `length` | S4 | JUSTIFIED. *Checks gaps stage 4:* now `build.rs:876-972` (`finish_joint`), for `uses:` and for every joint member that names a fastener alike; `member_item` (`build.rs:995`) reads the fastener kind's name and optional `species` into `MemberItem` on the placed `Body`, the same for every kind. `drill(count:)` and its spreading (`solid.rs` `hole_centres`) are gone: a row of holes is a joint member's |
| SO19 | `build.rs:1641-1655` | Reads a tool's `title` and `powered` | S4 | JUSTIFIED |
| SO20 | `joints.rs:235-250` | The call settings `glue`, `methods` (joints) and `face` (mounted hardware) | S25 | JUSTIFIED |
| SO21 | `place.rs:434`; `eval.rs:1017-1034`, `1086-1088`; `build.rs:1099`, `1170` | `StockForm::Dowel` | SC10 | JUSTIFIED |

### `stave-geom`

| # | Where | What it does | Why | Verdict |
|---|---|---|---|---|
| GE1 | `sweep.rs:645-678`, `727-855` | `MechanismMember::{Arm, Knuckle, InnerLink, OuterLink}`, `HingePart::{Piece, On}`, `Mechanism` (one hinge's polygons and its bore), `MEMBERS` | The links were in no body list | GENERALISE (A, B) |
| GE2 | `sweep.rs:866-951` | `mechanism_contact`: sweeps those four members against the piece (less one bore) and `on` | The swing check for a hinge's own steel | GENERALISE (A). Its loop (conservative advancement of two things that move differently, then bisection) is the general sweep; keep the loop, drop the member list |
| GE3 | `sweep.rs:953-1016` | `SteelInWood`, `steel_in_wood`: the worst depth per member, by sampling | Measured the 0.8mm fault | GENERALISE (A): becomes a general "deepest overlap over a motion" for any two bodies |
| GE4 | `sweep.rs:36-41`, `57-64` | Swept solids are boxes and the boxes around rods; "holes are left out" | Simple, and safe for wood against wood | GENERALISE (A): a cut must make room in a sweep too, or no hardware can sit in a bore while it moves |
| GE5 | `sweep.rs:464-640` | `LinkageSweep`, `linkage_first_contact`, `linkage_contact`, `linkage_reach`: sweep of a solid carried by the two-link linkage | S6 | GENERALISE (D): the same sweep over any mechanism's path |
| GE6 | `linkage.rs:1-240` | `LinkagePath`: solves the two-link linkage in floats; speed bound | S6 | GENERALISE (D) |
| GE7 | `hinge.rs:1-281` | Meshes of a cup hinge: cup, rim (`RIM_WIDTH`, `RIM_THICKNESS`, `WALL` at `22-25`), pins, links, arm | Nothing else described the shapes | GENERALISE (B, E): draw what the kind declares; nothing drawn that is not declared |
| GE8 | `lib.rs:365-392` | `hardware()`, `hardware_fixed()`, `hardware_links()`: three mesh entry points shaped like a cup hinge | M7 | GENERALISE (B) |
| GE9 | `collide.rs:149-172`, `729-748` | `WorldSolid::of_envelope`; `Placement::of_motions` with a `Motion::Linkage` arm | M7, M8 | GENERALISE (B, C) |
| GE10 | `solid.rs:814`, `846`, `864` | Handles `CutPlace::Recess` and `CutPlace::Mount` | M10 | GENERALISE (F) |
| GE11 | `sweep.rs:100-160` (`slide_contact`), `411-436` (`turn_contact`); `clearance.rs:52-75` (`Travel::{Slide, Swing}`) | Exact slide; closed-form turn about one line; `Swing` takes any pose function | Fast, exact cases | JUSTIFIED: chosen by the shape of the motion (one rail, one fixed pivot), not by a kind. `Travel::Swing` is already the general form |
| GE12 | `sweep.rs:1018-1075` | `overlap_depth`, `clipped_area` on convex polygons | Used by GE3 | JUSTIFIED: general helpers; keep |
| GE13 | `solid.rs:405-415`; `sequence.rs` | Names of cut operations; assembly steps | S24 | JUSTIFIED |

### `stave-check`

| # | Where | What it does | Why | Verdict |
|---|---|---|---|---|
| CH1 | `hinges.rs:15-20`, `27-148` | `hinge_overlays`: `error[hinge-overlay]` from `cup_inset`, `overlay`, `overlay_range`; help from `MAX_INSET_NM`, `MAX_PLATE_NM` | S7 | GENERALISE (H) |
| CH2 | `hinges.rs:155-258` | `mechanisms`: calls GE2 for assemblies whose motion is `Motion::Linkage`; names "the arm", "the knuckle", "the inner link", "the outer link" | S11 | GENERALISE (A): the general sweep names the member as the kind does |
| CH3 | `lib.rs:227-258` | Bodies: one for `envelope`, one for `fixed`; a cup hinge's arm and knuckle are folded into its plate; links are not bodies | M7 | GENERALISE (A, B): one body per member |
| CH4 | `lib.rs:190`, `570-637` | `slide_gaps`: `error[slide-gap]`; `misfit_slides` | S15 | GENERALISE (H) |
| CH5 | `lib.rs:663-672` | `exempt`: hardware never clashes with its own `piece` or `on` | S18 | GENERALISE (A): delete |
| CH6 | `lib.rs:653-658` | `exempt`: a slide already reported for its gap clashes with nothing | Avoids a second error for one mistake | GENERALISE (H): "hardware whose `require` or `adjust` failed is left out of the overlap check", for any kind |
| CH7 | `lib.rs:660-662` | `exempt`: two members of one piece of hardware never clash | A product's own parts | JUSTIFIED as a general rule, pending gap 2 |
| CH8 | `lib.rs:673-677` | `exempt`: pairs whose cut can't be made yet; parts already reported for spacing | `not-supported`; one error per mistake | Split: the spacing arm is JUSTIFIED (one error per mistake); the can't-cut arm is GENERALISE: delete once angled cuts are made (work pack stage 2a) |
| CH9 | `motion.rs:47-100` | `Sweeper::{Turn, Linkage, Travel}` | M8 | GENERALISE (D) |
| CH10 | `motion.rs:112-120` | `motion_hardware`: the first `Hinge` or `Slide` | S20 | GENERALISE (C: the hardware that has a drive) |
| CH11 | `motion.rs:433-437` | A hit before 30° on a `Motion::Linkage` gets the gap help | Cup hinges swing their edge out | GENERALISE (D): ask the mechanism whether the hit is on the hinge-edge side, not which variant it is |
| CH12 | `motion.rs:424-466` | "opened past", "pulled out past", "the hinges open to", "the slides travel" | Beginner words | JUSTIFIED: the preset's vocabulary (G). They must come from the base, not from the motion variant |
| CH13 | `explain.rs:238-254` | `cavity_noun`: looks for `"dado"`, `"rabbet"`, `"groove"`, `"mortise"`, `"housing"`, `"socket"` inside the joint kind's name | Friendlier help | GENERALISE: see gap 9 (a kind names its own cut) |
| CH14 | `explain.rs:170-182`; `motion.rs:222-345` | "the tenon", "the round tenon" from the cut operation; clearance help in the mount's words | S24, SC12 | JUSTIFIED |

### `stave-scene`

| # | Where | What it does | Why | Verdict |
|---|---|---|---|---|
| SN1 | `graph.rs:95-200` | `LinkFollower`, `RenderMotion::{Turn, Linkage, Travel}` | M8 | GENERALISE (C, D): each node follows its member's pose |
| SN2 | `graph.rs:423-481` | Node names "fixed leaf", "plate and arm", "plate", "cabinet member", "inner link", "outer link" | M7 | GENERALISE (C): the member's own name |
| SN3 | `graph.rs:1027-1049` | Exploded view: a fixed member goes with `on`, or for a slide with whatever part touches it | No role says what a slide is fixed to | GENERALISE (G: each member says which role it is with) |
| SN4 | `focus.rs:639-695`, `778-810` | `edge_point`, `hinge_view`, `Plan::slide`: where to look at a thing that turns or travels | Presentation | JUSTIFIED: keyed on how it moves, not on a kind |

### `stave-app` (the UI lane's crate: hand-offs, not changes in this push)

| # | Where | What it does | Verdict |
|---|---|---|---|
| AP1 | `view.rs:164-193`, `217-240`, `253` | "Cup", "Plate", "Leaf", "Drawer member"; "deep" for a hinge's cylinder; a "Mechanism" row that says "four-bar linkage" | GENERALISE (C): hand-off H2 |
| AP2 | `focus.rs:702-705` | `on` from `HardwareData::Hinge \| ButtHinge` | GENERALISE (G): H2 |
| AP3 | `focus.rs:744-746`, `754`, `818`, `877-882` | "How a cup hinge works", "How a butt hinge works", "Opens to", "Pulls out", "Slide check" | GENERALISE: headings from the base's words and the kind's doc comment; `is_butt_hinge()` goes. H2 |
| AP4 | `focus.rs:932-984` | `member_anchor`: one arm per `MemberName`; finds links by node names ending in "link" | GENERALISE (B): anchor on the member's solved bounds. H1 |
| AP5 | `focus.rs:1020-1023` | Plan caption chosen by `heading.contains("slide")` | GENERALISE: from the motion. H2 |
| AP6 | `focus.rs:389-395` | A notch is called "Dado", "Rabbet" or "Groove" by the joint kind's name | GENERALISE: gap 9. H2 |
| AP7 | `focus.rs:538-545` | A part that turns about the up axis is shown from above ("the chidori's lock" is only the comment) | JUSTIFIED: a rule about the step, not the kind |
| AP8 | `tests/focus.rs:737-797` | `no_cup_hinge_puts_steel_in_wood`: filters on `HardwareData::Hinge`, calls `steel_in_wood` and `mechanism_contact` | GENERALISE (A): re-routed in stage 3 |

### `stave-cli`, `stave-gtk` (out of scope to change)

| # | Where | What it does | Verdict |
|---|---|---|---|
| CL1 | `stave-cli/src/dump.rs:229-246`, `257-322`, `402-420` | Prints `Recess` and `Mount` cuts, one line per `HardwareData` variant, "cup"/"plate"/"arm"/"knuckle" rows, "on a cup hinge's linkage" | GENERALISE: hand-off H3. The dump snapshots are this push's safety net, so the text must not change until the stage that explains it |
| GT1 | `stave-gtk/src/screenshot.rs:575`; doc comments in `main.rs`, `screenshot.rs` | A test and help text use the node name "plate and arm" | JUSTIFIED as written; the name changes with SN2 (hand-off H1) |

### `tools/` (Python)

| # | Where | What it does | Verdict |
|---|---|---|---|
| T1 | `tools/hinge-sim/linkage.py` (283 lines) | Seven checks of the cup hinge against the catalogue; copies the pins from `hinges.rs` by hand (`53-63`) | GENERALISE (K): Rust tests that read the kind |
| T2 | `tools/hinge-sim/fit.py` (200 lines) | Finds the pins by differential evolution | GENERALISE (K): an ignored Rust test |
| T3 | `tools/joint-sim/chidori.py` (259 lines) | Proves the chidori's turn and slide by sampling | GENERALISE (K): mostly covered by `stave-geom`'s sequence check already; port what is not |

---

## The approved design

Spec first: each element below is a change to `docs/grammar.md`, then code.
Decisions A to K were made with the user; details marked *draft* are to be
settled in the stage that writes that part of the spec.

### A. One collision model

**Rule (replaces grammar 1372-1375, 951-955, 1630-1633).** Every solid in
the design is a body: every part with its cuts made, and every shape of
every hardware member, wood or steel. Each body is posed by the state of
the motions. One overlap check covers every pose that is checked at rest
or in a scene; one continuous sweep covers every motion. Two bodies may
touch (within the 0.05mm contact tolerance). They may share room only where
a cut has made it: a bore, a recess, a mortise, a pilot hole. Hardware cuts
its own room the way a joint does (F). There is no way to switch the check
off and no pair is let off.

- The own-parts exemption (S18, CH5) is deleted. A cup sits in its door
  because the kind bores the hole.
- The hinge-only sweep (GE1 to GE3, CH2) is deleted. Links, arms and
  knuckles are bodies like any other.
- **Sweeps see cuts.** In the plane of a motion, the wood of a part is its
  boxes less its cuts wherever that can be done exactly or by taking away
  no more than the cut does (a round hole crossed by the plane is taken as
  its narrowest chord over the other body's width, as `Mechanism::wood`
  does today for one bore). Where it cannot, the wood is left whole: a
  contact may be reported that a finer model would clear, never the
  reverse.
- **Screws.** A screw that is a member (a knob's screw, a bar pull's two)
  is a solid, and the kind drills its hole (F): it fits its hole exactly
  and touches. Screws that only fix hardware to wood (plate screws, leaf
  screws, slide screws) are fasteners: a hardware kind may list them with
  `uses:`, as a joint does, so they reach the hardware list. Fasteners named
  in `uses:` are **counted, not placed, and not modelled for collision**.
  That is said once in the spec, for joints and hardware alike, and is the
  only thing not modelled. Why: `uses:` knows how many and what size, not
  where; a screw's length against the board it enters is a `require` in the
  kind (as `screw`'s `stays_inside` is today).
- Diagnostics do not change shape: `overlap` at rest and in scenes,
  `motion` in a sweep, naming the member as the kind names it
  ("`door_l.hinges[upper]`'s inner_link hits `door_l.stile[hinge]` when
  `door_l` is opened past 98.2deg").

### B. Collision shapes in text, required

Every member of a hardware kind has one or more **shapes**. A member with
no shape is `error[missing-field]`. Shapes are convex and are written in
the kind's own frame (C), closed:

| Shape | Meaning |
|---|---|
| `box(along:, across:, depth:)` | a box; each argument is a range (`-2.9mm..15mm`) or one length, which is centred on the origin |
| `cylinder(diameter:, depth:, at:)` | a cylinder whose axis runs along `depth`; `axis: along` or `axis: across` lays it down |
| `prism(points: [...], along:)` | a convex outline in the across/depth plane, extruded along `along` |
| `bar(from:, to:, thickness:, along:)` | a straight bar between two points, half its thickness past each: a prism, written the way a link is made |

Sizes may use the kind's params and fields, so `side_mount_slide(length:
450mm)` and `euro_hinge(cup_inset: 3mm)` are still families. A member with
two shapes names them (`shape body: ...`, `shape knuckle: ...`).

The sweep keeps today's method, made general: convex pieces in the plane of
the motion, conservative advancement, then bisection to the contact
tolerance (`sweep.rs`). No mesh against mesh.

`envelope:`, `plate:` and `leaf:` go away; `box(length:, width:,
thickness:)` and `cylinder(diameter:, length:)` remain for fasteners.

### C. Mechanisms as data in the kind

**The kind's frame (new spec text, beside 1244-1246).** A hardware kind is
drawn in its own frame, closed, as a catalogue drawing is: `along` runs
along the piece's length, `across` runs across the mount face from the edge
the kind's `mount` names toward the middle of the piece, `depth` runs from
the mount face into the piece (negative stands off it). These are the words
a cut already uses on a face. They are **measurements within the
hardware**, like a profile's stations and a cut's marks: catalogue data,
never a position in a design. `point(...)` outside a `kind` is
`error[syntax]`.

**Members** are declared by the kind, with any name; the fixed table
(S8, SC5) is removed. Each is one rigid piece of metal.

**`mechanism { }`** says how the members move:

| Entry | Meaning |
|---|---|
| `pivot a, b at point(...)` | `a` and `b` turn on one pin there |
| `slot a, b from point(...) to point(...)` | a pin on `a` runs in a straight slot in `b` |
| `rail a, b along <axis>` | `a` runs straight along `b` without turning |
| `couple a to b: 1/2` | `a` travels that share of `b`'s travel (a full-extension slide's middle member) |
| `drive: turn(m, from:, to:)` or `travel(m, from:, to:)` | the one input, and its range |
| `stop: <value> "message"` | what stops it there; the message is shown when a pose or a design asks for more |

Draft, as it will be written in `std/hardware/hinges.stave` (the numbers are
`CUP_HINGE_110`'s, unchanged):

```
kind euro_hinge : hinge {
  roles {
    piece: Part                      // the door's hinge stile
    on:    Part                      // the cabinet side
  }
  param opening_angle = 110deg within 85deg..110deg
  param cup_inset     = 5mm within [3mm, 4mm, 4.5mm, 5mm, 6mm, 7mm]
  param plate_setback = 37mm within [37mm, 52mm]
  param plate_height  = 0mm within [0mm, 1.5mm, 3mm, 6mm, 9mm]
  cup_diameter: 35mm
  cup_depth:    12.8mm

  // On the back of the stile, the cup's rim cup_inset in from the edge
  // nearer the side. The design says where along the stile.
  mount {
    face:   back
    across: from(edge(piece, nearer: on), inset: cup_inset + cup_diameter / 2)
    along:  open
  }

  /// The cup: a steel cup pressed into a 35mm hole bored in the back of
  /// the door. The boss in its middle is where the links are pinned.
  member cup {
    with:  piece
    shape: cylinder(diameter: cup_diameter, depth: 0mm..cup_depth)
  }
  /// The mounting plate: screwed to the inside of the cabinet side.
  member plate {
    with:  on
    // 11mm thick, standing off the side's inside face, which is 5mm and
    // the plate's height from the cup's centre, on the hinge-edge side.
    shape: box(along: 50mm, across: -(5mm + plate_height)..6mm - plate_height,
               depth: -(plate_setback + 18mm)..-(plate_setback - 18mm))
    mount {
      across: against(face(on, toward: piece))
      depth:  from(crossing(on, piece), inset: plate_setback)
    }
  }
  /// The arm: it clips onto the plate and reaches out to the door.
  member arm {
    with:  plate
    shape body:    box(along: 16mm, across: -2.90mm..15.00mm, depth: -(plate_setback + 18mm)..0mm)
    shape knuckle: box(along: 16mm, across: 8.00mm..15.00mm, depth: 0mm..12.40mm)
  }
  /// The inner link: a steel bar between the arm and the cup.
  member inner_link {
    shape: bar(from: point(across: 13.50mm, depth: 10.90mm),
               to:   point(across: -8.80mm, depth: 7.33mm), thickness: 1.50mm, along: 9mm)
  }
  /// The outer link: a strap whose two sides pass either side of the inner link.
  member outer_link {
    shape near: bar(from: point(across: 13.28mm, depth: 3.56mm),
                    to:   point(across: -9.54mm, depth: 1.39mm), thickness: 1.50mm, along: 5.50mm..7mm)
    shape far:  bar(from: point(across: 13.28mm, depth: 3.56mm),
                    to:   point(across: -9.54mm, depth: 1.39mm), thickness: 1.50mm, along: -7mm..-5.50mm)
  }

  mechanism {
    pivot arm, inner_link at point(across: 13.50mm, depth: 10.90mm)
    pivot cup, inner_link at point(across: -8.80mm, depth: 7.33mm)
    pivot arm, outer_link at point(across: 13.28mm, depth: 3.56mm)
    pivot cup, outer_link at point(across: -9.54mm, depth: 1.39mm)
    drive: turn(cup, from: 0deg, to: opening_angle)
    stop:  opening_angle "the limiter in the arm stops the door here"
  }

  cut crossing(piece, cup) { drill(diameter: cup_diameter, depth: cup_depth) }
  adjust arm on plate: across within 2mm
}
```

A member `with:` a role's part moves with that part; `with:` another member
rides on it; a member with neither is placed by the mechanism. All of the
jargon (`pivot`, `slot`, `point`) is confined to library files: a project
still writes `hardware hinges[lower, upper] : euro_hinge(stile[hinge], on:
side_l)`.

A butt hinge is `pivot leaf, fixed_leaf at point(across: 0mm, depth: 0mm)`
with `drive: turn(leaf, ...)`. A slide is `rail drawer_member,
cabinet_member along along` with `drive: travel(drawer_member, from: 0mm,
to: travel)`. A pull has members and cuts and no mechanism. The 110° cap
(SO1) is no longer a rule: it is this kind's `stop` and its param's range.

### D. One closed-form planar solver

- **Planar, one degree of freedom, version 1.** Every pivot's pin is
  parallel to one axis of the piece; the mechanism moves in the plane
  across it. The spec says so. A kind whose pieces do not add up to exactly
  one freedom (three per moving member, less two per pivot or rail, less
  one per slot or coupling) is `error[mechanism]`, a new layer 3 code, at
  the kind.
- **Solved in steps, each closed form.** Starting from the members that
  are held (those `with:` a part outside the moving assembly) and from the
  drive, the solver repeats: find a member, or two members pinned
  together, held by exactly two things it can place:
  1. two links meeting at a pin, each pinned to a known point: circle and
     circle;
  2. a link pinned to a known point whose other pin runs in a known slot:
     circle and line;
  3. a member whose angle is known (the driven cup) hung on two links, or
     a link and a slot: its position is on a circle and a circle, or a
     circle and a line.
  A rail is direct. If no order exists, that is `error[mechanism]`: "this
  mechanism can't be solved one step at a time from its drive", with the
  members left over.
- **Which of the two crossing points** is the one the declared closed pose
  is on, and stays so by continuity. A step whose two points run together
  (a dead point), or whose branch would have to flip, anywhere in the
  drive's range, is `error[motion]` at the hardware, naming the angle or
  distance: "`<hardware>`'s links lock at 124deg, before it opens 125deg".
  The first step from the drive is checked exactly, in integers, as
  `stave_model::linkage` does today (its 0.1mm margin is kept); later steps
  are checked at the geometry boundary with a bounded step.
- No Newton, no iteration to convergence. Exact pins in, floats only in
  `stave-geom`.
- **Several items on one assembly.** An assembly has one motion. Its
  hardware falls in two groups: items with a `drive` and items without
  (followers: a lid stay). All pins must be parallel to one axis
  (`error[motion]` otherwise). All items with a drive must give the same
  motion: the same mechanism, in the same plane, seen along the pins;
  two cup hinges of one kind and setting on one stile do, a 110° hinge and
  a butt hinge on one door do not (`error[motion]`: "`door_l`'s hinges
  don't move alike", naming the two). Each follower is then solved with the
  assembly's pose given, by the same steps; where it cannot follow, that
  is `error[motion]` at the first angle. The assembly opens as far as the
  smallest `stop` among them. An assembly whose hardware has no drive has
  no motion.

### E. Models are for looks; the text is checked against them

The `.stave` numbers are the truth. A kind with a `model:` is linted: every
declared pivot and slot point, and the bounds of every shape, is compared
with the node of the same name in the `.glb`. A difference over 0.01mm is
`warning[model-mismatch]`, with the two values and help that names the
usual causes: glTF is metres with +Y up, Blender is Z up and often
millimetres; an object's scale or rotation was not applied before export.
`model("./x.glb", sha256: "...")` pins the file; another file is
`warning[model-mismatch]` too.

Authoring uses Blender's own glTF exporter. No Python in the repository.
Proposed for later, in `stave-cli` (another lane; not scheduled here):
`stave hardware import model.glb` drafts a kind from named nodes;
`--update` rewrites only the points and shape sizes of an existing kind, so
the change is a text diff a person can review; `stave hardware preview
<kind>` shows a kind on a scrap of wood.

### F. `cut` in hardware kinds

A hardware kind may have `cut` entries. A member takes the place of the
second part: `crossing(piece, cup)` is the face of `piece` the member
touches or enters, and the footprint is the outline of the member's shape
on it (one footprint per shape). The operations are the existing ones.

- cup: `cut crossing(piece, cup) { drill(diameter: cup_diameter, depth: cup_depth) }`
- butt leaves: `cut crossing(piece, leaf) { notch(depth: thickness) }` and the same for `on`
- pulls: `cut crossing(piece, screw) { drill(diameter: screw_diameter, depth: through) }`

This replaces the base-done cuts (S16, S17, SO13) and `CutPlace::Recess`
and `CutPlace::Mount` (M10). Recipes and work cards see them as they see a
joint's cuts.

### G. Roles and placement defaults

Hardware kinds declare `roles { }` as joints do; a role may also be a
`Region` (a slide's `in`). The kind's `mount { }` gives the defaults: the
face, and for each of `across` and `along` either a term or `open` (the
design's `place { }` sets it; setting an axis that is not open stays
`error[axis-set-twice]`). New selectors, usable in kinds only:

| Selector | Meaning |
|---|---|
| `edge(p, nearer: q)` | of the two long edges of the mount face, the one nearer the middle of `q` |
| `edge(p, flush_with: q)` | the long edge where `p`'s neighbouring face lies in a face of `q`; if neither or both do, the nearer one |
| `face(p, toward: q)` | the face of `p` that looks toward the middle of `q` |
| `face(p, nearer: r)` | the face across `p`'s thickness nearer a side of region `r` |

`crossing(on, piece)` already names "the face of `on` the piece closes
against". `hinge`, `slide` and `pull` stay as thin presets: they give the
verbs in messages and the param types, nothing else. Mirroring re-binds
every role, whatever its name (SO14).

### H. `adjust`, and catalogue rules as `require`

`adjust arm on plate: across within 2mm` says a member can be set that far
either way on the member that carries it. A member with its own `mount`
is drawn in the kind at its nominal place; the design may put it elsewhere
only by as much as an `adjust` allows, on that axis. The rest pose is
solved within the adjustments and everything is checked at the setting the
design needs. When it cannot be reached:

```
error[adjust]: door_l.hinges[upper]: the arm would sit 8.5mm off its place on the plate; it adjusts 2mm
   = help: bore the cup with cup_inset: 3mm, or use plate_height: 9mm
```

`error[adjust]` is a new layer 4 code. The help is a **general search**:
for each param of the kind with a list after `within`, try each value with
the others as given, and print the ones that fit. The same search runs
when a `require` of a kind that compares for equality fails. It replaces
`hinge-overlay`'s arithmetic and its two constants (CH1), and can only
print values the kind accepts.

Catalogue rules become `require` entries of the kind:

```
require gap : between(mount, in) == clearance "the slide needs exactly this much beside the drawer"
```

(*draft*: in a hardware kind, `mount` is the mount face as an anchor.)
`slide-gap` and `hinge-overlay` are removed from the diagnostics table;
both designs now fail with `require` or `adjust`. A general `covers(a, b)`
is not needed: the overlay rule is the arm's adjustment.

### I. Glue-ups: derived, not stored

- A part of `form: solid` stock whose cut width is more than its stock's
  `max_width` is **made of boards**: `ceil(width / max_width)` boards of
  equal width, the seams along the grain. Nothing is written in the file;
  boards are derived, as cut lists are, and appear as `top.board[1]` ...
  in the cut list and work cards. Placement, joints and collision see one
  part.
- **Length is not split.** A part longer than `max_length` stays an error:
  an end joint carries load and is a design choice. The help names it:
  "make it from two parts joined end to end (for example `ari_tsugi`), or
  buy longer stock". A sheet (`plywood`, `mdf`) wider than `max_width` is
  an error for the same reason.
- **How the boards are joined** is an edge-joint kind, named per part or
  once for the project:

  ```
  part top : oak_19 {
    boards: joined(by: tongue_and_groove)
  }
  project "Side table" {
    boards: joined(by: [tongue_and_groove(glue: false), suitsuki_zan])
  }
  ```

  The default is `joined(by: glued_edge)`. A list combines kinds.
- **New kinds in `std/joinery`**, ordinary `kind ... : joint` files:
  `glued_edge` (default), `tongue_and_groove`, `splined_edge` (makes the
  spline), `chigiri` (butterfly keys; makes the keys), `suitsuki_zan`
  (sliding dovetail battens; makes the battens). Three need no glue.
  Recipes choose how each cut is made with the user's tools, as for any
  joint.
- **A deliberate layout** (book-matched boards, a wide centre board): write
  the boards as parts and use the same edge-joint kinds directly.
- Wood movement of an unglued panel (a panel held only by battens must be
  free to move across them) is written as `require` rules in those kinds;
  a movement check over seasons is future work.

### J. A standing rule

Proposed text for `CLAUDE.md`, under "Crate boundaries" (not edited in this
push until stage 8):

> **No kind in the core.** `stave-lang`, `stave-schema`, `stave-modules`,
> `stave-model`, `stave-solve`, `stave-geom`, `stave-check`,
> `stave-workplan` and `stave-scene` never name a joint or hardware kind
> (`euro_hinge`, `dado`), a member of one (`cup`, `leaf`, `arm`) or a field
> only one kind has (`cup_inset`, `plate_height`), in code or in a string.
> Behaviour that a kind needs and the language cannot yet say is a new
> language primitive: a spec entry in `docs/grammar.md` first, then code
> that works for every kind. The bases `hinge`, `slide` and `pull` may
> choose words in messages; they may not choose behaviour. No check may
> let a particular pairing off: room is made by a cut or not at all.
> `crates/stave-check/tests/standing_rule.rs` reads the names out of
> `std/` and fails if a core crate uses one.

### K. Python oracles become Rust tests

`tools/hinge-sim/linkage.py`, `fit.py` and `tools/joint-sim/chidori.py` are
ported before they are deleted; nothing Python stays. The hinge checks
become tests that read the kind from `std/hardware`, so they follow the
data.

---

## Gaps the audit found in the design, and how to close them

1. **The rim and the pins (new).** Under A the cup's rim (0.8mm proud of
   the door, over the cabinet side's front edge) and the outer boss pin
   (0.11mm proud) are steel in wood in every example with overlay doors.
   Today they are drawn and not checked. Choices: (a) declare them and
   give the examples the gap between door and carcass that a real hinge
   needs, a design change to the examples and to `overlay(...)`; (b)
   declare only what is true now (the cup's body; pins of a diameter that
   stays inside the cup) and stop drawing the rest. Recommended: (b) in
   stage 3, so examples behave as now and nothing is drawn that is not
   checked, with (a) recorded as the follow-up it is. This needs the
   user's decision: either way a picture changes.
2. **Members of one piece of hardware against each other.** The arm's
   knuckle is declared as a solid box; the links are pinned inside it. A
   check of steel against steel inside one item would need every kind
   drawn as finely as its maker's drawing. Recommended: a general rule, in
   the spec, that a kind's own members are not checked against each other
   (Stave checks a design, not a product), with `stave hardware preview`
   as the place to look at one later. It is the same for every kind, so it
   is not an exemption for a pairing; but it is a limit, and should be the
   user's call.
3. **A butt hinge's knuckle.** The door turns about the wood's own edge
   line. A real knuckle is a barrel outside the two flush faces; declared
   as a shape on today's pivot line it would be in the wood of both parts.
   Recommended: stage 4 moves the butt hinge over unchanged (pin on the
   edge line, the `pin` member a shape that stays within the leaves'
   thickness), then one separate commit moves the pivot out by the
   knuckle's radius and explains the dump change (the box's lid).
4. **A drive on a member that is not pinned to anything held.** The cup's
   angle is the input, and the cup hangs on two links: step 3 of D. A
   mechanism with more links (a real 165° hinge is not a four-bar) may have
   no order of steps from that drive. D makes that a diagnostic, not a
   wrong answer; if the 165° kind needs it, the fix is a fourth step type
   in the spec, not a special case.
5. **The dump is in another lane.** `stave-cli/src/dump.rs` and
   `stave-app/src/{view,focus}.rs` read `HardwareData`, `fixed` and
   `mechanism`. The work pack adds the new contract beside the old one,
   fills the old one from a single shim module in `stave-solve`, and
   deletes both only after hand-offs H1 to H3. Until then the standing
   rule's test cannot pass for `stave-model` and `stave-solve`; that is
   why J lands in stage 8.
6. **Which setting is checked.** The Python oracle checked the side screw
   at five settings. Under H the design fixes the setting, so one is
   checked. The oracle's sweep over settings is kept as a test of the kind
   (stage 0), not as a check of every design.
7. **Edge joints need two things the language lacks.** A tongue is "all
   but a centred tongue along an edge": `tenon` is defined on an end only.
   A batten crosses every board of a panel: a joint kind with one `Parts`
   role, which "Roles and pairs" forbids. Stage 7 writes both into the
   spec (allow `tenon` on `crossing` for parts that butt edge to edge; a
   kind with a single `Parts` role is one connection across all of them).
   `chigiri` and `suitsuki_zan` use `outline` and `dovetail_socket`, which
   the geometry cannot cut yet. Resolved by adding stage 2a to the work
   pack: those cuts and angled slots become exact prisms (a polygon on a
   face, cut straight down), drawn and checked like every other cut. The
   can't-cut exemption (CH8) goes with them, and the examples' two
   `not-supported` warnings disappear.
8. **An assembly inside a moving assembly** is still not swept. Unchanged
   by this push; said in the spec's version 1 limits.
9. **A cut's everyday name (CH13, AP6).** "Dado", "rabbet" and "groove"
   are found by searching the kind's name. Proposed: a cut may be named,
   `cut dado: crossing(into, piece) { notch(depth: depth) }`, and messages
   use that name. It also gives patches a way to address a cut (review 2,
   finding 11). Small; fits in stage 2.
10. **Kinds outside `std/`.** `tests/broken/misnamed_member` and the test
    `a_cup_hinge_opens_at_most_110_degrees` declare hinge kinds with cup
    entries and no mechanism, relying on the built-in one. After stage 1 a
    hinge kind without a `mechanism` has no motion, and after stage 3 a
    member without a shape is an error. Both tests are rewritten to say
    what they now mean; the work pack lists them.
