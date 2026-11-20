//! A project opened in the viewer: its files through the pipeline, and an
//! agent's suggestion drawn as ghosts over it.

use stave_model::solved::{AssemblyId, SolvedDesign};
use stave_modules::LoadError;
use stave_scene::pipeline::{self, Opened};
use stave_scene::{GhostVerdict, Ghosts, Highlight, MotionRange, NodeId, SceneGraph};

use crate::files::Files;

/// Whether an agent's suggestion checks clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionVerdict {
    /// The project with the suggestion has no errors.
    Fits,
    /// The project with the suggestion has errors.
    Refused,
}

impl SuggestionVerdict {
    /// The word the page shows: `fits` or `refused`.
    pub fn word(self) -> &'static str {
        match self {
            SuggestionVerdict::Fits => "fits",
            SuggestionVerdict::Refused => "refused",
        }
    }

    fn ghost(self) -> GhostVerdict {
        match self {
            SuggestionVerdict::Fits => GhostVerdict::Fits,
            SuggestionVerdict::Refused => GhostVerdict::Refused,
        }
    }
}

/// An assembly that moves (a door on its hinges, a drawer on its slides),
/// for a slider.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    /// The assembly, in the drawn design.
    pub assembly: AssemblyId,
    /// Its name as written: `door_l`.
    pub name: String,
    /// How far it goes.
    pub range: MotionRange,
}

/// What is drawn: the project's own scene, or the scene with the
/// suggestion made and its ghosts marked.
#[derive(Debug)]
enum Drawn {
    /// Nothing placed: the project has errors before placement.
    Nothing,
    /// The project as it is.
    Project(SceneGraph),
    /// The project with the suggestion made, alone: the project itself
    /// doesn't place, so there is nothing to draw it over.
    SuggestionAlone(SceneGraph),
    /// The suggestion's scene, drawn over the project: its ghosts are
    /// what it adds or changes.
    Suggestion(Box<Ghosts>),
}

/// A project opened from its files, with an agent's suggestion when the
/// page sent one.
#[derive(Debug)]
pub struct Project {
    project: Opened,
    suggestion: Option<(Opened, SuggestionVerdict)>,
    drawn: Drawn,
    motions: Vec<Motion>,
    highlight: Highlight,
}

impl Project {
    /// Opens the project in `files` and, when there is one, the suggestion
    /// whose files `suggestion` puts in place of the project's.
    pub fn open(files: &Files, suggestion: Option<&Files>) -> Result<Project, LoadError> {
        let project = pipeline::open_with(files.folder(), &files.memory())?;
        let suggestion = match suggestion {
            Some(changes) => {
                let changed = files.with_changes(changes);
                let opened = pipeline::open_with(changed.folder(), &changed.memory())?;
                let verdict = if opened.error_count() == 0 {
                    SuggestionVerdict::Fits
                } else {
                    SuggestionVerdict::Refused
                };
                Some((opened, verdict))
            }
            None => None,
        };
        let drawn = match (&project.design, &suggestion) {
            (Some(base), Some((opened, verdict))) => match &opened.design {
                Some(candidate) => {
                    let graph = pipeline::scene(base);
                    Drawn::Suggestion(Box::new(Ghosts::new(
                        base,
                        &graph,
                        candidate,
                        pipeline::viewer_options(),
                        verdict.ghost(),
                    )))
                }
                // The suggestion doesn't place: the project is drawn, and
                // the verdict says why there is nothing over it.
                None => Drawn::Project(pipeline::scene(base)),
            },
            (Some(base), None) => Drawn::Project(pipeline::scene(base)),
            (None, Some((opened, _))) => match &opened.design {
                Some(candidate) => Drawn::SuggestionAlone(pipeline::scene(candidate)),
                None => Drawn::Nothing,
            },
            (None, None) => Drawn::Nothing,
        };
        let mut built = Project {
            project,
            suggestion,
            drawn,
            motions: Vec::new(),
            highlight: Highlight::default(),
        };
        built.motions = built.find_motions();
        built.highlight = built.find_highlight();
        Ok(built)
    }

