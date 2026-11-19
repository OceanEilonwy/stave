//! Instantiation: from resolved definitions to a concrete [`Model`].
//!
//! Every module is instantiated once (its params, lets and requires); the
//! project module's design items are instantiated, and each `assembly`
//! instantiates its component once per element. Arrays are expanded into
//! one concrete item per element, so `drawer[top].side[left]` is a part with
//! its own [`PartId`].
//!
//! Nothing is evaluated here except array counts, which must be known to
//! know how many items there are. Values are worked out on demand by
//! `stave-solve`, in the [`Ctx`] each item records: the instance whose
//! names it sees, and the element of the array being declared (for
//! `[same]`).
//!
//! A mirrored assembly instantiates its component again; every item of it
//! records the item of the source it is the image of, and the solver
//! reflects the source's results instead of evaluating the copy.

use std::collections::BTreeMap;

use stave_lang::ast::{BinaryOp, Span};
use stave_lang::units::Rational;
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_modules::ModuleId;

use crate::defs::{ArrayDef, AssemblyKind, DefId, Defs, RExpr, RExprKind, Scope, Symbol};
use crate::ids::{
    AssemblyDefId, AssemblyId, ComponentId, HardwareDefId, HardwareId, InstanceId, JointDefId,
    JointId, LetDefId, LetId, ParamDefId, ParamId, PartDefId, PartId, RegionDefId, RegionId,
    RequireDefId, RequireId, SceneDefId,
};

/// Where an expression is evaluated: the instance whose names it sees and
/// the element of the array being declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ctx {
    /// The instance.
    pub instance: InstanceId,
    /// The index of the element being declared, inside an array
    /// declaration.
    pub element: Option<usize>,
}

/// One item, or the elements of an array in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot<T> {
    /// A single item.
    One(T),
    /// An array's elements.
    Many(Vec<T>),
}

impl<T> Slot<T> {
    /// Every element, borrowed: the item alone, or the array's elements.
    pub fn as_slice(&self) -> &[T] {
        match self {
            Slot::One(item) => std::slice::from_ref(item),
            Slot::Many(items) => items,
        }
    }
}

impl<T: Copy> Slot<T> {
    /// Every element, copied out. Prefer [`Slot::as_slice`] to count,
    /// test or walk them.
    pub fn all(&self) -> Vec<T> {
        self.as_slice().to_vec()
    }

    /// The element at an index of an array; the item itself otherwise.
    pub fn element(&self, index: usize) -> Option<T> {
        match self {
            Slot::One(item) => Some(*item),
            Slot::Many(items) => items.get(index).copied(),
        }
    }
}

/// A concrete item in an instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemSlot {
    /// A param.
    Param(ParamId),
    /// A let.
    Let(LetId),
    /// Parts.
    Part(Slot<PartId>),
    /// Regions.
    Region(Slot<RegionId>),
    /// Joints.
    Joint(Slot<JointId>),
    /// Hardware.
    Hardware(Slot<HardwareId>),
    /// Assemblies.
    Assembly(Slot<AssemblyId>),
    /// A require.
    Require(RequireId),
}

/// What an instance is an instance of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceKind {
    /// A module (every module is instantiated once).
    Module(ModuleId),
    /// A component, for one assembly.
    Component {
        /// The component.
        component: ComponentId,
        /// The assembly.
        assembly: AssemblyId,
    },
}

/// A module or component instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// Its id.
    pub id: InstanceId,
    /// The module whose names it sees.
    pub module: ModuleId,
    /// What it instantiates.
    pub kind: InstanceKind,
    /// Its items by declaration.
    pub items: BTreeMap<DefId, ItemSlot>,
    /// What its item names are prefixed with: `drawer[top].`.
    pub prefix: String,
    /// The instance it is the mirror image of.
    pub mirror_of: Option<InstanceId>,
}

/// Where a param's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamSource {
    /// Its default, evaluated in its own instance.
    Default,
    /// An assembly field, evaluated where the assembly is declared.
    Argument {
        /// The assembly declaration.
        assembly: AssemblyDefId,
        /// The index into its arguments.
        index: usize,
        /// Where it is evaluated.
        ctx: Ctx,
    },
    /// The same value as another param (a mirrored assembly's).
    SameAs(ParamId),
}

