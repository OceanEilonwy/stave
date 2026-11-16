//! Loading modules from memory: every rule of the module stage.

use std::path::{Path, PathBuf};

use stave_lang::{Code, Diagnostic, render_diagnostics};
use stave_modules::{
    LoadError, MemoryFiles, ModuleHeader, ModuleId, ModuleSet, ModuleSource, STD_MODULES, std_files,
};

const PROJECT: &str = "stave 1\n\nproject \"Shop\" {\n  units: mm\n}\n";

fn library(title: &str) -> String {
    format!("stave 1\n\nlibrary \"{title}\"\n\nlet width = 20mm\n")
}

fn project_with_imports(imports: &str) -> String {
    format!("stave 1\n\n{imports}\n\nproject \"Shop\" {{\n  units: mm\n}}\n")
}

fn load(files: &MemoryFiles) -> ModuleSet {
    ModuleSet::load_with(Path::new("shop"), files).unwrap()
}

fn render(set: &ModuleSet) -> String {
    render_diagnostics(&set.sources, &set.diagnostics)
}

fn with_code(set: &ModuleSet, code: Code) -> Vec<&Diagnostic> {
    set.diagnostics.iter().filter(|d| d.code == code).collect()
}

fn only(set: &ModuleSet, code: Code) -> &Diagnostic {
    assert_eq!(set.diagnostics.len(), 1, "{}", render(set));
    let diagnostic = &set.diagnostics[0];
    assert_eq!(diagnostic.code, code, "{}", render(set));
    diagnostic
}

fn root_imports(set: &ModuleSet) -> Vec<(String, ModuleId)> {
    set.module(set.root)
        .unwrap()
        .files
        .iter()
        .flat_map(|file| &file.imports)
        .map(|import| (import.name.clone(), import.module))
        .collect()
}

fn named(set: &ModuleSet, name: &str) -> ModuleId {
    set.modules
        .iter()
        .find(|module| module.name == name && matches!(module.source, ModuleSource::Local { .. }))
        .unwrap()
        .id
}

#[test]
fn root_project_with_two_files() {
    let files = MemoryFiles::new()
        .with("shop/stock.stave", "stave 1\n\nlet width = 20mm\n")
        .with("shop/cabinet.stave", PROJECT);
    let set = load(&files);
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
    let root = set.module(set.root).unwrap();
    assert_eq!(set.root.index(), 0);
    assert_eq!(root.name, "shop");
    let paths: Vec<&str> = root.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["shop/cabinet.stave", "shop/stock.stave"]);
    assert_eq!(
        root.header,
        Some(ModuleHeader::Project {
            file: root.files[0].file,
            item: 0
        })
    );
    // Root files get the first file ids, in file-name order.
    assert_eq!(root.files[0].file.index(), 0);
    assert_eq!(root.files[1].file.index(), 1);
    assert_eq!(set.sources.files().count(), 2 + std_files().count());
}

#[test]
fn root_is_module_zero_and_std_follows() {
    let files = MemoryFiles::new().with("shop/cabinet.stave", PROJECT);
    let set = load(&files);
    assert_eq!(set.root.index(), 0);
    assert_eq!(set.modules.len(), 4);
    for (index, module) in set.modules.iter().enumerate() {
        assert_eq!(module.id.index(), index);
    }
}

#[test]
fn std_is_always_loaded_in_prelude_order() {
    let files = MemoryFiles::new().with("shop/cabinet.stave", PROJECT);
    let set = load(&files);
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
    let names: Vec<(&str, bool)> = set
        .prelude
        .iter()
        .map(|id| {
            let module = set.module(*id).unwrap();
            (
                module.name.as_str(),
                matches!(module.header, Some(ModuleHeader::Library { .. })),
            )
        })
        .collect();
    assert_eq!(
        names,
        [("joinery", true), ("hardware", true), ("tools", true)]
    );
    for (id, expected) in set.prelude.iter().zip(STD_MODULES) {
        let module = set.module(*id).unwrap();
        assert_eq!(module.source, ModuleSource::Std { name: expected });
        assert!(
            module
                .files
                .iter()
                .all(|file| file.path.starts_with(&format!("std/{expected}/")))
        );
    }
    let joinery = set.module(set.prelude[0]).unwrap();
    assert!(
        joinery
            .files
            .iter()
            .any(|file| file.path == "std/joinery/dado.stave")
    );
}

