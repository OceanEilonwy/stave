//! Placement and sizes (layer 3): from a resolved [`Model`] to a
//! [`SolvedDesign`].
//!
//! There is no solver in the numerical sense. Every axis of every part is
//! fixed by exactly one placement term whose anchors belong to things
//! placed earlier, so each value is worked out **on demand** and
//! remembered: asking for a part's position along `up` asks for the
//! position of the face it stands against, which asks for that part's
//! position, and so on. Something asked for while it is being worked out is
//! a cycle, reported as `placement-cycle`. Values are exact fractions
//! throughout and rounded to whole nanometres once, when the solved design
//! is written.
//!
//! The order of work:
//!
//! 1. the project checks: exactly one part placed with `ground`;
//! 2. every part's orientation, drawn sizes (including `fit`) and position;
//! 3. every region;
//! 4. every joint: its pairs, what it adds to each part (`adds:`), the parts
//!    it makes, then its cuts (measured on the final cut sizes);
//! 5. every piece of hardware, its cuts and the assembly motions;
//! 6. mirrored assemblies: the source's results reflected, with
//!    connections to outside parts re-bound;
//! 7. requires at the current values, scenes, and the solved design.
//!
//! Every failure is a [`Diagnostic`] with the spec's code, a span and a help
//! line. Solving stops early only when nothing useful can be produced.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod boards;
mod build;
mod contact;
mod eval;
mod fixes;
mod hardware;
mod help;
mod joints;
mod mechanism;
mod members;
mod models;
mod pieces;
mod place;
mod rules;
mod walls;

use std::collections::HashMap;

use stave_lang::ast::Span;
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::Model;
use stave_model::defs::DefId;
use stave_model::ids::{JointId, LetId, ParamId, PartId, RegionId};
use stave_model::solved::SolvedDesign;
use stave_model::value::{RegionValue, Value};

pub use place::Terms;

/// Places everything in `model`. Returns the solved design (when solving
/// got far enough to produce one) and the diagnostics of this stage.
pub fn solve(model: &Model) -> (Option<SolvedDesign>, Vec<Diagnostic>) {
    let Some(project) = model.defs.project(model.root) else {
        // A library: nothing to place. Its kinds and components were
        // checked when resolving.
        return (None, Vec::new());
    };
    let mut solver = Solver::new(model, project.file, project.span);
    let design = solver.run();
    let mut diagnostics = solver.diagnostics;
    // The rules of hardware kinds are fit (layer 4): reported only when no
    // earlier layer found an error, so one mistake is one error.
    if !diagnostics
        .iter()
        .any(|d| d.is_error() && d.code.layer() < Code::Adjust.layer())
    {
        diagnostics.extend(solver.deferred);
    }
    diagnostics.sort_by_key(|d| {
        d.primary()
            .map(|label| (label.file, label.span.start(), label.span.end()))
    });
    diagnostics.dedup();
    (design, diagnostics)
}

/// Something went wrong and has been reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reported;

/// The result of anything that may fail; the failure is already reported.
pub(crate) type Solved<T> = Result<T, Reported>;

/// The memo state of a value worked out on demand.
#[derive(Debug, Clone, Default)]
pub(crate) enum Memo<T> {
    #[default]
    Todo,
    Busy,
    Done(T),
    Failed,
}

/// What is being worked out, for cycle reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Frame {
    Position(PartId, stave_lang::placement::Axis),
    Size(PartId, stave_lang::placement::PartAxis),
    Orientation(PartId),
    Region(RegionId),
    Param(ParamId),
    Let(LetId),
    Joint(JointId),
}

pub(crate) struct Solver<'m> {
    pub(crate) model: &'m Model,
    /// The file and span of the project item, for diagnostics about the
    /// whole design.
    pub(crate) project_file: FileId,
    pub(crate) project_span: Span,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) params: Vec<Memo<Value>>,
    pub(crate) lets: Vec<Memo<Value>>,
    pub(crate) regions: Vec<Memo<RegionValue>>,
    pub(crate) parts: Vec<place::PartState>,
    pub(crate) joints: Vec<Memo<joints::Binding>>,
    pub(crate) hardware: Vec<Memo<hardware::Placed>>,
    pub(crate) stack: Vec<(Frame, Option<(FileId, Span)>)>,
    pub(crate) mirror_planes: HashMap<
        stave_model::ids::AssemblyId,
        Memo<(stave_lang::placement::Axis, stave_lang::units::ExactLength)>,
    >,
    pub(crate) extensions: Vec<Vec<joints::Extend>>,
    pub(crate) reported_cycles: Vec<Frame>,
    /// Something was skipped because an item it depends on has an error
    /// reported by an earlier layer.
    pub(crate) skipped: bool,
    /// Spaced elements that run past their `to` anchor.
    pub(crate) overruns: Vec<stave_model::solved::Overrun>,
    /// The kinds' rules that read `wall(cut)`, worked out but for the
    /// wall, which the fit check measures once every cut is made.
    pub(crate) wall_rules: Vec<stave_model::solved::WallRule>,
    /// Hardware kinds whose mechanism has been reported (a kind's mechanism
    /// is checked for each piece of hardware, and reported once).
    pub(crate) reported_mechanisms: std::collections::BTreeSet<stave_model::ids::KindId>,
    /// What the rules of hardware kinds found (layer 4), in the order the
    /// hardware was placed: reported only when no earlier layer found an
    /// error.
    pub(crate) deferred: Vec<Diagnostic>,
    /// The boards of a part made of boards, while its board joints are
    /// worked out: each stands in for a part under its own id (see
    /// [`boards::BOARD_ID_BASE`]).
    pub(crate) boards: std::collections::BTreeMap<PartId, boards::BoardPart>,
}