macro_rules! concrete {
    ($( $(#[doc = $doc:literal])* $name:ident { id: $id:ty, def: $def:ty } )*) => {
        $(
            $(#[doc = $doc])*
            #[derive(Debug, Clone, PartialEq, Eq)]
            pub struct $name {
                /// Its id.
                pub id: $id,
                /// Its declaration.
                pub def: $def,
                /// Where its expressions are evaluated.
                pub ctx: Ctx,
                /// Its name as written: `drawer[top].side[left]`.
                pub name: String,
                /// The innermost assembly it belongs to.
                pub assembly: Option<AssemblyId>,
                /// The item it is the mirror image of, in a mirrored assembly.
                pub mirror_of: Option<$id>,
            }
        )*
    };
}

concrete! {
    /// A concrete part.
    PartInst { id: PartId, def: PartDefId }
    /// A concrete region.
    RegionInst { id: RegionId, def: RegionDefId }
    /// A concrete joint.
    JointInst { id: JointId, def: JointDefId }
    /// A concrete piece of hardware.
    HardwareInst { id: HardwareId, def: HardwareDefId }
    /// A concrete let.
    LetInst { id: LetId, def: LetDefId }
    /// A concrete require.
    RequireInst { id: RequireId, def: RequireDefId }
}

/// A concrete param.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamInst {
    /// Its id.
    pub id: ParamId,
    /// Its declaration.
    pub def: ParamDefId,
    /// Its instance.
    pub instance: InstanceId,
    /// Its name as written.
    pub name: String,
    /// Where its value comes from.
    pub source: ParamSource,
}

/// A concrete assembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyInst {
    /// Its id.
    pub id: AssemblyId,
    /// Its declaration.
    pub def: AssemblyDefId,
    /// Where it is declared.
    pub ctx: Ctx,
    /// Its name as written.
    pub name: String,
    /// The assembly it is inside.
    pub parent: Option<AssemblyId>,
    /// Its component.
    pub component: ComponentId,
    /// The component instance.
    pub inner: InstanceId,
    /// The assembly it is the mirror image of.
    pub mirror_of: Option<AssemblyId>,
}

/// The concrete design: every item with an id.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The resolved definitions.
    pub defs: Defs,
    /// The project module.
    pub root: ModuleId,
    /// Every instance; the first ones are the modules, in module order.
    pub instances: Vec<Instance>,
    /// Params.
    pub params: Vec<ParamInst>,
    /// Lets.
    pub lets: Vec<LetInst>,
    /// Parts.
    pub parts: Vec<PartInst>,
    /// Regions.
    pub regions: Vec<RegionInst>,
    /// Joints.
    pub joints: Vec<JointInst>,
    /// Hardware.
    pub hardware: Vec<HardwareInst>,
    /// Assemblies.
    pub assemblies: Vec<AssemblyInst>,
    /// Requires.
    pub requires: Vec<RequireInst>,
    /// The project's scenes, in source order.
    pub scenes: Vec<SceneDefId>,
}

impl Model {
    /// The instance of a module.
    pub fn module_instance(&self, module: ModuleId) -> Option<InstanceId> {
        InstanceId::from_index(module.index())
    }

    /// An instance.
    pub fn instance(&self, id: InstanceId) -> Option<&Instance> {
        self.instances.get(id.index())
    }

    /// The instance whose names a declaration in `scope` sees, when the
    /// expression is evaluated in `ctx`.
    pub fn scope_instance(&self, scope: Scope, ctx: Ctx) -> Option<InstanceId> {
        match scope {
            Scope::Module(module) => self.module_instance(module),
            Scope::Component(_) => Some(ctx.instance),
        }
    }

    /// The concrete item a declaration names, seen from `ctx`.
    pub fn item(&self, def: DefId, ctx: Ctx) -> Option<&ItemSlot> {
        let scope = self.defs.decl(def)?.scope;
        let instance = self.scope_instance(scope, ctx)?;
        self.instance(instance)?.items.get(&def)
    }