    /// The opened files whose design is drawn: the suggestion's when its
    /// scene is drawn, the project's otherwise.
    fn drawn_opened(&self) -> &Opened {
        match (&self.drawn, &self.suggestion) {
            (Drawn::Suggestion(_) | Drawn::SuggestionAlone(_), Some((opened, _))) => opened,
            (Drawn::Nothing | Drawn::Project(_), _)
            | (Drawn::Suggestion(_) | Drawn::SuggestionAlone(_), None) => &self.project,
        }
    }

    /// The design drawn, when something placed.
    pub fn design(&self) -> Option<&SolvedDesign> {
        self.drawn_opened().design.as_ref()
    }

    /// The scene drawn, when something placed: the suggestion's (with the
    /// project's unchanged parts) when there is one, else the project's.
    pub fn graph(&self) -> Option<&SceneGraph> {
        match &self.drawn {
            Drawn::Nothing => None,
            Drawn::Project(graph) | Drawn::SuggestionAlone(graph) => Some(graph),
            Drawn::Suggestion(ghosts) => Some(ghosts.graph()),
        }
    }

    /// The nodes of [`Project::graph`] that are the suggestion's ghosts:
    /// what it adds or changes. Empty without a suggestion.
    pub fn ghost_nodes(&self) -> Vec<NodeId> {
        match &self.drawn {
            Drawn::Suggestion(ghosts) => ghosts.ghosts().iter().map(|ghost| ghost.node).collect(),
            Drawn::Nothing | Drawn::Project(_) | Drawn::SuggestionAlone(_) => Vec::new(),
        }
    }

    /// The colour the ghosts are tinted, from the suggestion's verdict.
    pub fn ghost_tint(&self) -> Option<[f32; 3]> {
        match &self.drawn {
            Drawn::Suggestion(ghosts) => Some(ghosts.verdict().tint()),
            Drawn::Nothing | Drawn::Project(_) | Drawn::SuggestionAlone(_) => None,
        }
    }

    /// Whether the suggestion checks clean; `None` without one.
    pub fn verdict(&self) -> Option<SuggestionVerdict> {
        self.suggestion.as_ref().map(|(_, verdict)| *verdict)
    }

    /// The number of errors in the project as it is.
    pub fn error_count(&self) -> usize {
        self.project.error_count()
    }

    /// The number of errors in the project with the suggestion made;
    /// `None` without one.
    pub fn suggestion_error_count(&self) -> Option<usize> {
        self.suggestion
            .as_ref()
            .map(|(opened, _)| opened.error_count())
    }

    /// The project's title, when it placed.
    pub fn title(&self) -> Option<&str> {
        self.graph().map(|graph| graph.title.as_str())
    }

    /// Every assembly of the drawn design that moves, in assembly order.
    pub fn motions(&self) -> &[Motion] {
        &self.motions
    }

    /// What the drawn design's problems are about, to tint in red as the
    /// window does.
    pub fn highlight(&self) -> &Highlight {
        &self.highlight
    }

    fn find_motions(&self) -> Vec<Motion> {
        let (Some(design), Some(graph)) = (self.design(), self.graph()) else {
            return Vec::new();
        };
        design
            .assemblies()
            .iter()
            .filter_map(|assembly| {
                Some(Motion {
                    assembly: assembly.id,
                    name: assembly.name.clone(),
                    range: graph.motion_range(assembly.id)?,
                })
            })
            .collect()
    }

    fn find_highlight(&self) -> Highlight {
        let Some(graph) = self.graph() else {
            return Highlight::default();
        };
        let mut all = Highlight::default();
        for (_, contact) in &self.drawn_opened().contacts {
            let one = pipeline::highlight(graph, contact);
            all.nodes.extend(one.nodes);
            all.volumes.extend(one.volumes);
        }
        all.nodes.sort();
        all.nodes.dedup();
        all
    }
}
