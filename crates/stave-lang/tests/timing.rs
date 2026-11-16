//! Parsing is fast enough to run on every keystroke: the cabinet example's
//! files parse in well under a millisecond each (in a release build; the
//! bound below is generous so that debug builds and busy machines pass).

use std::time::{Duration, Instant};

use stave_lang::{SourceMap, parse_file};

#[test]
fn the_cabinet_parses_in_well_under_a_millisecond_per_file() {
    let folder =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
    let mut texts = Vec::new();
    for name in ["cabinet", "carcass", "doors", "drawers", "stock"] {
        texts.push(std::fs::read_to_string(folder.join(format!("{name}.stave"))).unwrap());
    }
    let mut sources = SourceMap::new();
    let file = sources.add("cabinet", "").unwrap();
    // Warm up, then time.
    for text in &texts {
        assert!(parse_file(file, text).diagnostics.is_empty());
    }
    let rounds = 50;
    let start = Instant::now();
    for _ in 0..rounds {
        for text in &texts {
            std::hint::black_box(parse_file(file, std::hint::black_box(text)));
        }
    }
    let per_file = start.elapsed() / (rounds * u32::try_from(texts.len()).unwrap());
    eprintln!("average parse time per cabinet file: {per_file:?}");
    let bound = if cfg!(debug_assertions) {
        Duration::from_millis(5)
    } else {
        Duration::from_micros(500)
    };
    assert!(per_file < bound, "{per_file:?} per file is too slow");
}