impl<'m> Solver<'m> {
    fn new(model: &'m Model, project_file: FileId, project_span: Span) -> Self {
        Solver {
            model,
            project_file,
            project_span,
            diagnostics: Vec::new(),
            params: vec![Memo::Todo; model.params.len()],
            lets: vec![Memo::Todo; model.lets.len()],
            regions: vec![Memo::Todo; model.regions.len()],
            parts: (0..model.parts.len())
                .map(|_| place::PartState::default())
                .collect(),
            joints: vec![Memo::Todo; model.joints.len()],
            hardware: vec![Memo::Todo; model.hardware.len()],
            stack: Vec::new(),
            mirror_planes: HashMap::new(),
            extensions: vec![Vec::new(); model.parts.len()],
            reported_cycles: Vec::new(),
            skipped: false,
            overruns: Vec::new(),
            wall_rules: Vec::new(),
            reported_mechanisms: std::collections::BTreeSet::new(),
            deferred: Vec::new(),
            boards: std::collections::BTreeMap::new(),
        }
    }

    /// Skips something that depends on an item with an earlier error.
    pub(crate) fn skip(&mut self) -> Reported {
        self.skipped = true;
        Reported
    }

    /// `true` if a declaration had a name or type error.
    pub(crate) fn poisoned(&self, def: DefId) -> bool {
        self.model.defs.is_poisoned(def)
    }

    pub(crate) fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn error(
        &mut self,
        code: Code,
        message: impl Into<String>,
        file: FileId,
        span: Span,
        help: impl Into<String>,
    ) -> Reported {
        self.report(Diagnostic::error(code, message, Label::new(file, span)).with_help(help));
        Reported
    }

    /// Where a declaration is.
    pub(crate) fn place_of(&self, def: DefId) -> (FileId, Span) {
        match self.model.defs.decl(def) {
            Some(decl) => (decl.file, decl.span),
            None => (self.project_file, self.project_span),
        }
    }

    /// Starts working on something; detects cycles.
    pub(crate) fn enter(&mut self, frame: Frame, at: Option<(FileId, Span)>) {
        self.stack.push((frame, at));
    }

    pub(crate) fn leave(&mut self) {
        self.stack.pop();
    }

    /// Reports that `frame` was asked for while being worked out.
    pub(crate) fn cycle(&mut self, frame: Frame) -> Reported {
        if self.reported_cycles.contains(&frame) {
            return Reported;
        }
        let start = self
            .stack
            .iter()
            .position(|(f, _)| *f == frame)
            .unwrap_or(0);
        let cycle: Vec<(Frame, Option<(FileId, Span)>)> = self
            .stack
            .get(start..)
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        for (f, _) in &cycle {
            self.reported_cycles.push(*f);
        }
        self.reported_cycles.push(frame);
        let names: Vec<String> = {
            let mut names: Vec<String> = Vec::new();
            for (f, _) in &cycle {
                let name = self.frame_name(*f);
                if names.last() != Some(&name) {
                    names.push(name);
                }
            }
            names
        };
        let last = self.stack.last().copied();
        let (file, span) = last
            .and_then(|(_, at)| at)
            .unwrap_or((self.project_file, self.project_span));
        let message = match (names.first(), names.last()) {
            (Some(a), Some(b)) if names.len() >= 2 => {
                format!("{b} and {a} are placed against each other")
            }
            (Some(a), _) => format!("{a} and {a} are placed against each other"),
            _ => "these items are placed against each other".to_owned(),
        };
        let mut diagnostic =
            Diagnostic::error(Code::PlacementCycle, message, Label::new(file, span)).with_help(
                "place one of them against something placed earlier, such as the ground part",
            );
        if names.len() > 2 {
            diagnostic = diagnostic.with_note(format!(
                "the cycle is {} -> {}",
                names.join(" -> "),
                names.first().cloned().unwrap_or_default()
            ));
        }
        self.report(diagnostic);
        Reported
    }

    fn frame_name(&self, frame: Frame) -> String {
        match frame {
            Frame::Position(part, _) | Frame::Size(part, _) | Frame::Orientation(part) => {
                self.part_name(part)
            }
            Frame::Region(region) => self
                .model
                .regions
                .get(region.index())
                .map(|r| r.name.clone())
                .unwrap_or_default(),
            Frame::Param(param) => self
                .model
                .params
                .get(param.index())
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            Frame::Let(binding) => self
                .model
                .lets
                .get(binding.index())
                .map(|l| l.name.clone())
                .unwrap_or_default(),
            Frame::Joint(joint) => self
                .model
                .joints
                .get(joint.index())
                .map(|j| j.name.clone())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn part_name(&self, part: PartId) -> String {
        if let Some(board) = self.boards.get(&part) {
            let word = match board.division {
                boards::Division::Board => "board",
                boards::Division::Piece => "piece",
            };
            return format!(
                "{}.{word}[{}]",
                self.part_name(board.part),
                board.index.get()
            );
        }
        self.model
            .parts
            .get(part.index())
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    fn run(&mut self) -> Option<SolvedDesign> {
        self.check_ground().ok()?;
        build::build(self)
    }
}
