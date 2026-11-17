//! Times loading and indexing a project folder.
fn main() {
    let Some(folder) = std::env::args().nth(1) else {
        return;
    };
    let Ok(folder) = std::path::Path::new(&folder).canonicalize() else {
        return;
    };
    for _ in 0..3 {
        let started = std::time::Instant::now();
        let set = stave_modules::ModuleSet::load(&folder);
        let loaded = started.elapsed();
        if let Ok(set) = set {
            let analysis = stave_ide::Analysis::from_modules(&folder, set);
            let indexed = started.elapsed();
            let files = analysis.project_files();
            let mut tokens = 0;
            for (file, _) in &files {
                tokens += analysis.tokens(*file).len();
            }
            println!(
                "load {loaded:?}, index {:?}, tokens {:?} ({tokens})",
                indexed - loaded,
                started.elapsed() - indexed
            );
        }
    }
}
