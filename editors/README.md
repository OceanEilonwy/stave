# Editors

Stave in editors other than the GTK app. Everything an editor shows beyond
basic colouring comes from the language server, `stave-lsp`
(`crates/stave-lsp`), which every editor here starts the same way: the
executable `stave-lsp`, **no arguments**, speaking the language server
protocol over **standard input and output**. Install it with

```sh
cargo install --path crates/stave-lsp
```

| Folder | What | Colouring |
|---|---|---|
| [`textmate/`](textmate/) | `stave.tmLanguage.json`, a TextMate grammar, and its tests | before the server starts, on GitHub, in any TextMate host |
| [`vscode/`](vscode/) | VS Code extension: starts the server, maps the custom token types and modifiers to scopes and styles | TextMate grammar, then semantic tokens |
| [`helix/`](helix/) | `languages.toml` snippet | none yet (Helix colours with tree-sitter) |
| [`neovim/`](neovim/) | `stave.lua`: filetype, `vim.lsp.config` / nvim-lspconfig, highlight links | semantic tokens |
| [`zed/`](zed/) | Zed extension: language and server | none yet (Zed colours with tree-sitter) |

## The contract with `stave-lsp`

This section binds the server and the editor configurations here. Change
both together. The machine-readable copy of the legend is
[`semantic-tokens.json`](semantic-tokens.json);
`vscode/scripts/check-manifest.mjs` checks the VS Code extension against it,
and a `stave-lsp` unit test (`legend::tests`) checks the server's legend
against it.

### Starting

- Command `stave-lsp`, no arguments, stdio. No `--stdio` flag is passed
  (VS Code's client adds one only when a transport is named; ours names
  none).
- Documents have language id `stave` and the extension `.stave`.

### Semantic token legend

The server sends this legend in its `initialize` result
(`semanticTokensProvider.legend`), in this order. Types not in the protocol's
predefined list are custom; editors that do not know them ignore them and
keep their TextMate or tree-sitter colours.

| # | Type | Custom | Sent for (`stave_ide::TokenClass`) |
|---|---|---|---|
| 0 | `keyword` | | `Keyword`: keywords and contextual words in their position |
| 1 | `comment` | | `Comment`: `// …`, `/* … */`; `DocComment` (`/// …`) with the `documentation` modifier |
| 2 | `string` | | `Text`: a string's quotes and text |
| 3 | `number` | | `Number`: a number, or the number of a quantity (`18` of `18mm`, `3/4` of `3/4in`, `1-1/2` of `1-1/2in`) |
| 4 | `operator` | | `Punctuation`: brackets, operators, separators |
| 5 | `function` | | `Function`: a call head fixed by position or a built-in function (`flush`, `tenon`, `overlay`) |
| 6 | `type` | | `Type`: type names (`Region`) |
| 7 | `namespace` | | `Module`, `Item(Module)`: a module's name (`casework`, `hw`) |
| 8 | `enumMember` | | `EnumWord`: a word of a closed list (`up`, `across`, `open`, `plywood`) |
| 9 | `property` | | `Field`: a field name, a named argument, a member after `.` |
| 10 | `variable` | | `Plain` **with** a modifier (`unresolved`): a name that resolves to nothing |
| 11 | `unit` | yes | `Unit`: the unit of a quantity (`mm`, `in`, `deg`), always its own token right after its number |
| 12 | `component` | yes | `Item(Component)` |
| 13 | `kind` | yes | `Item(Kind)` |
| 14 | `recipe` | yes | `Item(Recipe)` |
| 15 | `stock` | yes | `Item(Stock)` |
| 16 | `profile` | yes | `Item(Profile)` |
| 17 | `scene` | yes | `Item(Scene)` |
| 18 | `param` | yes | `Item(Param)` |
| 19 | `let` | yes | `Item(Let)` |
| 20 | `region` | yes | `Item(Region)` |
| 21 | `part` | yes | `Item(Part)` |
| 22 | `joint` | yes | `Item(Joint)` |
| 23 | `hardware` | yes | `Item(Hardware)` |
| 24 | `assembly` | yes | `Item(Assembly)` |
| 25 | `require` | yes | `Item(Require)` |
| 26 | `role` | yes | `Item(Role)`: a kind's role (`piece: Part`) |
| 27 | `madePart` | yes | `Item(MadePart)`: a part a kind makes |
| 28 | `member` | yes | `Item(Member)`: a hardware kind's member (`member cup`) |
| 29 | `label` | yes | `Item(Label)`: an array label (`bottom` in `rail[bottom, top]`) |

