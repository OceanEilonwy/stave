# Language review 2 (2026-11-16)

Independent review of `docs/grammar.md`, the examples, `std/` and the Rust
syntax tree against the proposal's goals. To be applied as **grammar draft 3**.
Decision recorded with the user: apply all findings, except keep `locked`
(smart edits need an explicit "never change" marker).

## Findings (most important first)

1. **Reserved words used as names (bug).** `part` as a role name in every joint
   kind; `joint` as a base kind after `kind x :`; `at:` as a named argument in
   `split(..., at: divider)`; `.place` and `.stock` after a dot; `hardware` as a
   module name. Fix: rename the default role `part` to `piece` (so `makes part`
   stays unambiguous); allow any keyword after `.` and before `:` in a named
   argument or field; either unreserve `joint` in base-kind position or use a
   distinct form; module names must be non-keyword identifiers, otherwise `as`
   is required (also covers hyphenated folder names).
2. **Body separators and newline rules (bug).** `body = "{" , { entry , sep } , "}"`
   requires a separator after the last entry, but the lexer drops the newline
   before `}`, so `{ depth: 9mm }` fails. Fix every block to
   `"{" , { sep } , [ entry , { sep , { sep } , entry } ] , { sep } , "}"`.
   Do not drop newlines after `:` or `=` (or `length:` + newline can't report
   "has no value" on its own line). Require `NL` between top-level items. List
   exactly which keywords suppress a following newline.
3. **Name-collision rules contradict each other (bug).** Parts named `back`,
   `front`, `bottom` vs face/direction words; prelude-name rule vs `hardware
   pull`, component param `slide`, param `opening`, label `hinge`. Built-ins
   collide with themselves: `slide` (base kind and insert step), `drill` (cut
   op, cutting step, tool, recipe target), `turn` (shape and step), `length`
   (axis, field, `adds: length(...)`), `gap`/`inset`/`between` (function and
   argument). Fix: four explicit namespaces — items; names after `.` or before
   `:`; call heads fixed by position (placement terms, selectors, cut ops,
   steps); bare enum words resolved by expected type. A clash is an error only
   when ambiguous at that position. Rename self-collisions: insert step
   `slide` → `push`, cutting step `drill` → `bore`, `adds: length(p, d)` →
   `adds: extend(p, by: d)`.
4. **Settings in two places; kind values undefined (expressiveness).** Joint
   settings in the body, hardware settings in the call; `method:`, `glue:`,
   `count:` share the namespace; partially applied kinds
   (`side_mount_slide(length: ...)`, `euro_hinge(...)`, `knob(...)`,
   `bar_pull(...)`) are undefined and never declared. Fix: **all settings go in
   the call; the body only holds `place`.** A call with no roles is a kind
   value; using it later supplies the roles. Roles and params share one
   namespace per kind. Declare parametric hardware families, e.g.
   `kind side_mount_slide : slide { param length ... envelope: box(length: length, ...) }`.
5. **`inside(joint)` depends on a housing no kind can describe
   (expressiveness).** Rabbet cuts `crossing(into, part)` while `back` is placed
   `inside(back_rabbets)`: circular. Nothing bounds the back vertically; `orient`
   is forbidden with `inside`; `adds:` only extends length. Fix: **delete
   `inside`.** Place panels with ordinary terms and let `fit` come from joints;
   allow `adds` in width too.
6. **`[*]` has two meanings; nested lists undefined (expressiveness).** Fix:
   `x[*]` always means all and flattens inside a list; elementwise is a separate
   word, e.g. `drawer_bay[same]`. Labelled arrays are indexed by label only.
7. **Role semantics underspecified (gap).** Define: every listed part must meet
   at least one counterpart, otherwise an error; role params like
   `into.thickness` are evaluated per pair; the positional argument binds the
   first declared role; replace positional roles (`into[0]`, `into[1]`) with
   named roles (`roles { lock: Part, first: Part, second: Part }`); define
   `cut_first` for 3+ roles and made parts.
8. **`makes part` collides and has no placement; joints can't consume hardware
   (bug/gap).** kanawa `key` is both a param and a made part — rename the param
   `key_thickness`. Let a made part carry `place { }` with selector anchors.
   Add `uses: fastener(count: ...)` so dowels/screws reach the hardware list.
9. **Recipe selection and vocabulary (gap).** "First recipe in this file"
   contradicts "file order doesn't matter"; `prefer:` undefined; candidate
   recipe modules undefined. Fix: project field `methods: [...]` as an explicit
   preference list, then std defaults by declared `rank:`. `method:` is per
   joint but recipes are per cut operation. Tools are undeclared bare words —
   declare them (`kind sumitsubo : tool { }`). Allow `with: tool` on steps so
   libraries can add marking methods. `before(op)`/`after(op)` can't name a cut.
   No recipes exist for `dovetail_socket`, `dovetail_tail`, `outline`, `slot`.