#[test]
fn import_with_alias() {
    let files = MemoryFiles::new()
        .with(
            "shop/cabinet.stave",
            project_with_imports("import \"./casework\"\nimport \"./hardware\" as hw"),
        )
        .with("shop/casework/library.stave", library("Casework"))
        .with("shop/casework/drawer.stave", "stave 1\n\nlet depth = 5mm\n")
        .with("shop/hardware/library.stave", library("Hardware"));
    let set = load(&files);
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
    let casework = named(&set, "casework");
    let hardware = named(&set, "hardware");
    assert_eq!(
        root_imports(&set),
        [
            (String::from("casework"), casework),
            (String::from("hw"), hardware)
        ]
    );
    // Imported modules come right after the root, in import order.
    assert_eq!(casework.index(), 1);
    assert_eq!(hardware.index(), 2);
    let casework = set.module(casework).unwrap();
    let paths: Vec<&str> = casework.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        ["shop/casework/drawer.stave", "shop/casework/library.stave"]
    );
    assert!(matches!(
        casework.header,
        Some(ModuleHeader::Library { .. })
    ));
    let import = &set.module(set.root).unwrap().files[0].imports[1];
    let text = set
        .sources
        .get(set.module(set.root).unwrap().files[0].file)
        .unwrap();
    let span = import.name_span;
    assert_eq!(
        &text.text()[span.start() as usize..span.end() as usize],
        "hw"
    );
}

#[test]
fn same_folder_from_two_files_is_loaded_once() {
    let files = MemoryFiles::new()
        .with(
            "shop/a.stave",
            project_with_imports("import \"./casework\""),
        )
        .with("shop/b.stave", "stave 1\n\nimport \"./casework/\" as cw\n")
        .with("shop/casework/library.stave", library("Casework"));
    let set = load(&files);
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
    assert_eq!(set.modules.len(), 5);
    let casework = named(&set, "casework");
    assert_eq!(
        root_imports(&set),
        [
            (String::from("casework"), casework),
            (String::from("cw"), casework)
        ]
    );
}

#[test]
fn keyword_folder_without_alias_is_reserved_word() {
    let files = MemoryFiles::new()
        .with(
            "shop/cabinet.stave",
            project_with_imports("import \"./hardware\""),
        )
        .with("shop/hardware/library.stave", library("Hardware"));
    let set = load(&files);
    let diagnostic = only(&set, Code::ReservedWord);
    assert_eq!(
        diagnostic.message,
        "'hardware' is a reserved word and can't be used as a name"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("give the module another name: import \"./hardware\" as hw")
    );
    // The module is still loaded, but the import has no name.
    named(&set, "hardware");
    assert!(root_imports(&set).is_empty());
}

#[test]
fn folder_that_is_not_a_word_is_reserved_word() {
    let files = MemoryFiles::new()
        .with(
            "shop/cabinet.stave",
            project_with_imports("import \"./my-shop\""),
        )
        .with("shop/my-shop/library.stave", library("Mine"));
    let set = load(&files);
    let diagnostic = only(&set, Code::ReservedWord);
    assert_eq!(diagnostic.message, "'my-shop' can't be used as a name");
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("give the module another name: import \"./my-shop\" as my_shop")
    );
    assert!(root_imports(&set).is_empty());
}

#[test]
fn remote_source_is_not_supported() {
    let files = MemoryFiles::new().with(
        "shop/cabinet.stave",
        project_with_imports(
            "import \"github:some-maker/catalog/slides@v2.1.0\" as slides\n\
             import \"https://example.com/stock#sha256=9f2c\" as lumber",
        ),
    );
    let set = load(&files);
    let diagnostics = with_code(&set, Code::NotSupported);
    assert_eq!(set.diagnostics.len(), 2, "{}", render(&set));
    assert_eq!(
        diagnostics[0].message,
        "importing from 'github:some-maker/catalog/slides@v2.1.0' is not supported yet"
    );
    assert_eq!(
        diagnostics[0].help.as_deref(),
        Some("only folders on this computer and std: modules can be imported in this version")
    );
    assert_eq!(
        diagnostics[1].message,
        "importing from 'https://example.com/stock#sha256=9f2c' is not supported yet"
    );
    assert!(root_imports(&set).is_empty());
    assert_eq!(set.modules.len(), 4);
}

#[test]
fn missing_folder_is_unknown_module() {
    let files = MemoryFiles::new()
        .with(
            "shop/cabinet.stave",
            project_with_imports("import \"./nowhere\"\nimport \"./empty\""),
        )
        .with("shop/empty/notes.txt", "not stave");
    let set = load(&files);
    let diagnostics = with_code(&set, Code::UnknownModule);
    assert_eq!(diagnostics.len(), 2, "{}", render(&set));
    assert_eq!(diagnostics[0].message, "can't find module './nowhere'");
    assert_eq!(
        diagnostics[0].help.as_deref(),
        Some("there is no folder at 'shop/nowhere'")
    );
    assert_eq!(diagnostics[1].message, "can't find module './empty'");
    assert_eq!(
        diagnostics[1].help.as_deref(),
        Some("there are no .stave files in 'shop/empty'")
    );
}

