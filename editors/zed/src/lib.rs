//! The Stave extension for Zed: starts `stave-lsp`, found on the worktree's
//! `PATH`, with no arguments, speaking the language server protocol over
//! standard input and output. Everything else comes from the server; the
//! contract is `editors/README.md`.

use zed_extension_api::{self as zed, LanguageServerId, Result};

/// The name of the language server's executable.
const SERVER_EXECUTABLE: &str = "stave-lsp";

/// The extension. It keeps no state: the server is looked up on each start.
struct StaveExtension;

impl zed::Extension for StaveExtension {
    fn new() -> Self {
        StaveExtension
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let command = worktree.which(SERVER_EXECUTABLE).ok_or_else(|| {
            format!(
                "{SERVER_EXECUTABLE} is not on PATH: install it with \
                 `cargo install --path crates/stave-lsp` in the Stave repository"
            )
        })?;
        Ok(zed::Command {
            command,
            args: Vec::new(),
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(StaveExtension);