10. **`steps { }` stores what should be computed (simplify).** Delete it; allow
    one optional `note: "..."`.
11. **Patches can't reach much (expressiveness).** Component internals
    (`add ... to casework.Drawer`, `remove part casework.Drawer.back`), imports,
    place terms other than single axes (`.place.orient`), unset/upsert, rename;
    unnamed things (profile stations, cuts, poses).
12. **Example placements contradict term definitions (bug).**
    `against(opening.bottom, gap: 20mm)` puts the drawer side below the opening
    (use `flush(opening.bottom, inset: 20mm)`); `cover(...)` region depth
    undefined; `orient(length: back)` with default `width: back` conflicts —
    define partial-orient defaults or require both; `center(between: a, b)` →
    `center(from: a, to: b)`; `inset:` on `spaced` vs `Spacing::Ends { inset }`;
    `corners` by list position → bind by labels; regions "placed like parts"
    but are `= expr` only; hardware mounting face never named; `ari_tsugi`
    insert direction vs sample orientation.
13. **Examples fail their own checks (bug).** kanawa `room` require fails on the
    samples; `hanger_room` fails at the low end of `cabinet_depth`; a 500mm
    slide fails at 450mm depth; two 180mm boxes fail at low `drawer_zone`; the
    foot tenon is counted in the profile and again by `round_tenon`. Decide:
    requires must hold over the whole declared range, and the diagnostic offers
    to tighten `within`.
14. **Three parameter syntaxes (simplify).** One `param` form everywhere
    (projects, components, kinds); type needed only without a default; ranges
    allowed on component and kind params. (Keep `locked`.)
15. **Mirroring an assembly underspecified (gap).** Define mirror on results:
    geometry is reflected and each connection re-binds to the part occupying
    the reflected location; error if none. Drop the textual left/right swap.
16. **Syntax tree disagrees with the grammar (bug).** Store ordered
    `Vec<Entry>` enums in bodies, kind bodies and recipe bodies; project and
    library bodies must not accept `place`; placement types need hardware
    placement and elementwise arrays.
17. **No field schema; type list incomplete (gap).** One table of fields per
    declaration; add `Mount`, `Parts`, `Shape`, `Envelope`, `Anchor`,
    `Direction`, `Axis`; regions use `across`/`up`/`deep` sizes.
18. **Smaller points.** Remove `on(...)` or accept the alias; accept `1/3in`
    (evaluation is rational); restrict `part x = expr` to `mirror`; define which
    profile segments `smooth:` applies to.

## Simplification plan (adopted)

1. All settings in the kind call; bodies only hold `place`.
2. One `param` form everywhere; delete the `params { }` block and component
   paren list (keep `locked`).
3. Delete `inside(joint)`.
4. Delete `steps { }`, keep optional `note:`.
5. `[*]` means all, `[same]` elementwise, labelled arrays by label only, named
   roles instead of `into[0]`.

## Resolution (grammar draft 3, 2026-11-16)

Every finding is applied in `docs/grammar.md` (draft 3), in the syntax tree
(`crates/stave-lang/src/{ast,placement,units,lib}.rs`) and in every `.stave`
file under `examples/`, `std/` and `tests/broken/`. `locked` is kept, as
decided. Section names below refer to `docs/grammar.md`.