    /// Whether a scope is the project module (where parts may be declared).
    pub fn is_root(&self, scope: Scope) -> bool {
        scope == Scope::Module(self.root)
    }
}

/// Instantiates the project `root` of resolved definitions. Returns the
/// model and the diagnostics of this step (array counts that aren't known).
pub fn instantiate(defs: Defs, root: ModuleId) -> (Model, Vec<Diagnostic>) {
    let mut builder = Builder {
        model: Model {
            defs,
            root,
            instances: Vec::new(),
            params: Vec::new(),
            lets: Vec::new(),
            parts: Vec::new(),
            regions: Vec::new(),
            joints: Vec::new(),
            hardware: Vec::new(),
            assemblies: Vec::new(),
            requires: Vec::new(),
            scenes: Vec::new(),
        },
        diagnostics: Vec::new(),
        pending_mirrors: Vec::new(),
    };
    builder.run();
    if builder.diagnostics.iter().any(Diagnostic::is_error) {
        builder.model.defs.earlier_errors = true;
    }
    (builder.model, builder.diagnostics)
}

struct Builder {
    model: Model,
    diagnostics: Vec<Diagnostic>,
    /// Mirrored assemblies whose source isn't instantiated yet.
    pending_mirrors: Vec<PendingMirror>,
}

#[derive(Clone)]
struct PendingMirror {
    def: AssemblyDefId,
    ctx: Ctx,
    prefix: String,
    parent: Option<AssemblyId>,
    elements: Vec<String>,
}

fn push<T>(arena: &mut Vec<T>, make: impl FnOnce(usize) -> T) -> usize {
    let index = arena.len();
    arena.push(make(index));
    index
}

impl Builder {
    fn run(&mut self) {
        let module_count = self.model.defs.modules.len();
        for index in 0..module_count {
            let Some(module) = self.model.defs.modules.get(index).map(|m| m.id) else {
                continue;
            };
            self.new_instance(module, InstanceKind::Module(module), String::new(), None);
        }
        for index in 0..module_count {
            let Some(module_defs) = self.model.defs.modules.get(index) else {
                continue;
            };
            let module = module_defs.id;
            let items = module_defs.items.clone();
            let Some(instance) = InstanceId::from_index(index) else {
                continue;
            };
            let is_root = module == self.model.root;
            for def in items {
                let design = matches!(def, DefId::Param(_) | DefId::Let(_) | DefId::Require(_));
                if design || is_root {
                    self.instantiate_item(def, instance, None, "", None);
                }
                if let (DefId::Scene(scene), true) = (def, is_root) {
                    self.model.scenes.push(scene);
                }
            }
        }
        // Mirrors whose source came later, possibly chained.
        loop {
            let pending = std::mem::take(&mut self.pending_mirrors);
            if pending.is_empty() {
                break;
            }
            let before = pending.len();
            for mirror in pending {
                self.try_mirror(mirror);
            }
            if self.pending_mirrors.len() == before {
                for mirror in std::mem::take(&mut self.pending_mirrors) {
                    self.report_unresolved_mirror(&mirror);
                }
                break;
            }
        }
    }

    fn new_instance(
        &mut self,
        module: ModuleId,
        kind: InstanceKind,
        prefix: String,
        mirror_of: Option<InstanceId>,
    ) -> InstanceId {
        let index = push(&mut self.model.instances, |index| Instance {
            id: InstanceId::from_usize(index),
            module,
            kind,
            items: BTreeMap::new(),
            prefix,
            mirror_of,
        });
        InstanceId::from_usize(index)
    }

    fn file_of(&self, def: DefId) -> Option<(FileId, Span)> {
        self.model.defs.decl(def).map(|decl| (decl.file, decl.span))
    }

