# Stave for VS Code

Support for `.stave` files, the Stave woodworking language: colouring,
diagnostics, completion, signature help, hover, go to definition,
references, rename, symbols, folding, formatting and quick fixes. Everything
but the first colouring comes from the language server, `stave-lsp`.

## Install

1. Install the server from the Stave repository and make sure it is on
   `PATH`:

   ```sh
   cargo install --path crates/stave-lsp
   ```

2. Build and install the extension:

   ```sh
   cd editors/vscode
   npm ci
   npm run package          # makes stave-0.1.0.vsix
   code --install-extension stave-0.1.0.vsix
   ```

   Or open `editors/vscode` in VS Code, run `npm ci && npm run compile`,
   and press F5 to start an Extension Development Host.

## Settings

| Setting | Default | |
|---|---|---|
| `stave.server.path` | `stave-lsp` | The server executable: a name on `PATH` or an absolute path. It is started with no arguments over stdio. |
| `stave.trace.server` | `off` | Log the protocol messages to the Stave output channel. |

The command **Stave: Restart Language Server** restarts it.

## Colours

Files are coloured by the TextMate grammar (`editors/textmate`) as soon as
they open, then by the server's semantic tokens. The extension declares the
custom token types (`unit`, `docComment`, and one per item kind: `part`,
`joint`, `param`, `region`, `hardware`, `assembly`, `scene`, `stock`,
`profile`, `recipe`, `let`, `require`, `role`, `label`, `member`) and
modifiers (`reference`, `unresolved`, `solvedParam`), maps each to a
TextMate scope for themes without semantic rules, and sets these defaults:
units in italics, references underlined, solved params bold, standard
library names in italics.

Change them in your settings, for example:

```json
"editor.semanticTokenColorCustomizations": {
  "rules": {
    "part:stave": "#c08040",
    "joint:stave": { "foreground": "#4090c0", "bold": true },
    "*.reference:stave": { "underline": false }
  }
}
```

The legend and what each entry means are in `editors/README.md`.

## Standard library

Go to definition on a standard library name (`dado`, `FramePanelDoor`)
opens a read-only `stave-std:///…` document, whose text the extension asks
the server for (`stave/standardLibraryText`).