| # | Finding | Resolution |
|---|---|---|
| 1 | Reserved words used as names | The grammar has two word classes: `IDENT` (never a keyword) for new names and `NAME` (any word) after `.` and before `:`. So `.stock`, `.place`, `at:` and role names are free. The default role is `piece` (was `part`). The base kind after `kind x :` is its own rule, `base_kind`, which lists `joint`. A module whose folder name is a keyword or not a word needs `as` (`import "./hardware" as hw`); without it: `error[reserved-word]`. In the tree: `ast::Ident` rejects keywords, `ast::Name` accepts them. |
| 2 | Body separators and newlines | One template, `block(X)`, is the only place `{ }` and separators appear; it allows separators before the first and after the last entry, so `{ depth: 9mm }` parses. `file` requires a newline before every import and item. The lexer drops a newline in exactly two cases (inside `( )`/`[ ]`, and after a listed token); the list is in "Newlines" and in `stave_lang::LINE_CONTINUATION_TOKENS`, and a test keeps the two equal. `:`, `=` and `{` are not on it, so `length:` + newline is `error[missing-value]`. `within` is the only keyword that suppresses a newline. `;` is no longer a separator. |
| 3 | Name collisions | "Names" defines four namespaces (items; members and argument names; call heads fixed by position; bare enum words by expected type) and six rules. A clash is an error only when a position accepts both readings (`error[ambiguous-name]`). The prelude is the outermost scope, so local names may reuse prelude names. Renamed: insert step `slide` → `push`; cutting step `drill` → `bore`; `adds: length(p, d)` → `adds: extend(p, by: d)`. Also renamed for the same reason: marking step `centre` → `centre_mark` (it took a feature called `centre`), shape function `turn` → `turned` (there is a cutting step `turn`), recipe target for turned parts → `turning`, and the round-tenon cut operation is `spigot` (the joint kind is `round_tenon`). |
| 4 | Settings in two places; kind values | All settings are arguments of the kind call; a joint has no body and a hardware body holds only `place`. "Kind values" defines a call without roles as a kind value and says how using it supplies the roles. Roles and params share one namespace per kind. `std/hardware` declares the parametric families `side_mount_slide`, `euro_hinge`, `knob` and `bar_pull`, with envelopes. The base kinds are a closed list with a field table each ("Declarations and their fields"); `knob` is a kind of base `pull`, so there is no separate `knob` base. |
| 5 | `inside(joint)` | Deleted. Panels are placed with ordinary terms. A size written `fit` comes from the joints the part is the `piece` of ("Sizes"), and `extend` adds in whichever direction the parts butt, so a back panel grows in its width and a shelf in its length with the same kind. `fit` no longer has a "placed against faces at both ends" case: that is written `between(a, b)`. |
| 6 | `[*]` and nested lists | `x[*]` always means every element and flattens inside a list; lists never nest. `x[same]` is the elementwise form. A labelled array is indexed by label only. In the tree: `ast::Index::{All, Same, At}`. |
| 7 | Role semantics | "Roles and pairs": a kind with a `Parts` role has exactly two roles and applies to every pair that meets; a kind with three or more roles has only `Part` roles. "Meet", "butt" and "cross" are defined. Every listed part must meet a counterpart (`error[no-contact]`). Kind expressions are evaluated per pair. The positional argument binds the first declared role. Positional role lists (`into[0]`) are gone: roles are named (`roles { lock: Part, first: Part, second: Part }`). `cut_first:` takes a role or an ordered list; unlisted roles follow, made parts come last. |
| 8 | `makes part`; fasteners | The kanawa param is `key_thickness`; the made part is `key`. A made part has a `place { }` with `orient(length:, width:, of: role)` and terms that take face selectors (`center(crossing(piece, into))`). `uses:` takes fastener kind values with `count:`; `std/hardware/fasteners.stave` declares `dowel_pin` and `wood_screw`, used by the `dowel` and `screw` joints, so they reach the hardware list. |
| 9 | Recipe selection and vocabulary | "Choosing a recipe" is a three-step rule: candidates by module, filter by the project's `tools:`, then the joint's `methods:`, the project's `methods:`, the lowest `rank:`, and a name tie-break. `prefer:` and "first in this file" are gone. `methods:` is a list of recipes; each applies only to cuts of its own operation, which settles "per joint vs per cut". Tools are declared (`kind square : tool { }`, 35 of them in `std/tools`), so a misspelt tool is `error[unknown-name]`. Every step accepts `with: <tool>`, and `line(feature, with: tool)` is a generic marking step (see `outline_with_sumitsubo` in the Japanese library). `before(op)`/`after(op)` are defined as "every cut of that operation on the same part"; order between the parts of one joint is `cut_first:`, and a `transfer` step orders a cut after its mate. `std/joinery/recipes.stave` has a power and a hand recipe for `notch`, `slot`, `pocket`, `tenon`, `spigot`, `drill`, `dovetail_socket`, `dovetail_tail`, `outline` and `turning`. |
| 10 | `steps { }` | Deleted. A kind may have one `note: "..."`, which adds advice to the manual and never adds or orders steps. |
| 11 | Patches | New operations: `add ... to <component>`, `add import`, `remove import`, `replace <item>` (for things without a name of their own: profile stations, cuts, poses), `unset`, and `rename ... to`. `set` adds the value if it is missing. A table lists what a path can address, including kind-call settings, assembly arguments, `.place.<axis>` and `.place.orient`, and items inside a component of a local module. |
| 12 | Placements vs term definitions | `against` and `flush` are defined precisely; the drawer side uses `flush(opening.bottom, inset: bottom_gap)`. `cover(region, mount, thickness: d)` gives the region its depth, and `opening(...)` takes six anchors (`front:` and `back:`), so every region is a complete box. `orient` requires both `length:` and `width:`, except for round parts. `center(between: a, b)` is `center(from: a, to: b)`. The inset of `ends` belongs to the spacing value: `spaced(ends(inset: d), ...)`, matching `placement::Spacing::Ends`. `corners` binds by the labels `front_left`, `front_right`, `back_left`, `back_right`. Regions are no longer "placed like parts": they are always one expression. "Hardware placement" names the mount face for each base and adds `face:` to choose it. `ari_tsugi` now inserts along the piece's thickness, which is "from above" for the sample beams lying flat. |
| 13 | Examples fail their own checks | "Checks and `require`" says a require must hold over the whole declared range, with `error[require-range]` and a help line that tightens `within`. The examples are fixed: `cabinet_depth` is `within 550mm..650mm` (hanger room, and room for a 500mm slide); the drawer box height is `opening.up - bottom_gap - top_gap`, with a require, so it fits every `drawer_zone`; `cabinet_height` starts at 1400mm; the foot's profile no longer contains the tenon, which the `round_tenon` joint adds; the kanawa sample beams are 1200mm, so the splice fits. Also fixed while checking them: the door panel was 12mm in a 19mm frame (now 6mm in a 6mm groove), the door knob sat at a fixed 450mm (now centred, so it fits every door height), the dowel joints named the wrong part as the piece, and the hinge's pivot is defined so an overlay door clears its cabinet side. |
| 14 | Three parameter syntaxes | One form: `param name [: Type] [= default] [within domain] [locked]`, in modules, components and kinds. The `params { }` block and the component's parenthesised list are gone. In the tree: `ast::Param` with `ast::ParamSpec`, which can't hold "neither type nor default". |
| 15 | Mirroring | "Mirroring" defines it on results: the solved source is reflected, and each connection of a mirrored assembly that refers outside it re-binds to whatever occupies the reflected place (`error[mirror]` if nothing does). There is no textual swap of `left` and `right`. |
| 16 | Syntax tree vs grammar | Rewritten rule for rule. Bodies are ordered `Vec`s of entry enums (`PartBody`/`PartEntry`, `KindBody`/`KindEntry`, `RecipeBody`/`RecipeEntry`); project, library, stock and assembly bodies are `FieldBlock`s, which can't hold `place`; a component holds `DesignItem`s only. `placement.rs` has `HardwarePlacement` (mount face plus the open axes), per-element array placement (`Translation::Spaced` with an `ArraySlot`, `Corner::translations`), a validated `Orientation`, and no `Inside`. Tests in `lib.rs` read `docs/grammar.md` and fail if the keyword list, the contextual words, the newline list, the base kinds, the item tags, the units, the types, the operations or the placement words differ from the code. |
| 17 | Field schema; type list | "Declarations and their fields" has one table per declaration, including each hardware base. "Types, units and arithmetic" lists every type (`ast::TypeName`), with `Mount`, `Parts`, `Shape`, `Envelope`, `Anchor`, `Direction` and `Axis` added, and kind types per base (`SlideKind`, `HingeKind`, `PullKind`, `FastenerKind`). Regions have the sizes `across`, `up` and `deep`. |
| 18 | Smaller points | `on(...)` is removed (`against(base.top)` says the same). `1/3in` is a valid literal: literals are exact fractions (`units::ExactLength`), and only stored results are whole nanometres. `part x = expr` is restricted to `mirror(...)` in the grammar itself. `smooth:` is a `Bool`; a shoulder splits the curve and a segment between equal stations stays straight. |