    /// The element labels of an array declaration (numbers for a count).
    fn elements(&mut self, def: DefId, ctx: Ctx) -> Option<Vec<String>> {
        let array = self.model.defs.array(def)?.clone();
        Some(match array {
            ArrayDef::Labels(labels) => labels.into_iter().map(|(label, _)| label).collect(),
            ArrayDef::Count(count) => match self.const_count(&count, ctx) {
                Some(n) => (0..n).map(|i| i.to_string()).collect(),
                // The count already has its own name or type error.
                None if matches!(count.kind, RExprKind::Error)
                    || self.model.defs.is_poisoned(def) =>
                {
                    Vec::new()
                }
                None => {
                    let name = self
                        .model
                        .defs
                        .decl(def)
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    let Some((file, _)) = self.file_of(def) else {
                        return Some(Vec::new());
                    };
                    self.diagnostics.push(
                        Diagnostic::error(
                            Code::TypeMismatch,
                            format!(
                                "'{name}' needs a whole number of elements known before placing"
                            ),
                            Label::new(file, count.span),
                        )
                        .with_help("write a count, a let, or a param whose within is a list of counts, such as param shelves = 3 within [2, 3, 4]"),
                    );
                    Vec::new()
                }
            },
        })
    }

    fn const_count(&self, expr: &RExpr, ctx: Ctx) -> Option<u32> {
        let value = self.const_number(expr, ctx, 0)?;
        if !value.is_integer() || value.is_negative() {
            return None;
        }
        u32::try_from(value.numerator()).ok()
    }

    fn const_number(&self, expr: &RExpr, ctx: Ctx, depth: u32) -> Option<Rational> {
        if depth > 64 {
            return None;
        }
        match &expr.kind {
            RExprKind::Number(n) => Some(*n),
            RExprKind::Binary { op, lhs, rhs } => {
                let (a, b) = (
                    self.const_number(lhs, ctx, depth + 1)?,
                    self.const_number(rhs, ctx, depth + 1)?,
                );
                match op {
                    BinaryOp::Add => a.checked_add(b),
                    BinaryOp::Sub => a.checked_sub(b),
                    BinaryOp::Mul => a.checked_mul(b),
                    BinaryOp::Div | BinaryOp::In => a.checked_div(b),
                    _ => None,
                }
            }
            RExprKind::Symbol(Symbol::Def(def @ DefId::Param(_))) => {
                let ItemSlot::Param(param) = self.model.item(*def, ctx)? else {
                    return None;
                };
                let param = self.model.params.get(param.index())?;
                match param.source {
                    ParamSource::Default => {
                        let DefId::Param(id) = def else { return None };
                        let param_def = self.model.defs.param(*id)?;
                        // A count over a continuous range can't be tried
                        // value by value; it needs a within list.
                        if matches!(param_def.domain, Some(crate::defs::Domain::Range { .. })) {
                            return None;
                        }
                        let default = param_def.default.as_ref()?;
                        let ctx = Ctx {
                            instance: param.instance,
                            element: None,
                        };
                        self.const_number(default, ctx, depth + 1)
                    }
                    ParamSource::Argument {
                        assembly,
                        index,
                        ctx,
                    } => {
                        let AssemblyKind::Instance { args, .. } =
                            &self.model.defs.assembly(assembly)?.kind
                        else {
                            return None;
                        };
                        self.const_number(&args.get(index)?.1, ctx, depth + 1)
                    }
                    ParamSource::SameAs(_) => None,
                }
            }
            RExprKind::Symbol(Symbol::Def(DefId::Let(id))) => {
                let value = &self.model.defs.let_def(*id)?.value;
                self.const_number(value, ctx, depth + 1)
            }
            _ => None,
        }
    }

