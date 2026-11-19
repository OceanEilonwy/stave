//! Solving and checking a project written in the test, end to end.

#![allow(dead_code)]

use std::path::Path;

use stave_lang::Diagnostic;
use stave_lang::units::Length;
use stave_model::solved::{Part, SolvedDesign, WorldBox};
use stave_modules::{MemoryFiles, ModuleSet};

/// What a project gives: every diagnostic of every stage and the checks,
/// and the solved design when it solved.
pub struct Checked {
    pub diagnostics: Vec<Diagnostic>,
    pub design: Option<SolvedDesign>,
}

impl Checked {
    /// The design, which must have solved and checked clean.
    pub fn clean(&self) -> &SolvedDesign {
        let messages: Vec<String> = self
            .diagnostics
            .iter()
            .map(|d| format!("{}[{}] {}", d.severity, d.code, d.message))
            .collect();
        assert!(messages.is_empty(), "{messages:#?}");
        self.design.as_ref().expect("the design solved")
    }

    /// The messages of the diagnostics, in order.
    pub fn messages(&self) -> Vec<String> {
        self.diagnostics.iter().map(|d| d.message.clone()).collect()
    }
}

/// Resolves, solves and checks `project.stave` with this text.
pub fn check(text: &str) -> Checked {
    let folder = Path::new("/memory/project");
    let files = MemoryFiles::new().with(folder.join("project.stave"), text);
    let modules = ModuleSet::load_with(folder, &files).unwrap();
    let mut diagnostics = modules.diagnostics.clone();
    let (defs, found) = stave_model::resolve(&modules);
    diagnostics.extend(found);
    let (model, found) = stave_model::instance::instantiate(defs, modules.root);
    diagnostics.extend(found);
    let (design, found) = stave_solve::solve(&model);
    diagnostics.extend(found);
    if let Some(design) = &design
        && !diagnostics.iter().any(Diagnostic::is_error)
    {
        diagnostics.extend(stave_check::check(&model, design));
    }
    Checked {
        diagnostics,
        design,
    }
}

/// A length in millimetres.
pub fn mm(value: i64) -> Length {
    Length::from_nm(value * 1_000_000)
}

/// The part with a display name.
pub fn part<'d>(design: &'d SolvedDesign, name: &str) -> &'d Part {
    design
        .part_named(name)
        .unwrap_or_else(|| panic!("no part {name}"))
}

/// A part's drawn box.
pub fn drawn(design: &SolvedDesign, name: &str) -> WorldBox {
    part(design, name).drawn_box()
}

/// A region's box.
pub fn region(design: &SolvedDesign, name: &str) -> WorldBox {
    design
        .regions()
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no region {name}"))
        .bounds
}

/// The parts each pair of a joint binds, by name, in role order.
pub fn pairs(design: &SolvedDesign, joint: &str) -> Vec<Vec<String>> {
    let joint = design
        .joints()
        .iter()
        .find(|j| j.name == joint)
        .unwrap_or_else(|| panic!("no joint {joint}"));
    joint
        .pairs
        .iter()
        .map(|pair| {
            pair.roles
                .iter()
                .map(|role| design.part(role.part).unwrap().name.clone())
                .collect()
        })
        .collect()
}

/// `a + b`.
pub fn plus(a: Length, b: Length) -> Length {
    Length::from_nm(a.nm() + b.nm())
}

/// `a - b`.
pub fn minus(a: Length, b: Length) -> Length {
    Length::from_nm(a.nm() - b.nm())
}