### Simplification plan

| # | Plan | Done |
|---|---|---|
| 1 | All settings in the kind call; bodies only hold `place` | yes; `joint` has no body at all |
| 2 | One `param` form everywhere (keep `locked`) | yes |
| 3 | Delete `inside(joint)` | yes |
| 4 | Delete `steps { }`, keep optional `note:` | yes |
| 5 | `[*]` all, `[same]` elementwise, labels only, named roles | yes |

### Other decisions made while applying the review

These were not in the review; they were needed to make the spec precise or
the examples correct.

- **Base kinds are closed**: `joint`, `slide`, `hinge`, `pull`, `fastener`,
  `tool`. A kind can't extend another kind; a family is a kind with params.
- **Motions belong to assemblies.** The slides or hinges inside a component
  move the whole instance. A hinged assembly turns about the outer long edge
  of the hinge's piece nearest the part the hinge is fixed to. This is a
  simplification of a cup hinge's real path.
- **An assembly body has no `place`**: a component places its own parts
  against the regions and parts it is handed.
- **`schematic: true`** on a joint kind skips the overlap check between its
  parts (used by `chidori`, whose cuts don't describe the whole node).
- **A project without `tools:`** is assumed to have every tool.
- **`check` in a recipe is a field** (`check: "..."`), like everything else.
- **`style:` is `Text`**, because suggestion styles are not a closed list.
- **Diagnostic codes and message shapes are fixed** in "Diagnostics", and
  `tests/broken/` holds twelve small projects that must each produce one.
- Removed as unused: `movement_gap(...)` (a groove's `clearance:` is the
  movement gap) and the `prefer:` recipe field.
- The cabinet's hardware kinds no longer name `.glb` model files that don't
  exist, so the example checks without warnings. The `model:` syntax is
  unchanged and documented under "Hardware".
