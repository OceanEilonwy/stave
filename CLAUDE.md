# Stave

Stave is a woodworking CAD tool for beginners: a `.stave` text file describes a
project (parameters, stock, parts, joints, hardware, relationships), and Stave
checks it like a compiler, renders it in 3D, and produces a cut list and a PDF
build plan with a drawn work card for every part. Licence: AGPL-3.0-or-later.

## Sources of truth

- `docs/grammar.md` — the language spec. Code follows the spec; if the spec is
  wrong or ambiguous, fix the spec in the same change and say so.
- `docs/plan.md` — the current build push: scope, simplifications, crate
  ownership, interfaces and definition of done.
- `docs/reviews/` — design reviews and how they were resolved.
- `examples/`, `std/` — `.stave` files that are also the test corpus. Every one
  must parse and check once those stages exist.

## Guiding principles (they settle disagreements)

1. Super simple for a beginner.
2. Woodworking concepts, not geometry, in the language and UI.
3. Relationships only, never coordinates, in the language. (Internal solved
   positions are fine; they are never written to a `.stave` file.)
4. Errors are type errors: compiler-style diagnostics with file, span, message
   and a concrete help line.
5. Exact, always: lengths are `units::Length` (i64 nanometres); evaluation uses
   exact `units::Rational`; values are rounded once, in the cut list.
6. Store intent, compute results: positions, meshes, cut lists and the order of
   operations are derived, never stored.
7. Bring your own AI: patches use the same grammar.
8. Text is the source of truth.
9. Headless first: all behaviour lives in crates below `stave-gtk`.

## Rust standards

Code must be idiomatic, performant and correct, and use the type system to make
invalid states unrepresentable.

- Edition 2024. `cargo fmt` clean; `cargo clippy --all-targets -- -D warnings`
  clean; tests pass. Run all three before every commit.
- **Correctness by construction:**
  - Newtypes for ids and quantities (`PartId`, `Length`, `Angle`); never pass a
    raw `u32`, `i64` or `f64` where a domain type exists.
  - Enums over strings and booleans. Closed vocabularies (faces, directions,
    units, cut operations) are enums.
  - Separate types for separate stages (syntax tree → resolved model → solved
    design) so a later stage cannot receive unchecked data.
  - Constructors that validate (`fn new(..) -> Result<Self, _>`) and private
    fields when an invariant must hold; non-empty collections as types when
    emptiness is invalid.
  - `match` exhaustively; avoid `_ =>` on domain enums so new variants are
    caught by the compiler. `clippy::wildcard_enum_match_arm` enforces it:
    it is in `[workspace.lints]`, and every crate but stave-gtk takes it with
    `[lints] workspace = true`. Where a wildcard is right (a projection that
    picks out one variant, a type check whose every other value is an error,
    a foreign or non-exhaustive enum, the flat `SyntaxKind` tag set), put
    `#[expect(clippy::wildcard_enum_match_arm, reason = "...")]` on the
    narrowest statement or function, or write `if let` / `let ... else`.
- **No panics in library code:** no `unwrap`, `expect`, `panic!`, indexing that
  can fail, or lossy `as` casts outside tests. Use `TryFrom`, checked
  arithmetic and `?`. Library crates use `thiserror`; binaries use `anyhow`.
- **No `unsafe`:** every crate forbids `unsafe_code`, with one exception:
  `stave-gtk`'s `render/dmabuf.rs` (handing frames to GTK as dmabufs) is
  allowed it, `stave-gtk` denies it everywhere else, and each `unsafe`
  block there carries a `// SAFETY:` comment.
- **Floats** (`f32`/`f64`) only at the geometry and rendering boundary
  (`stave-geom`, `stave-scene`, `stave-gtk`, drawings in `stave-output`).
  Convert from exact values in one place.
- **Determinism:** outputs (diagnostics order, cut lists, PDFs, plans) must be
  identical run to run. Use `BTreeMap`/sorted `Vec`s where order is observable.
- **Performance:** no needless clones or allocation in hot paths; prefer
  borrowing; index by id rather than searching by name after resolution.
- Every public item has a doc comment. Every feature has tests, preferably
  driven by `.stave` corpus files; diagnostics get snapshot tests.
- Full words in names (`length`, not `len`; `connection`, not `conn`).

## Crate boundaries

Dependency order (a crate depends only on crates above it):
`stave-lang` → `stave-schema` → `stave-modules` → `stave-model` → `stave-solve`
→ `stave-geom` → `stave-check` → `stave-workplan` → `stave-scene` →
`stave-output` → `stave-app` → `stave-cli`, `stave-gtk`, `stave-lsp`.

`stave-ide` (the editor's navigation layer: names, uses, hovers, colours,
completion, open buffers) sits beside that chain after `stave-modules` and
before `stave-app`: it depends only on `stave-lang`, `stave-schema` and
`stave-modules`, and has no GTK or protocol types. `stave-lsp` (the
language server binary) is a front end like `stave-cli` and `stave-gtk`:
it builds on `stave-app` (for the slow diagnostics and worked-out values)
and on `stave-ide`, as `docs/grammar.md` ("Tooling and editors") says,
and holds only the protocol: positions, URIs and message plumbing.

- Only `stave-gtk` may depend on GTK, libadwaita or wgpu.
- `stave-gtk` contains no business logic: handlers call into `stave-app`.
- **No kind in the core.** `stave-lang`, `stave-schema`, `stave-modules`,
  `stave-model`, `stave-solve`, `stave-geom`, `stave-check`,
  `stave-workplan` and `stave-scene` never name a joint or hardware kind
  (`euro_hinge`, `dado`), a member of one (`cup`, `leaf`, `arm`) or a field
  only one kind has (`cup_inset`, `plate_height`), in code or in a string
  (a help line's example included). Behaviour comes from the language: a
  behaviour a kind needs that the language cannot yet say is a new language
  primitive, with a spec entry in `docs/grammar.md` first, then code that
  works for every kind. The bases `hinge`, `slide` and `pull` may choose
  words in messages; they may not choose behaviour. No check may let a
  particular pairing off: room is made by a cut or not at all.
  `crates/stave-check/tests/standing_rule.rs` reads the names out of
  `std/` and fails, naming file and line, if a core crate uses one; its
  allow-list holds only the language's own vocabulary and plain English,
  each entry with its reason, and is not grown to make a change pass.

## Working in this repo

- Commands: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`. When several agents build in parallel, each sets its own
  `CARGO_TARGET_DIR=target/<lane>` to avoid lock contention.
- z3 is built from source by the `z3` crate's static-link feature (no system
  libz3 is installed). The first build takes several minutes.
- Commit when a unit of work is complete and green, staging only the paths you
  own (`git add <paths>`, never `git add -A` while others work). If
  `.git/index.lock` exists, wait and retry. Never rewrite history.
- User-facing text (diagnostics, PDF, UI) is plain, specific English and uses
  woodworking words a beginner can look up.
- Words the window shows (`stave-gtk`, and what `stave-app` words for it)
  are marked for translation with `stave_app::i18n`: `tr("…")`, `ntr` for
  a count (never an `if` on the count), `fill_in` for values (whole
  sentences with named holes, never pieces glued together), `tr_noop` for
  a table translated where it is shown. Text that is matched on stays
  untranslated. Run `po/update-pot` after adding a file that marks words.
  The core crates' diagnostics stay English and unmarked.
