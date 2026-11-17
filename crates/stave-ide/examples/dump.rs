//! Prints a file's concrete syntax tree, without whitespace.
fn main() {
    let Some(path) = std::env::args().nth(1) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut sources = stave_lang::SourceMap::new();
    let Ok(file) = sources.add(path.clone(), text.clone()) else {
        return;
    };
    let parse = stave_lang::parse_file(file, &text);
    print!("{:#?}", parse.syntax());
}