#[test]
fn cycle_between_two_modules() {
    let files = MemoryFiles::new()
        .with("a/project.stave", project_with_imports("import \"../b\""))
        .with(
            "b/library.stave",
            "stave 1\n\nimport \"../a\"\n\nlibrary \"B\"\n",
        );
    let set = ModuleSet::load_with(Path::new("a"), &files).unwrap();
    let diagnostic = only(&set, Code::ImportCycle);
    assert_eq!(diagnostic.message, "modules import each other: b -> a -> b");
    let b = named(&set, "b");
    let b_file = set.module(b).unwrap().files[0].file;
    assert_eq!(diagnostic.primary().unwrap().file, b_file);
    assert_eq!(set.modules.len(), 5);
}

#[test]
fn self_import_is_a_cycle() {
    let files = MemoryFiles::new().with(
        "shop/cabinet.stave",
        project_with_imports("import \"../shop\" as me"),
    );
    let set = load(&files);
    let diagnostic = only(&set, Code::ImportCycle);
    assert_eq!(
        diagnostic.message,
        "modules import each other: shop -> shop"
    );
    assert_eq!(set.modules.len(), 4);
}

#[test]
fn duplicate_import_names_in_one_file() {
    let files = MemoryFiles::new()
        .with(
            "shop/cabinet.stave",
            project_with_imports("import \"./casework\" as parts\nimport \"./doors\" as parts"),
        )
        .with("shop/casework/library.stave", library("Casework"))
        .with("shop/doors/library.stave", library("Doors"));
    let set = load(&files);
    let diagnostic = only(&set, Code::DuplicateName);
    assert_eq!(diagnostic.message, "'parts' is declared twice");
    assert_eq!(diagnostic.labels.len(), 2);
    assert!(diagnostic.labels[1].span.start() < diagnostic.labels[0].span.start());
    assert_eq!(root_imports(&set).len(), 1);
}

#[test]
fn same_name_in_two_files_is_fine() {
    let files = MemoryFiles::new()
        .with(
            "shop/a.stave",
            project_with_imports("import \"./casework\" as parts"),
        )
        .with(
            "shop/b.stave",
            "stave 1\n\nimport \"./casework\" as parts\n",
        )
        .with("shop/casework/library.stave", library("Casework"));
    let set = load(&files);
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
}

#[test]
fn import_name_clashing_with_an_item() {
    let files = MemoryFiles::new()
        .with(
            "shop/cabinet.stave",
            project_with_imports("import \"./casework\""),
        )
        .with("shop/stock.stave", "stave 1\n\nlet casework = 20mm\n")
        .with("shop/casework/library.stave", library("Casework"));
    let set = load(&files);
    let diagnostic = only(&set, Code::DuplicateName);
    assert_eq!(diagnostic.message, "'casework' is declared twice");
    let root = set.module(set.root).unwrap();
    assert_eq!(diagnostic.labels[0].file, root.files[0].file);
    assert_eq!(diagnostic.labels[1].file, root.files[1].file);
    assert!(root_imports(&set).is_empty());
}

#[test]
fn module_without_project_or_library() {
    let files = MemoryFiles::new().with("shop/stock.stave", "stave 1\n\nlet width = 20mm\n");
    let set = load(&files);
    let diagnostic = only(&set, Code::MissingField);
    assert_eq!(
        diagnostic.message,
        "module 'shop' is missing 'project' or 'library'"
    );
    assert_eq!(set.module(set.root).unwrap().header, None);
}

#[test]
fn module_with_two_headers() {
    let files = MemoryFiles::new()
        .with("shop/a.stave", PROJECT)
        .with("shop/b.stave", "stave 1\n\nlibrary \"Also\"\n");
    let set = load(&files);
    let diagnostic = only(&set, Code::DuplicateName);
    assert_eq!(diagnostic.message, "'library' is declared twice");
    let root = set.module(set.root).unwrap();
    assert_eq!(diagnostic.labels[0].file, root.files[1].file);
    assert_eq!(diagnostic.labels[1].file, root.files[0].file);
    assert_eq!(root.header, None);
}

#[test]
fn explicit_std_import_with_alias() {
    let files = MemoryFiles::new().with(
        "shop/cabinet.stave",
        project_with_imports("import \"std:joinery\" as j\nimport \"std:tools\""),
    );
    let set = load(&files);
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
    assert_eq!(
        root_imports(&set),
        [
            (String::from("j"), set.prelude[0]),
            (String::from("tools"), set.prelude[2])
        ]
    );
    assert_eq!(set.modules.len(), 4);
    // An imported std module is numbered where it is first reached.
    assert_eq!(set.prelude[0].index(), 1);
    assert_eq!(set.prelude[2].index(), 2);
    assert_eq!(set.prelude[1].index(), 3);
}

