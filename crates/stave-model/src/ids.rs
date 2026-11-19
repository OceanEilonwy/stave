//! Typed ids. Every id is an index into one arena of [`crate::Defs`],
//! [`crate::Model`] or [`crate::solved::SolvedDesign`]; the newtypes keep the
//! arenas from being mixed up.

use serde::Serialize;

macro_rules! ids {
    ($( $(#[doc = $doc:literal])* $name:ident; )*) => {
        $(
            $(#[doc = $doc])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
            #[serde(transparent)]
            pub struct $name(u32);

            impl $name {
                /// The id of the element at `index` of its arena; `None` past
                /// `u32::MAX`.
                pub fn from_index(index: usize) -> Option<Self> {
                    u32::try_from(index).ok().map(Self)
                }

                /// The id of the element at `index`, saturating at `u32::MAX`.
                /// Arenas are filled from source files of at most 4 GiB, so
                /// they never hold that many items.
                pub fn from_usize(index: usize) -> Self {
                    Self(u32::try_from(index).unwrap_or(u32::MAX))
                }

                /// The index into its arena.
                pub fn index(self) -> usize {
                    // u32 always fits in usize on the platforms Stave supports
                    // (32 and 64 bit).
                    usize::try_from(self.0).unwrap_or(usize::MAX)
                }

                /// The raw number, for display.
                pub fn get(self) -> u32 {
                    self.0
                }
            }

            impl std::fmt::Display for $name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{}", self.0)
                }
            }
        )*
    };
}

ids! {
    /// A `param` declaration (generic: once per declaration, not per instance).
    ParamDefId;
    /// A `let` declaration.
    LetDefId;
    /// A `region` declaration.
    RegionDefId;
    /// A `part` declaration.
    PartDefId;
    /// A `joint` declaration.
    JointDefId;
    /// A `hardware` declaration.
    HardwareDefId;
    /// An `assembly` declaration.
    AssemblyDefId;
    /// A `require` declaration.
    RequireDefId;
    /// A `stock` item.
    StockId;
    /// A `profile` item.
    ProfileId;
    /// A `kind` item.
    KindId;
    /// A `recipe` item.
    RecipeId;
    /// A `component` item.
    ComponentId;
    /// A `scene` item.
    SceneDefId;
    /// A module or component instance: the scope a concrete item lives in.
    InstanceId;
    /// A concrete param: a param declaration in one instance.
    ParamId;
    /// A concrete `let`.
    LetId;
    /// A concrete part. An array element is its own part.
    PartId;
    /// A concrete region.
    RegionId;
    /// A concrete joint.
    JointId;
    /// A concrete piece of hardware.
    HardwareId;
    /// A concrete assembly (an instance of a component, or its mirror image).
    AssemblyId;
    /// A concrete `require`.
    RequireId;
    /// A scene of the project.
    SceneId;
    /// A pair of a joint: its index among the joint's
    /// [`pairs`](crate::solved::Joint::pairs).
    PairIndex;
    /// A seam of a part made of boards: its index among the part's
    /// [`seams`](crate::solved::Boards::seams).
    SeamIndex;
    /// A part a joint kind makes: its index among the kind's
    /// [`makes`](crate::defs::JointKindDef::makes).
    MadeIndex;
    /// A rule of a kind: its index among the kind's
    /// [`requires`](crate::defs::KindDef::requires).
    RequireIndex;
}

impl PairIndex {
    /// The first pair.
    pub const FIRST: PairIndex = PairIndex(0);
}

impl SeamIndex {
    /// The first seam, nearest the reference edge.
    pub const FIRST: SeamIndex = SeamIndex(0);

    /// Its number from 1, as the source writes it (`top.seam[1]`).
    pub fn number(self) -> u32 {
        self.0.saturating_add(1)
    }
}