    fn instantiate_item(
        &mut self,
        def: DefId,
        instance: InstanceId,
        assembly: Option<AssemblyId>,
        prefix: &str,
        mirror_of: Option<InstanceId>,
    ) {
        let ctx = Ctx {
            instance,
            element: None,
        };
        let Some(decl) = self.model.defs.decl(def) else {
            return;
        };
        let name = decl.name.clone();
        let source_slot = mirror_of
            .and_then(|m| self.model.instance(m))
            .and_then(|i| i.items.get(&def))
            .cloned();
        let slot = match def {
            DefId::Param(id) => {
                let source = match &source_slot {
                    Some(ItemSlot::Param(source)) => ParamSource::SameAs(*source),
                    _ => ParamSource::Default,
                };
                let index = push(&mut self.model.params, |index| ParamInst {
                    id: ParamId::from_usize(index),
                    def: id,
                    instance,
                    name: format!("{prefix}{name}"),
                    source,
                });
                ItemSlot::Param(ParamId::from_usize(index))
            }
            DefId::Let(id) => {
                let source = match &source_slot {
                    Some(ItemSlot::Let(source)) => Some(*source),
                    _ => None,
                };
                let index = push(&mut self.model.lets, |index| LetInst {
                    id: LetId::from_usize(index),
                    def: id,
                    ctx,
                    name: format!("{prefix}{name}"),
                    assembly,
                    mirror_of: source,
                });
                ItemSlot::Let(LetId::from_usize(index))
            }
            DefId::Require(id) => {
                let source = match &source_slot {
                    Some(ItemSlot::Require(source)) => Some(*source),
                    _ => None,
                };
                let index = push(&mut self.model.requires, |index| RequireInst {
                    id: RequireId::from_usize(index),
                    def: id,
                    ctx,
                    name: format!("{prefix}{name}"),
                    assembly,
                    mirror_of: source,
                });
                ItemSlot::Require(RequireId::from_usize(index))
            }
            DefId::Part(id) => {
                let sources = slot_sources(source_slot.as_ref(), |s| match s {
                    ItemSlot::Part(slot) => Some(slot.all()),
                    _ => None,
                });
                let ids =
                    self.array_items(def, ctx, prefix, &name, |builder, element, label, index| {
                        let item_ctx = Ctx { instance, element };
                        let id = push(&mut builder.model.parts, |i| PartInst {
                            id: PartId::from_usize(i),
                            def: id,
                            ctx: item_ctx,
                            name: label,
                            assembly,
                            mirror_of: sources.as_ref().and_then(|s| s.get(index).copied()),
                        });
                        PartId::from_usize(id)
                    });
                ItemSlot::Part(ids)
            }
            DefId::Region(id) => {
                let sources = slot_sources(source_slot.as_ref(), |s| match s {
                    ItemSlot::Region(slot) => Some(slot.all()),
                    _ => None,
                });
                let ids =
                    self.array_items(def, ctx, prefix, &name, |builder, element, label, index| {
                        let id = push(&mut builder.model.regions, |i| RegionInst {
                            id: RegionId::from_usize(i),
                            def: id,
                            ctx: Ctx { instance, element },
                            name: label,
                            assembly,
                            mirror_of: sources.as_ref().and_then(|s| s.get(index).copied()),
                        });
                        RegionId::from_usize(id)
                    });
                ItemSlot::Region(ids)
            }
            DefId::Joint(id) => {
                let sources = slot_sources(source_slot.as_ref(), |s| match s {
                    ItemSlot::Joint(slot) => Some(slot.all()),
                    _ => None,
                });
                let ids =
                    self.array_items(def, ctx, prefix, &name, |builder, element, label, index| {
                        let id = push(&mut builder.model.joints, |i| JointInst {
                            id: JointId::from_usize(i),
                            def: id,
                            ctx: Ctx { instance, element },
                            name: label,
                            assembly,
                            mirror_of: sources.as_ref().and_then(|s| s.get(index).copied()),
                        });
                        JointId::from_usize(id)
                    });
                ItemSlot::Joint(ids)
            }
            DefId::Hardware(id) => {
                let sources = slot_sources(source_slot.as_ref(), |s| match s {
                    ItemSlot::Hardware(slot) => Some(slot.all()),
                    _ => None,
                });
                let ids =
                    self.array_items(def, ctx, prefix, &name, |builder, element, label, index| {
                        let id = push(&mut builder.model.hardware, |i| HardwareInst {
                            id: HardwareId::from_usize(i),
                            def: id,
                            ctx: Ctx { instance, element },
                            name: label,
                            assembly,
                            mirror_of: sources.as_ref().and_then(|s| s.get(index).copied()),
                        });
                        HardwareId::from_usize(id)
                    });
                ItemSlot::Hardware(ids)
            }
            DefId::Assembly(id) => {
                let elements = self.elements(def, ctx);
                let sources = slot_sources(source_slot.as_ref(), |s| match s {
                    ItemSlot::Assembly(slot) => Some(slot.all()),
                    _ => None,
                });
                match self.model.defs.assembly(id).map(|a| a.kind.clone()) {
                    Some(AssemblyKind::Instance { component, .. }) => {
                        let labels = elements.clone().unwrap_or_else(|| vec![String::new()]);
                        let mut ids = Vec::new();
                        for (index, label) in labels.iter().enumerate() {
                            let element = elements.as_ref().map(|_| index);
                            let full = element_name(
                                prefix,
                                &name,
                                elements.as_ref().map(|_| label.as_str()),
                            );
                            let source = sources.as_ref().and_then(|s| s.get(index).copied());
                            let assembly_id = self.new_assembly(
                                id,
                                Ctx { instance, element },
                                full,
                                assembly,
                                component,
                                source,
                            );
                            ids.push(assembly_id);
                        }
                        if elements.is_some() {
                            ItemSlot::Assembly(Slot::Many(ids))
                        } else {
                            match ids.first() {
                                Some(first) => ItemSlot::Assembly(Slot::One(*first)),
                                None => return,
                            }
                        }
                    }
                    Some(AssemblyKind::Mirror(_)) => {
                        self.pending_mirrors.push(PendingMirror {
                            def: id,
                            ctx,
                            prefix: prefix.to_owned(),
                            parent: assembly,
                            elements: elements.unwrap_or_default(),
                        });
                        return;
                    }
                    Some(AssemblyKind::Invalid) | None => return,
                }
            }
            DefId::Stock(_)
            | DefId::Profile(_)
            | DefId::Kind(_)
            | DefId::Recipe(_)
            | DefId::Component(_)
            | DefId::Scene(_) => return,
        };
        if let Some(instance) = self.model.instances.get_mut(instance.index()) {
            instance.items.insert(def, slot);
        }
    }