#[test]
fn std_hardware_without_alias_is_reserved_word() {
    let files = MemoryFiles::new().with(
        "shop/cabinet.stave",
        project_with_imports("import \"std:hardware\""),
    );
    let set = load(&files);
    let diagnostic = only(&set, Code::ReservedWord);
    assert_eq!(
        diagnostic.message,
        "'hardware' is a reserved word and can't be used as a name"
    );
}

#[test]
fn unknown_std_module() {
    let files = MemoryFiles::new().with(
        "shop/cabinet.stave",
        project_with_imports("import \"std:clamps\""),
    );
    let set = load(&files);
    let diagnostic = only(&set, Code::UnknownModule);
    assert_eq!(diagnostic.message, "can't find module 'std:clamps'");
}

#[test]
fn root_identical_to_std_module_is_that_module() {
    let mut files = MemoryFiles::new();
    for (module, name, text) in std_files() {
        if module == "joinery" {
            files.insert(Path::new("std/joinery").join(name), text);
        }
    }
    let set = ModuleSet::load_with(Path::new("std/joinery"), &files).unwrap();
    assert!(set.diagnostics.is_empty(), "{}", render(&set));
    assert_eq!(set.modules.len(), 3);
    assert_eq!(set.prelude[0], set.root);
    let root = set.module(set.root).unwrap();
    assert_eq!(root.source, ModuleSource::Std { name: "joinery" });
    assert_eq!(root.name, "joinery");
}

#[test]
fn edited_std_folder_is_a_local_module() {
    let mut files = MemoryFiles::new();
    for (module, name, text) in std_files() {
        if module == "tools" {
            files.insert(Path::new("mine").join(name), format!("{text}\n"));
        }
    }
    let set = ModuleSet::load_with(Path::new("mine"), &files).unwrap();
    assert_eq!(set.modules.len(), 4);
    assert!(matches!(
        set.module(set.root).unwrap().source,
        ModuleSource::Local { .. }
    ));
}

#[test]
fn patch_file_is_not_part_of_the_module() {
    let files = MemoryFiles::new().with("shop/cabinet.stave", PROJECT).with(
        "shop/widen.stave",
        "stave 1\n\npatch \"Widen\" {\n  set width = 30mm\n}\n",
    );
    let set = load(&files);
    let root = set.module(set.root).unwrap();
    assert_eq!(root.files.len(), 1);
    assert_eq!(root.files[0].path, "shop/cabinet.stave");
    // The patch file is still in the source map, with its own id.
    let paths: Vec<&str> = set.sources.files().map(|(_, file)| file.path()).collect();
    assert_eq!(&paths[..2], ["shop/cabinet.stave", "shop/widen.stave"]);
    assert!(
        set.diagnostics
            .iter()
            .all(|d| !matches!(d.code, Code::MissingField | Code::DuplicateName))
    );
}

#[test]
fn parse_errors_are_kept_with_their_file() {
    let files = MemoryFiles::new()
        .with("shop/cabinet.stave", PROJECT)
        .with("shop/stock.stave", "stave 1\n\nlet width =\n");
    let set = load(&files);
    assert!(!set.diagnostics.is_empty());
    let stock = set.module(set.root).unwrap().files[1].file;
    assert!(
        set.diagnostics
            .iter()
            .all(|d| d.primary().unwrap().file == stock)
    );
}

#[test]
fn root_errors() {
    let files = MemoryFiles::new()
        .with("shop/notes.txt", "hi")
        .with("file.stave", PROJECT);
    assert!(matches!(
        ModuleSet::load_with(Path::new("shop"), &files),
        Err(LoadError::NoStaveFiles { .. })
    ));
    assert!(matches!(
        ModuleSet::load_with(Path::new("elsewhere"), &files),
        Err(LoadError::Unreadable { .. })
    ));
    assert!(matches!(
        ModuleSet::load_with(Path::new("file.stave"), &files),
        Err(LoadError::NotAFolder { .. })
    ));
}

#[test]
fn diagnostics_are_sorted_and_deterministic() {
    let files: MemoryFiles = [
        (
            PathBuf::from("shop/b.stave"),
            project_with_imports("import \"./x\"\nimport \"github:a/b\" as b"),
        ),
        (
            PathBuf::from("shop/a.stave"),
            String::from("stave 1\n\nimport \"./hardware\"\n\nlibrary \"Two\"\n"),
        ),
        (PathBuf::from("shop/hardware/library.stave"), library("H")),
    ]
    .into_iter()
    .collect();
    let first = load(&files);
    let second = load(&files);
    assert_eq!(render(&first), render(&second));
    let keys: Vec<(u32, u32)> = first
        .diagnostics
        .iter()
        .map(|d| {
            let label = d.primary().unwrap();
            (label.file.index(), label.span.start())
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert_eq!(first.diagnostics.len(), 4, "{}", render(&first));
}