`Error` and `Plain` without a modifier are not sent: the editor's own
colouring stays.

| Bit | Modifier | Custom | Sent for (`stave_ide::TokenModifier`) |
|---|---|---|---|
| 0 (1) | `declaration` | | `Declaration`: the name an item is declared with |
| 1 (2) | `reference` | yes | `Reference`: a name meaning an item declared elsewhere |
| 2 (4) | `defaultLibrary` | | `StandardLibrary`: an item from the standard library |
| 3 (8) | `unresolved` | yes | `Unresolved`: a word where a name goes that names nothing |
| 4 (16) | `solvedParam` | yes | `SolvedParam`: a `solved param`, declared or named |
| 5 (32) | `documentation` | | a doc comment (`TokenClass::DocComment`), on type `comment` |

The order is `TokenModifier::ALL`'s, so bit `n` of the protocol's
modifier set is `TokenModifier::ALL[n]`: the server builds the number by
folding `TokenModifiers::iter()` over that order (the set's own bits are
private to `stave-ide`).

Positions are UTF-16 code units, as the protocol requires; the server
converts from byte offsets at the edge (`docs/grammar.md`, "Positions").

### Standard library documents

Go to definition on `dado` or `FramePanelDoor` answers with a
`stave-std:///joinery/dado.stave` URI. An editor that cannot read that
scheme asks the server for the text with the custom request

```
method: "stave/standardLibraryText"
params: { "uri": "stave-std:///joinery/dado.stave" }
result: string | null      (null: no such file)
```

The VS Code extension registers a read-only document provider for the
`stave-std` scheme that sends it. Once such a document is open the client
sends the usual `textDocument/didOpen` for it, with its `stave-std:///`
URI, and asks for its semantic tokens, hover and definitions like any
other document; the server answers them and never accepts edits to it.

### How colours look

The VS Code extension sets these defaults for `.stave` files; the Neovim
file sets the same with highlight groups (set again after each colour
scheme change). They match the GTK editor:

- `unit`: italic;
- `*.reference`: underlined;
- `*.solvedParam`: bold;
- `*.defaultLibrary`: italic.

For themes without semantic token rules, each custom type falls back to a
TextMate scope (`vscode/package.json`, `semanticTokenScopes`): parts and
assemblies colour like classes, joints like interfaces, hardware, stock and
profiles like structs and types, params and roles like parameters, and so
on.

## The TextMate grammar

`textmate/stave.tmLanguage.json` is derived from the lexer
(`crates/stave-lang/src/lexer.rs`), the keyword and contextual word lists
(`crates/stave-lang/src/lib.rs`), the units (`stave_lang::ast::Unit`) and the
closed word lists (`crates/stave-schema/src/words.rs`). It scopes:

- comments `comment.line.double-slash`, `comment.block`, doc comments
  `comment.line.documentation`;
- strings `string.quoted.double`, escapes `constant.character.escape`,
  interpolations `{name.member}` as `meta.interpolation` with
  `variable.other.interpolated`;
- a quantity as two tokens: `constant.numeric` for `18`, `3/4` or
  `1-1/2`, then `keyword.other.unit` for `mm`; an unknown suffix is
  `invalid.illegal.unit`;
- item keywords `storage.type.<keyword>` and the declared name
  `entity.name.<keyword>` (`entity.name.type.component`,
  `entity.name.type.kind`, `entity.name.function.recipe`);
  `private`, `locked`, `solved` `storage.modifier`; `true`, `false`
  `constant.language.boolean`; the other keywords `keyword.other`;
- enum words `constant.language.enum`, `units: mm` as
  `constant.language.enum.unit`;
- fields and named arguments `variable.other.property`, members after `.`
  `variable.other.member`, call heads `entity.name.function.call`,
  capitalised names `entity.name.type`.

TextMate cannot tell a part from a joint or a param; the semantic tokens do.

### Testing

```sh
cd editors/textmate
npm ci
npm test
```

runs two checks:

- `vscode-tmgrammar-test` on `test/scopes.test.stave`: one scope assertion
  per rule;
- `test/check.mjs`: compiles every expression with Oniguruma, checks the
  grammar's keyword, unit, contextual and enum word lists against the Rust
  sources, and tokenises every `.stave` file in `examples/` and `std/`,
  comparing it token by token with a port of the lexer's first pass
  (comments, doc comments, strings, interpolations, numbers, each
  quantity's number and unit, keywords) and failing on any `invalid.*`
  scope.

`cd editors/vscode && npm ci && npm run check` type-checks the extension and
checks its manifest against `semantic-tokens.json`.
