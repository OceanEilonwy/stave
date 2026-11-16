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
