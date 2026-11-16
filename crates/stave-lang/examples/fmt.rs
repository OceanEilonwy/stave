//! Prints a `.stave` file as `stave fmt` would write it.
fn main() {
    let path = std::env::args().nth(1).expect("usage: fmt <file>");
    let text = std::fs::read_to_string(&path).expect("readable file");
    let mut sources = stave_lang::SourceMap::new();
    let file = sources.add(&path, text.as_str()).expect("file fits");
    match stave_lang::format_file(file, &text) {
        Ok(formatted) => print!("{formatted}"),
        Err(error) => eprintln!("{error}"),
    }
}
