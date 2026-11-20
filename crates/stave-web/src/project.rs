//! A project opened in the viewer: its files through the pipeline, and an
//! agent's suggestion drawn as ghosts over it.

use stave_model::solved::{AssemblyId, SolvedDesign};
use stave_modules::LoadError;
use stave_scene::pipeline::{self, Opened};
use stave_scene::{
    GhostOverlay, GhostVerdict, Ghosts, Highlight, Item, Look, MotionRange, NodeId, SceneGraph,
};

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

/// What is drawn: the project's own scene, or the project's scene with
/// the suggestion's ghosts over it.
#[derive(Debug)]
enum Drawn {
    /// Nothing placed: the project has errors before placement.
    Nothing,
    /// The project as it is.
    Project(SceneGraph),
    /// The project with the suggestion made, alone: the project itself
    /// doesn't place, so there is nothing to draw it over.
    SuggestionAlone(SceneGraph),
    /// The project's scene with the suggestion's ghosts (what it adds or
    /// changes) added after its nodes, and what they stand in for (what
    /// it changes or takes out) faded.
    Suggestion(Box<Suggested>),
}

/// A suggestion drawn over the project.
#[derive(Debug)]
struct Suggested {
    /// What the suggestion adds or changes, in its own scene.
    ghosts: Ghosts,
    /// The project's scene with the ghosts over it: what is drawn.
    overlay: GhostOverlay,
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
                    let ghosts = Ghosts::new(
                        base,
                        &graph,
                        candidate,
                        pipeline::viewer_options(),
                        verdict.ghost(),
                    );
                    let overlay = ghosts.over(&graph);
                    Drawn::Suggestion(Box::new(Suggested { ghosts, overlay }))
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

    /// The suggestion's opened files.
    fn suggestion_opened(&self) -> Option<&Opened> {
        self.suggestion.as_ref().map(|(opened, _)| opened)
    }

    /// The design the drawn scene's own nodes come from: the project's,
    /// or the suggestion's when it is drawn alone. A ghost's node comes
    /// from the suggestion's design ([`Project::item_of`]).
    pub fn design(&self) -> Option<&SolvedDesign> {
        match &self.drawn {
            Drawn::SuggestionAlone(_) => self.suggestion_opened()?.design.as_ref(),
            Drawn::Nothing | Drawn::Project(_) | Drawn::Suggestion(_) => {
                self.project.design.as_ref()
            }
        }
    }

    /// The scene drawn, when something placed: the project's, with the
    /// suggestion's ghosts added after its own nodes when there is one.
    pub fn graph(&self) -> Option<&SceneGraph> {
        match &self.drawn {
            Drawn::Nothing => None,
            Drawn::Project(graph) | Drawn::SuggestionAlone(graph) => Some(graph),
            Drawn::Suggestion(suggested) => Some(suggested.overlay.graph()),
        }
    }

    /// The design a node of [`Project::graph`] comes from, and what it
    /// draws there: the suggestion's design for a ghost, else
    /// [`Project::design`].
    pub fn item_of(&self, node: NodeId) -> Option<(&SolvedDesign, Item)> {
        if let Drawn::Suggestion(suggested) = &self.drawn
            && let Some(candidate) = suggested.overlay.candidate_node(node)
        {
            let design = self.suggestion_opened()?.design.as_ref()?;
            return Some((design, suggested.ghosts.graph().node(candidate)?.item));
        }
        Some((self.design()?, self.graph()?.node(node)?.item))
    }

    /// The nodes of [`Project::graph`] that are the suggestion's ghosts:
    /// what it adds or changes. Empty without a suggestion.
    pub fn ghost_nodes(&self) -> Vec<NodeId> {
        match &self.drawn {
            Drawn::Suggestion(suggested) => suggested.overlay.ghost_nodes().collect(),
            Drawn::Nothing | Drawn::Project(_) | Drawn::SuggestionAlone(_) => Vec::new(),
        }
    }

    /// The project's nodes the suggestion changes or takes out, drawn
    /// faded under its ghosts. Empty without a suggestion.
    pub fn faded_nodes(&self) -> &[NodeId] {
        match &self.drawn {
            Drawn::Suggestion(suggested) => suggested.overlay.faded(),
            Drawn::Nothing | Drawn::Project(_) | Drawn::SuggestionAlone(_) => &[],
        }
    }

    /// How each node of [`Project::graph`] looks: the ghosts see-through,
    /// what they stand in for faded, the rest as usual.
    pub fn looks(&self) -> Vec<Look> {
        match &self.drawn {
            Drawn::Suggestion(suggested) => suggested.overlay.looks(),
            Drawn::Project(graph) | Drawn::SuggestionAlone(graph) => {
                vec![Look::Normal; graph.nodes.len()]
            }
            Drawn::Nothing => Vec::new(),
        }
    }

    /// The colour the ghosts are tinted, from the suggestion's verdict.
    pub fn ghost_tint(&self) -> Option<[f32; 3]> {
        match &self.drawn {
            Drawn::Suggestion(suggested) => Some(suggested.ghosts.verdict().tint()),
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

    /// Every assembly of [`Project::design`] that moves, in assembly
    /// order. A ghost moves with the project's assembly of the same name.
    pub fn motions(&self) -> &[Motion] {
        &self.motions
    }

    /// What the problems are about, to tint in red as the window does:
    /// the suggestion's problems when there is one (on its ghosts, or on
    /// the project's nodes it leaves as they were), else the project's.
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
        let mut all = Highlight::default();
        match &self.drawn {
            Drawn::Nothing => {}
            Drawn::Project(graph) => {
                for (_, contact) in &self.project.contacts {
                    let one = pipeline::highlight(graph, contact);
                    all.nodes.extend(one.nodes);
                    all.volumes.extend(one.volumes);
                }
            }
            Drawn::SuggestionAlone(graph) => {
                for (_, contact) in self.suggestion_opened().map_or(&[][..], |o| &o.contacts) {
                    let one = pipeline::highlight(graph, contact);
                    all.nodes.extend(one.nodes);
                    all.volumes.extend(one.volumes);
                }
            }
            Drawn::Suggestion(suggested) => {
                let contacts = self.suggestion_opened().map_or(&[][..], |o| &o.contacts);
                for (_, contact) in contacts {
                    let one = pipeline::highlight(suggested.ghosts.graph(), contact);
                    all.nodes.extend(
                        one.nodes
                            .into_iter()
                            .filter_map(|node| suggested.overlay.of_candidate(node)),
                    );
                    all.volumes.extend(one.volumes);
                }
            }
        }
        all.nodes.sort();
        all.nodes.dedup();
        all
    }
}
