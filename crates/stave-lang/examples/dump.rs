//! Prints the items (and optionally the syntax tree) of a `.stave` file.
fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file> [tree]");
    let text = std::fs::read_to_string(&path).expect("readable file");
    let mut sources = stave_lang::SourceMap::new();
    let file = sources.add(&path, text.as_str()).expect("file fits");
    if stave_lang::is_patch_source(&text) {
        let parse = stave_lang::parse_patch_file(file, &text);
        println!("{:#?}", parse.ast);
        print!(
            "{}",
            stave_lang::render_diagnostics(&sources, &parse.diagnostics)
        );
        return;
    }
    let parse = stave_lang::parse_file(file, &text);
    if std::env::args().nth(2).as_deref() == Some("tree") {
        println!("{:#?}", parse.syntax());
    }
    if let Some(ast) = &parse.ast {
        for item in &ast.items {
            println!(
                "{} {:?}",
                item.kind.keyword(),
                item.kind.name().map(|n| n.as_str())
            );
        }
    }
    print!(
        "{}",
        stave_lang::render_diagnostics(&sources, &parse.diagnostics)
    );
}