    /// Creates one item per element of an array declaration (or one).
    fn array_items<T: Copy>(
        &mut self,
        def: DefId,
        ctx: Ctx,
        prefix: &str,
        name: &str,
        mut make: impl FnMut(&mut Self, Option<usize>, String, usize) -> T,
    ) -> Slot<T> {
        match self.elements(def, ctx) {
            None => Slot::One(make(self, None, format!("{prefix}{name}"), 0)),
            Some(labels) => {
                let mut items = Vec::new();
                for (index, label) in labels.iter().enumerate() {
                    items.push(make(
                        self,
                        Some(index),
                        element_name(prefix, name, Some(label)),
                        index,
                    ));
                }
                Slot::Many(items)
            }
        }
    }

    fn new_assembly(
        &mut self,
        def: AssemblyDefId,
        ctx: Ctx,
        name: String,
        parent: Option<AssemblyId>,
        component: ComponentId,
        mirror_of: Option<AssemblyId>,
    ) -> AssemblyId {
        let module = match self.model.defs.component(component).map(|c| c.decl.scope) {
            Some(Scope::Module(module)) => module,
            _ => self.model.root,
        };
        let assembly_id = AssemblyId::from_usize(self.model.assemblies.len());
        let source_instance = mirror_of
            .and_then(|m| self.model.assemblies.get(m.index()))
            .map(|a| a.inner);
        let inner = self.new_instance(
            module,
            InstanceKind::Component {
                component,
                assembly: assembly_id,
            },
            format!("{name}."),
            source_instance,
        );
        self.model.assemblies.push(AssemblyInst {
            id: assembly_id,
            def,
            ctx,
            name: name.clone(),
            parent,
            component,
            inner,
            mirror_of,
        });
        let items = self
            .model
            .defs
            .component(component)
            .map(|c| c.items.clone())
            .unwrap_or_default();
        // Params first, so arguments are attached before counts need them.
        for item in &items {
            if let DefId::Param(_) = item {
                self.instantiate_item(
                    *item,
                    inner,
                    Some(assembly_id),
                    &format!("{name}."),
                    source_instance,
                );
            }
        }
        if source_instance.is_none() {
            self.attach_arguments(def, ctx, inner);
        }
        for item in items {
            if !matches!(item, DefId::Param(_)) {
                self.instantiate_item(
                    item,
                    inner,
                    Some(assembly_id),
                    &format!("{name}."),
                    source_instance,
                );
            }
        }
        assembly_id
    }

