//! The standard library, embedded at compile time by `build.rs` from the
//! repository's `std/<module>/` folders.

include!(concat!(env!("OUT_DIR"), "/std_files.rs"));

/// The standard modules: the [`PRELUDE_MODULES`] in prelude order, then the
/// ones a project imports to use. `std:<name>` imports one of them.
pub const STD_MODULES: [&str; 4] = ["joinery", "hardware", "tools", "casework"];

/// The standard modules whose public items every file sees without an
/// import, in prelude order: the first entries of [`STD_MODULES`]. The
/// others (`std:casework`) are loaded only when a file imports them.
pub const PRELUDE_MODULES: [&str; 3] = ["joinery", "hardware", "tools"];

/// Every embedded standard-library file as (module, file name, text):
/// modules in [`STD_MODULES`] order, files sorted by name.
pub fn std_files() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    STD_FILES.iter().copied()
}

/// The files of one standard module, sorted by name, as (file name, text).
pub(crate) fn module_files(module: &str) -> impl Iterator<Item = (&'static str, &'static str)> {
    std_files()
        .filter(move |(name, _, _)| *name == module)
        .map(|(_, file, text)| (file, text))
}

/// The standard module called `name`, as its `'static` name.
pub(crate) fn std_module(name: &str) -> Option<&'static str> {
    STD_MODULES.into_iter().find(|module| *module == name)
}
