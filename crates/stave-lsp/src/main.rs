//! `stave-lsp`: Stave's language server over standard input and output.
//! It takes no arguments; an editor starts it for `.stave` files.

#![forbid(unsafe_code)]

fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).is_some_and(|a| a == "--version") {
        println!("stave-lsp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    stave_lsp::serve_stdio(stave_lsp::Config::default())?;
    Ok(())
}