    fn attach_arguments(&mut self, def: AssemblyDefId, ctx: Ctx, inner: InstanceId) {
        let Some(AssemblyKind::Instance { args, .. }) =
            self.model.defs.assembly(def).map(|a| &a.kind)
        else {
            return;
        };
        let targets: Vec<(usize, ParamDefId)> =
            args.iter().enumerate().map(|(i, (p, _))| (i, *p)).collect();
        for (index, param_def) in targets {
            let Some(ItemSlot::Param(param)) = self
                .model
                .instance(inner)
                .and_then(|i| i.items.get(&DefId::Param(param_def)))
                .cloned()
            else {
                continue;
            };
            if let Some(param) = self.model.params.get_mut(param.index()) {
                param.source = ParamSource::Argument {
                    assembly: def,
                    index,
                    ctx,
                };
            }
        }
    }

    /// Resolves a mirror's source statically: an assembly name, possibly
    /// indexed by a label.
    fn mirror_source(&self, expr: &RExpr, ctx: Ctx) -> Option<AssemblyId> {
        match &expr.kind {
            RExprKind::Symbol(Symbol::Def(def @ DefId::Assembly(_))) => {
                match self.model.item(*def, ctx)? {
                    ItemSlot::Assembly(Slot::One(id)) => Some(*id),
                    _ => None,
                }
            }
            RExprKind::Index {
                base,
                index: crate::defs::Index::Label(position),
            } => {
                let RExprKind::Symbol(Symbol::Def(def @ DefId::Assembly(_))) = &base.kind else {
                    return None;
                };
                match self.model.item(*def, ctx)? {
                    ItemSlot::Assembly(slot) => slot.element(*position),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn try_mirror(&mut self, mirror: PendingMirror) {
        let Some(AssemblyKind::Mirror(def)) =
            self.model.defs.assembly(mirror.def).map(|a| a.kind.clone())
        else {
            return;
        };
        let Some(source) = self.mirror_source(&def.source, mirror.ctx) else {
            self.pending_mirrors.push(mirror);
            return;
        };
        let Some(component) = self
            .model
            .assemblies
            .get(source.index())
            .map(|a| a.component)
        else {
            return;
        };
        let name = self
            .model
            .defs
            .assembly(mirror.def)
            .map(|a| a.decl.name.clone())
            .unwrap_or_default();
        let full = element_name(&mirror.prefix, &name, None);
        let id = self.new_assembly(
            mirror.def,
            mirror.ctx,
            full,
            mirror.parent,
            component,
            Some(source),
        );
        if let Some(instance) = self.model.instances.get_mut(mirror.ctx.instance.index()) {
            instance.items.insert(
                DefId::Assembly(mirror.def),
                ItemSlot::Assembly(Slot::One(id)),
            );
        }
        let _ = mirror.elements;
    }

    fn report_unresolved_mirror(&mut self, mirror: &PendingMirror) {
        let Some(AssemblyKind::Mirror(def)) =
            self.model.defs.assembly(mirror.def).map(|a| a.kind.clone())
        else {
            return;
        };
        if def.source.kind == RExprKind::Error {
            return;
        }
        let Some((file, _)) = self.file_of(DefId::Assembly(mirror.def)) else {
            return;
        };
        self.diagnostics.push(
            Diagnostic::error(
                Code::TypeMismatch,
                "'mirror' needs an assembly declared in this module",
                Label::new(file, def.source.span),
            )
            .with_help("mirror an assembly by its name, or one element of an assembly array: mirror(drawer[top], about: ...)"),
        );
    }
}

fn element_name(prefix: &str, name: &str, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("{prefix}{name}[{label}]"),
        None => format!("{prefix}{name}"),
    }
}

fn slot_sources<T>(
    slot: Option<&ItemSlot>,
    pick: impl Fn(&ItemSlot) -> Option<Vec<T>>,
) -> Option<Vec<T>> {
    slot.and_then(pick)
}
