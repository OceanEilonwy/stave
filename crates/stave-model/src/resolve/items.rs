//! Lowering declarations other than kinds and recipes.

use std::collections::BTreeMap;

use stave_lang::ast::BaseKind;
use stave_lang::ast::{self, ExprKind, Span};
use stave_lang::{Code, Diagnostic, Label};
use stave_modules::{Module, ModuleHeader};
use stave_schema::fields::{self, FieldSpec, Requirement};
use stave_schema::functions::Builtin;
use stave_schema::kinds::RoleType;
use stave_schema::placement::PlacementTerm;
use stave_schema::words::{Grain, OutputUnit, StockForm, WordSet};

use super::expr::Expect;
use super::{Body, Cx, Origin, Resolver, Syntax};
use crate::defs::{
    ArrayDef, AssemblyDef, AssemblyKind, DefId, Domain, HardwareDef, JointDef, KindCallDef,
    LibraryDef, MirrorDef, PartBodyDef, PartDef, PartKind, PlaceTerm, PlaceTermKind, PoseDef,
    ProfileDef, ProjectDef, RExpr, RExprKind, RegionDef, RequireDef, SceneDef, Setting, Size,
    StockDef, StockSectionDef, Symbol, Word,
};
use crate::ids::{KindId, RecipeId};
use crate::types::{KindTy, Ty};

/// The arguments of a call fixed by position, split and checked.
pub(crate) struct CallArgs<'c> {
    pub positional: Vec<&'c ast::Expr>,
    pub named: Vec<(&'c ast::Name, &'c ast::Expr)>,
}

impl CallArgs<'_> {
    pub fn get(&self, name: &str) -> Option<&ast::Expr> {
        self.named
            .iter()
            .find(|(n, _)| n.as_str() == name)
            .map(|(_, e)| *e)
    }
}

/// `true` if one of `fields` is not in `table` and is a misspelling of
/// `wanted`: it is already reported as unknown, with a suggestion, so
/// `wanted` must not also be reported missing.
pub(crate) fn misspells<'f>(
    fields: impl IntoIterator<Item = &'f ast::Field>,
    table: &'static [FieldSpec],
    wanted: &str,
) -> bool {
    fields.into_iter().any(|field| {
        let key = field.key.as_str();
        fields::find(table, key).is_none() && !crate::suggest::closest(key, [wanted]).is_empty()
    })
}

/// Converts a `path` (as in `pose drawer[top] = open`) to an expression.
pub(crate) fn path_expr(path: &ast::Path) -> ast::Expr {
    let mut expr = ast::Expr {
        kind: ExprKind::Name(path.root.clone()),
        span: path.root.span(),
    };
    for segment in &path.segments {
        // The span is taken first so `expr` can move into the new node.
        let start = expr.span;
        let (kind, end) = match segment {
            ast::PathSegment::Member(name) => (
                ExprKind::Member {
                    base: Box::new(expr),
                    member: name.clone(),
                },
                name.span(),
            ),
            ast::PathSegment::Index(index) => {
                let end = match index {
                    ast::Index::All(span) | ast::Index::Same(span) => *span,
                    ast::Index::At(at) => at.span,
                };
                (
                    ExprKind::Index {
                        base: Box::new(expr),
                        index: index.clone(),
                    },
                    end,
                )
            }
        };
        expr = ast::Expr {
            span: start.to(end),
            kind,
        };
    }
    expr
}

impl<'a> Resolver<'a> {
    /// Splits call arguments, reporting unknown names, duplicates and too
    /// many positional arguments.
    pub(crate) fn split_args<'c>(
        &mut self,
        cx: &Cx,
        head: &str,
        args: impl IntoIterator<Item = &'c ast::Arg>,
        named: &[&str],
        positional_max: usize,
    ) -> CallArgs<'c> {
        let mut out = CallArgs {
            positional: Vec::new(),
            named: Vec::new(),
        };
        for arg in args {
            match &arg.name {
                None => {
                    if out.positional.len() < positional_max {
                        out.positional.push(&arg.value);
                    } else {
                        let help = if named.is_empty() {
                            format!("'{head}' takes {positional_max} argument(s)")
                        } else {
                            format!("name it: '{head}' takes {}", named.join(":, ") + ":")
                        };
                        self.error_help(
                            Code::TypeMismatch,
                            format!("'{head}' takes no more arguments without a name"),
                            cx.file,
                            arg.span,
                            help,
                        );
                    }
                }
                Some(name) => {
                    if !named.contains(&name.as_str()) {
                        let help = if named.is_empty() {
                            format!("'{head}' takes no named arguments")
                        } else {
                            format!("'{head}' takes {}", named.join(", "))
                        };
                        self.unknown(
                            Code::UnknownField,
                            format!("'{head}' has no field '{}'", name.as_str()),
                            cx.file,
                            name.span(),
                            name.as_str(),
                            named.iter().copied(),
                            help,
                        );
                    } else if out.named.iter().any(|(n, _)| n.as_str() == name.as_str()) {
                        self.error_help(
                            Code::DuplicateName,
                            format!("'{}' is declared twice", name.as_str()),
                            cx.file,
                            name.span(),
                            format!("give '{}' once; remove one of them", name.as_str()),
                        );
                    } else {
                        out.named.push((name, &arg.value));
                    }
                }
            }
        }
        out
    }

    /// Checks fields against a table: unknown fields and duplicates are
    /// reported, and missing required ones. Returns the fields by name.
    pub(crate) fn field_table<'f>(
        &mut self,
        cx: &Cx,
        fields_in: impl IntoIterator<Item = &'f ast::Field>,
        table: &'static [FieldSpec],
        what: &str,
        whole: Span,
    ) -> BTreeMap<&'f str, &'f ast::Field> {
        let mut found: BTreeMap<&'f str, &'f ast::Field> = BTreeMap::new();
        let mut misspelt: Vec<&str> = Vec::new();
        for field in fields_in {
            let name = field.key.as_str();
            if fields::find(table, name).is_none() {
                let known: Vec<&str> = table.iter().map(|s| s.name).collect();
                misspelt.push(name);
                self.unknown(
                    Code::UnknownField,
                    format!("{what} has no field '{name}'"),
                    cx.file,
                    field.key.span(),
                    name,
                    known.iter().copied(),
                    format!("the fields are: {}", known.join(", ")),
                );
                continue;
            }
            if let Some(first) = found.get(name) {
                self.report(
                    Diagnostic::error(
                        Code::DuplicateName,
                        format!("'{name}' is declared twice"),
                        Label::new(cx.file, field.key.span()),
                    )
                    .with_label(Label::with_message(
                        cx.file,
                        first.key.span(),
                        "first written here",
                    ))
                    .with_help(format!("write '{name}' once; remove one of them")),
                );
                continue;
            }
            found.insert(name, field);
        }
        for spec in table {
            let misspelling = misspelt
                .iter()
                .any(|m| !crate::suggest::closest(m, [spec.name]).is_empty());
            if spec.requirement == Requirement::Required
                && !found.contains_key(spec.name)
                && !misspelling
            {
                self.error_help(
                    Code::MissingField,
                    format!("{what} is missing '{}'", spec.name),
                    cx.file,
                    whole,
                    format!("add {}: ...", spec.name),
                );
            }
        }
        found
    }

    pub(crate) fn field_value(
        &mut self,
        cx: &Cx,
        field: &ast::Field,
        spec_table: &'static [FieldSpec],
    ) -> RExpr {
        let expect = fields::find(spec_table, field.key.as_str())
            .map(|spec| Expect::value_type(spec.ty))
            .unwrap_or_default()
            .named(field.key.as_str());
        self.value(cx, &field.value, &expect)
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "takes only the values it names: any other, a new variant included, is a type error"
    )]
    fn word_field(&mut self, cx: &Cx, field: &ast::Field, set: WordSet) -> Option<Word> {
        let value = self.value(
            cx,
            &field.value,
            &Expect::ty(Ty::Word(set)).named(field.key.as_str()),
        );
        match value.kind {
            RExprKind::Word(word) => Some(word),
            RExprKind::Error => None,
            _ => {
                self.error_help(
                    Code::TypeMismatch,
                    format!(
                        "'{}' needs {}, found {}",
                        field.key.as_str(),
                        set.description(),
                        value.ty
                    ),
                    cx.file,
                    value.span,
                    format!("write one of: {}", set.words().join(", ")),
                );
                None
            }
        }
    }

    // -----------------------------------------------------------------------
    // Project and library
    // -----------------------------------------------------------------------

    pub(super) fn lower_header(
        &mut self,
        module: &'a Module,
    ) -> (Option<ProjectDef>, Option<LibraryDef>) {
        let Some(header) = &module.header else {
            return (None, None);
        };
        let (file, index, is_project) = match header {
            ModuleHeader::Project { file, item } => (*file, *item, true),
            ModuleHeader::Library { file, item } => (*file, *item, false),
        };
        let Some(item) = module
            .files
            .iter()
            .find(|f| f.file == file)
            .and_then(|f| f.ast.as_ref())
            .and_then(|ast| ast.items.get(index))
        else {
            return (None, None);
        };
        let cx = Cx {
            module: module.id,
            file,
            body: super::Body::Module,
            array: None,
        };
        match (&item.kind, is_project) {
            (ast::ItemKind::Project(project), true) => {
                let title = self.text(&cx, &project.title).literal();
                let found = self.field_table(
                    &cx,
                    &project.body.fields,
                    fields::PROJECT,
                    "the project",
                    project.body.span,
                );
                let goal = found
                    .get("goal")
                    .and_then(|f| self.plain_text(&cx, &f.value, "goal"));
                let style = found
                    .get("style")
                    .and_then(|f| self.plain_text(&cx, &f.value, "style"));
                let units = match found.get("units") {
                    Some(field) => match self.word_field(&cx, field, WordSet::OutputUnit) {
                        Some(Word::OutputUnit(unit)) => unit,
                        _ => OutputUnit::Millimetre,
                    },
                    None => OutputUnit::Millimetre,
                };
                let resolution = found
                    .get("resolution")
                    .map(|f| self.field_value(&cx, f, fields::PROJECT));
                let tools = found.get("tools").map(|f| {
                    let value = self.field_value(&cx, f, fields::PROJECT);
                    kind_ids(&value)
                });
                let methods = found
                    .get("methods")
                    .map(|f| {
                        let value = self.field_value(&cx, f, fields::PROJECT);
                        recipe_ids(&value)
                    })
                    .unwrap_or_default();
                let boards = found
                    .get("boards")
                    .map(|f| self.field_value(&cx, f, fields::PROJECT));
                let mut length = |name: &str| {
                    found
                        .get(name)
                        .map(|f| self.field_value(&cx, f, fields::PROJECT))
                };
                let kerf = length("kerf");
                let rough_length = length("rough_length");
                let rough_width = length("rough_width");
                (
                    Some(ProjectDef {
                        title,
                        goal,
                        units,
                        resolution,
                        style,
                        tools,
                        methods,
                        boards,
                        kerf,
                        rough_length,
                        rough_width,
                        file,
                        span: item.span,
                    }),
                    None,
                )
            }
            (ast::ItemKind::Library(library), false) => {
                let title = self.text(&cx, &library.title).literal();
                let (mut about, mut license, mut boards) = (None, None, None);
                if let Some(body) = &library.body {
                    let found = self.field_table(
                        &cx,
                        &body.fields,
                        fields::LIBRARY,
                        "the library",
                        body.span,
                    );
                    about = found
                        .get("about")
                        .and_then(|f| self.plain_text(&cx, &f.value, "about"));
                    license = found
                        .get("license")
                        .and_then(|f| self.plain_text(&cx, &f.value, "license"));
                    boards = found
                        .get("boards")
                        .map(|f| self.field_value(&cx, f, fields::LIBRARY));
                }
                (
                    None,
                    Some(LibraryDef {
                        title,
                        about,
                        license,
                        boards,
                        file,
                    }),
                )
            }
            _ => (None, None),
        }
    }

    // -----------------------------------------------------------------------
    // Params, lets, requires
    // -----------------------------------------------------------------------

    /// The type and default of a param (module, component or kind).
    pub(crate) fn lower_param_spec(&mut self, cx: &Cx, param: &ast::Param) -> (Ty, Option<RExpr>) {
        let declared = param.spec.ty().map(|name| match ast::TypeName::from_word(name.as_str()) {
            Some(ty) => Ty::from_type_name(ty),
            None => {
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{}'", name.as_str()),
                    cx.file,
                    name.span(),
                    name.as_str(),
                    ast::TypeName::ALL.map(ast::TypeName::word),
                    "a param's type is one of Length, Angle, Ratio, Count, Bool, Text, Part, Parts, Stock, Region, Profile, Anchor, Axis, Direction, Mount, Shape, Envelope, Model, Spacing, JointKind, HardwareKind, SlideKind, HingeKind, PullKind, SupportKind, FittingKind, FastenerKind, Tool, Recipe",
                );
                Ty::Unknown
            }
        });
        let default = param.spec.default().map(|default| {
            let expect = match &declared {
                Some(ty) => Expect::ty(ty.clone()).named(param.name.as_str()),
                None => Expect::any(),
            };
            self.value(cx, default, &expect)
        });
        let ty = match (declared, &default) {
            (Some(ty), _) => ty,
            (None, Some(default)) => default.ty.clone(),
            (None, None) => Ty::Unknown,
        };
        (ty, default)
    }

    pub(crate) fn lower_domain(&mut self, cx: &Cx, domain: &ast::Domain, ty: &Ty) -> Domain {
        let expect = Expect::ty(ty.clone()).named("within");
        match domain {
            ast::Domain::Range { lo, hi, span } => {
                if !matches!(
                    ty,
                    Ty::Length | Ty::Angle | Ty::Ratio | Ty::Count | Ty::Unknown
                ) {
                    self.error(
                        Code::TypeMismatch,
                        format!("'within' needs a range of numbers, found a range for {ty}"),
                        cx.file,
                        *span,
                    );
                }
                // Both ends of the wrong unit are one mistake: the high
                // end's diagnostic is dropped when it repeats the low end's.
                let before = self.diagnostics.len();
                let low = self.value(cx, lo, &expect);
                let low_found: Vec<(Code, String)> = self
                    .diagnostics
                    .get(before..)
                    .unwrap_or_default()
                    .iter()
                    .map(|d| (d.code, d.message.clone()))
                    .collect();
                let between = self.diagnostics.len();
                let high = self.value(cx, hi, &expect);
                let mut index = between;
                while index < self.diagnostics.len() {
                    let repeated = self.diagnostics.get(index).is_some_and(|d| {
                        low_found
                            .iter()
                            .any(|(code, message)| *code == d.code && *message == d.message)
                    });
                    if repeated {
                        self.diagnostics.remove(index);
                    } else {
                        index += 1;
                    }
                }
                Domain::Range {
                    low,
                    high,
                    span: *span,
                }
            }
            ast::Domain::OneOf { values, span } => Domain::OneOf {
                values: values.iter().map(|v| self.value(cx, v, &expect)).collect(),
                span: *span,
            },
        }
    }

    pub(super) fn lower_require(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Require,
        origin: Origin<'_>,
    ) -> RequireDef {
        let condition = self.value(
            cx,
            &syntax.condition,
            &Expect::ty(Ty::Bool).named(syntax.name.as_str()),
        );
        let message = self.text(cx, &syntax.message);
        RequireDef {
            decl: self.decl(def, &syntax.name, origin),
            condition,
            message,
        }
    }

    /// An item's array: its labels, or its count. A single bracketed name
    /// that names nothing is one label, which isn't an array (grammar,
    /// "Arrays"): `error[type-mismatch]`, and the item is not an array.
    pub(crate) fn lower_array(
        &mut self,
        cx: &Cx,
        name: &ast::Ident,
        array: Option<&ast::ArraySpec>,
    ) -> Option<ArrayDef> {
        if let Some(ast::ArraySpec::Count(count)) = array
            && let ast::ExprKind::Name(label) = &count.kind
            && self
                .lookup(&Cx { array: None, ..*cx }, label.as_str())
                .is_none()
        {
            let (name, label) = (name.as_str(), label.as_str());
            self.error_help(
                Code::TypeMismatch,
                format!("one label isn't an array: '{label}' is the only label of '{name}'"),
                cx.file,
                count.span,
                format!("name it '{name}_{label}' instead, or give two or more labels, as in {name}[{label}, other]"),
            );
            return None;
        }
        Some(match array? {
            ast::ArraySpec::Labels(labels) => ArrayDef::Labels(
                labels
                    .as_slice()
                    .iter()
                    .map(|l| (l.as_str().to_owned(), l.span()))
                    .collect(),
            ),
            ast::ArraySpec::Count(count) => {
                let outer = Cx { array: None, ..*cx };
                ArrayDef::Count(self.value(&outer, count, &Expect::ty(Ty::Count).named("count")))
            }
        })
    }

    // -----------------------------------------------------------------------
    // Regions, stocks, profiles
    // -----------------------------------------------------------------------

    pub(super) fn lower_region(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Region,
        origin: Origin<'_>,
    ) -> RegionDef {
        let array = self.lower_array(cx, &syntax.name, syntax.array.as_ref());
        let value = match self.whole_region_box(cx, &syntax.value) {
            Some(args) if array.is_none() => {
                let value = self.region_box(cx, args, syntax.value.span);
                if let Body::Component(_) = cx.body {
                    // A component is handed the space it fills; the root
                    // belongs to the project.
                    self.error_help(
                        Code::Ground,
                        format!(
                            "'{}' is made with 'box' inside a component",
                            syntax.name.as_str()
                        ),
                        cx.file,
                        syntax.value.span,
                        "give the component a Region param for the space it fills, and hand it that space in the assembly; only the project has a root box",
                    );
                }
                value
            }
            _ => self.value(
                cx,
                &syntax.value,
                &Expect::any().named(syntax.name.as_str()),
            ),
        };
        match (&value.ty, array.is_some()) {
            (Ty::Region | Ty::Unknown, false) | (Ty::Regions | Ty::Unknown, true) => {}
            (Ty::Regions, false) => self.error_help(
                Code::TypeMismatch,
                format!("'{}' needs a region, found regions", syntax.name.as_str()),
                cx.file,
                value.span,
                format!(
                    "declare an array to hold them: region {}[a, b] = ...",
                    syntax.name.as_str()
                ),
            ),
            (Ty::Region, true) => self.error_help(
                Code::TypeMismatch,
                format!("'{}' needs regions, found a region", syntax.name.as_str()),
                cx.file,
                value.span,
                "only split(...) gives several regions",
            ),
            (other, _) => self.error(
                Code::TypeMismatch,
                format!("'{}' needs a region, found {other}", syntax.name.as_str()),
                cx.file,
                value.span,
            ),
        }
        RegionDef {
            decl: self.decl(def, &syntax.name, origin),
            array,
            value,
        }
    }

    /// The arguments of `box(across:, up:, deep:)` when it is the whole of
    /// `value` and `box` is the built-in, not an item of that name.
    fn whole_region_box<'e>(&self, cx: &Cx, value: &'e ast::Expr) -> Option<&'e [ast::Arg]> {
        let ast::ExprKind::Call { callee, args } = &value.kind else {
            return None;
        };
        let ast::ExprKind::Name(head) = &callee.kind else {
            return None;
        };
        let builtin = Builtin::from_word(head.as_str())?;
        (self.lookup(cx, head.as_str()).is_none()
            && builtin.for_arguments(
                args.iter()
                    .filter_map(|a| a.name.as_ref().map(|n| n.as_str())),
            ) == Builtin::RegionBox)
            .then_some(args.as_slice())
    }

    pub(super) fn lower_stock(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Stock,
        origin: Origin<'_>,
    ) -> StockDef {
        let what = format!("stock '{}'", syntax.name.as_str());
        let found = self.field_table(
            cx,
            &syntax.body.fields,
            fields::STOCK,
            &what,
            syntax.body.span,
        );
        let species = match found
            .get("species")
            .and_then(|f| self.word_field(cx, f, WordSet::Species))
        {
            Some(Word::Species(species)) => species,
            _ => stave_schema::species::Species::Pine,
        };
        let form = match found
            .get("form")
            .and_then(|f| self.word_field(cx, f, WordSet::StockForm))
        {
            Some(Word::StockForm(form)) => form,
            _ => StockForm::Solid,
        };
        let mut length = |name: &str| {
            found
                .get(name)
                .map(|field| self.field_value(cx, field, fields::STOCK))
        };
        let thickness = length("thickness");
        let diameter = length("diameter");
        let width = length("width");
        let max_width = length("max_width");
        let max_length = length("max_length");
        let sold = length("sold");
        let kerf = length("kerf");
        let rough_length = length("rough_length");
        let rough_width = length("rough_width");
        let max_held_width = length("max_held_width");
        // A sheet or a dowel doesn't move across a grain the way a solid
        // board does (grammar, "Grain and wood movement").
        if form != StockForm::Solid
            && let Some(written) = &max_held_width
        {
            let help = match form {
                StockForm::Plywood | StockForm::Mdf => format!(
                    "{} hardly swells or shrinks with the seasons, so it may be held fast at any width",
                    form.word()
                ),
                StockForm::Dowel | StockForm::Solid => {
                    "a dowel is too narrow to move much across its grain, so it may be held fast at any width"
                        .to_owned()
                }
            };
            self.error_help(
                Code::UnknownField,
                format!("{what} has no field 'max_held_width'"),
                cx.file,
                written.span,
                help,
            );
        }
        // A sheet or a dowel is bought at its finished size and cut
        // straight to it (grammar, "stock").
        if form != StockForm::Solid {
            for (name, written) in [
                ("sold", &sold),
                ("rough_length", &rough_length),
                ("rough_width", &rough_width),
            ] {
                if let Some(written) = written {
                    self.error_help(
                        Code::UnknownField,
                        format!("{what} has no field '{name}'"),
                        cx.file,
                        written.span,
                        format!(
                            "{} is bought at its finished size and cut straight to it, so it has no rough size",
                            form.word()
                        ),
                    );
                }
            }
        }
        let misspelt = |name| misspells(&syntax.body.fields, fields::STOCK, name);
        let section = if form == StockForm::Dowel {
            for (name, written) in [("thickness", &thickness), ("width", &width)] {
                if let Some(written) = written {
                    self.error_help(
                        Code::UnknownField,
                        format!("{what} has no field '{name}'"),
                        cx.file,
                        written.span,
                        "dowel stock is round: its diameter is its thickness and its width",
                    );
                }
            }
            if diameter.is_none() && found.contains_key("form") && !misspelt("diameter") {
                self.error_help(
                    Code::MissingField,
                    format!("{what} is missing 'diameter'"),
                    cx.file,
                    syntax.body.span,
                    "dowel stock is sized by diameter: ...",
                );
            }
            diameter.map(|diameter| StockSectionDef::Dowel { diameter })
        } else {
            if found.contains_key("form") {
                if thickness.is_none() && !misspelt("thickness") {
                    self.error_help(
                        Code::MissingField,
                        format!("{what} is missing 'thickness'"),
                        cx.file,
                        syntax.body.span,
                        "add thickness: ...",
                    );
                }
                if let Some(diameter) = &diameter {
                    self.error_help(
                        Code::UnknownField,
                        format!("{what} has no field 'diameter'"),
                        cx.file,
                        diameter.span,
                        "only dowel stock has a diameter",
                    );
                }
            }
            thickness.map(|thickness| StockSectionDef::Board { thickness, width })
        };
        StockDef {
            decl: self.decl(def, &syntax.name, origin),
            species,
            form,
            section,
            max_width,
            max_length,
            sold: sold.filter(|_| form == StockForm::Solid),
            kerf,
            rough_length: rough_length.filter(|_| form == StockForm::Solid),
            rough_width: rough_width.filter(|_| form == StockForm::Solid),
            max_held_width: max_held_width.filter(|_| form == StockForm::Solid),
        }
    }

    pub(super) fn lower_profile(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Profile,
        origin: Origin<'_>,
    ) -> ProfileDef {
        let what = format!("profile '{}'", syntax.name.as_str());
        let field_list: Vec<&ast::Field> = syntax
            .entries
            .iter()
            .filter_map(|e| match e {
                ast::ProfileEntry::Field(f) => Some(f),
                ast::ProfileEntry::Station { .. } => None,
            })
            .collect();
        let found = self.field_table(cx, field_list, fields::PROFILE, &what, syntax.span);
        let measure = match found
            .get("measure")
            .and_then(|f| self.word_field(cx, f, WordSet::Measure))
        {
            Some(Word::Measure(m)) => m,
            _ => stave_schema::words::Measure::Diameter,
        };
        let smooth = match found
            .get("smooth")
            .map(|f| self.field_value(cx, f, fields::PROFILE))
        {
            Some(RExpr {
                kind: RExprKind::Bool(b),
                ..
            }) => b,
            _ => false,
        };
        let mut stations = Vec::new();
        for entry in &syntax.entries {
            if let ast::ProfileEntry::Station { at, value, .. } = entry {
                let at = self.value(cx, at, &Expect::ty(Ty::Length).named("at"));
                let value = self.value(cx, value, &Expect::ty(Ty::Length).named("station"));
                stations.push((at, value));
            }
        }
        if stations.len() < 2 {
            self.error_help(
                Code::MissingField,
                format!("{what} is missing 'at'"),
                cx.file,
                syntax.span,
                "a profile needs two or more stations: at <distance>: <size>",
            );
        }
        ProfileDef {
            decl: self.decl(def, &syntax.name, origin),
            measure,
            smooth,
            stations,
        }
    }

    // -----------------------------------------------------------------------
    // Parts
    // -----------------------------------------------------------------------

    pub(super) fn lower_part(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Part,
        origin: Origin<'_>,
    ) -> PartDef {
        let array = self.lower_array(cx, &syntax.name, syntax.array.as_ref());
        let what = format!("part '{}'", syntax.name.as_str());
        let kind = match &syntax.def {
            ast::PartDef::FromStock { stock, body } => {
                let (mut lowered, place) = self.lower_part_body(cx, stock, body, &what);
                match place {
                    Some(place) => {
                        lowered.place = self.lower_place(cx, place, false);
                        lowered.place_span = Some(place.span);
                    }
                    None => self.error_help(
                        Code::MissingField,
                        format!("{what} is missing 'place'"),
                        cx.file,
                        body.span,
                        "add place { ... } with one term per axis",
                    ),
                }
                PartKind::FromStock(Box::new(lowered))
            }
            ast::PartDef::Mirror(mirror) => {
                PartKind::Mirror(self.lower_mirror(cx, mirror, &Ty::Part))
            }
        };
        PartDef {
            decl: self.decl(def, &syntax.name, origin),
            array,
            kind,
        }
    }

    pub(crate) fn lower_mirror(
        &mut self,
        cx: &Cx,
        mirror: &ast::Mirror,
        source_ty: &Ty,
    ) -> MirrorDef {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "the variants it does not name take the general path written last"
        )]
        let expect = match source_ty {
            Ty::Part => Expect::ty(Ty::Part).named("mirror"),
            _ => Expect::any().named("mirror"),
        };
        let source = self.value(cx, &mirror.source, &expect);
        if *source_ty != Ty::Part && !matches!(source.ty, Ty::Assembly(_) | Ty::Unknown) {
            self.error(
                Code::TypeMismatch,
                format!("'mirror' needs an assembly, found {}", source.ty),
                cx.file,
                source.span,
            );
        }
        let about = self.value(cx, &mirror.about, &Expect::ty(Ty::Anchor).named("about"));
        MirrorDef {
            source,
            about,
            span: mirror.span,
        }
    }

    /// The fields of a part body; the `place` block is returned unlowered.
    pub(crate) fn lower_part_body<'b>(
        &mut self,
        cx: &Cx,
        stock: &ast::Ref,
        body: &'b ast::PartBody,
        what: &str,
    ) -> (PartBodyDef, Option<&'b ast::PlaceBlock>) {
        let stock_expr = self.reference(cx, stock, &Expect::ty(Ty::Stock).named("stock"));
        let found = self.field_table(cx, body.fields(), fields::PART, what, body.span);
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "the variants it does not name take the general path written last"
        )]
        let size = |this: &mut Self, field: &ast::Field| -> Size {
            let value = this.field_value(cx, field, fields::PART);
            match value.kind {
                RExprKind::Fit => Size::Fit(value.span),
                _ => Size::Expr(value),
            }
        };
        let length = match found.get("length") {
            Some(field) => size(self, field),
            None => Size::Expr(RExpr::error(body.span)),
        };
        let width = found.get("width").map(|field| size(self, field));
        if width.is_none()
            && !self.stock_gives_width(&stock_expr)
            && !misspells(body.fields(), fields::PART, "width")
        {
            self.error_help(
                Code::MissingField,
                format!("{what} is missing 'width'"),
                cx.file,
                body.span,
                "add width: ..., or cut it from stock with a fixed width",
            );
        }
        let thickness = found
            .get("thickness")
            .map(|f| self.field_value(cx, f, fields::PART));
        let grain = match found
            .get("grain")
            .and_then(|f| self.word_field(cx, f, WordSet::Grain))
        {
            Some(Word::Grain(grain)) => grain,
            _ => Grain::Length,
        };
        let shape = found
            .get("shape")
            .map(|f| self.field_value(cx, f, fields::PART));
        let face = |this: &mut Self, name: &str| {
            let field = found.get(name)?;
            match this.word_field(cx, field, WordSet::Face) {
                Some(Word::Face(face)) => Some((face, field.value.span)),
                _ => None,
            }
        };
        let face_side = face(self, "face_side");
        let face_edge = face(self, "face_edge");
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
        )]
        let note = found.get("note").and_then(|field| match &field.value.kind {
            ExprKind::String(string) => Some(self.text(cx, string)),
            _ => {
                self.plain_text(cx, &field.value, "note");
                None
            }
        });
        let boards = found
            .get("boards")
            .map(|f| self.field_value(cx, f, fields::PART));
        let lengths = found
            .get("lengths")
            .map(|f| self.field_value(cx, f, fields::PART));
        let loose = found
            .get("loose")
            .map(|f| self.field_value(cx, f, fields::PART));
        let edges = found
            .get("edges")
            .map(|f| self.lower_edges(cx, &f.value))
            .unwrap_or_default();
        let banding = found
            .get("banding")
            .and_then(|f| self.lower_banding(cx, &f.value));
        (
            PartBodyDef {
                stock: stock_expr,
                length,
                width,
                thickness,
                grain,
                shape,
                face_side,
                face_edge,
                note,
                boards,
                lengths,
                loose,
                edges,
                banding,
                place: Vec::new(),
                place_span: None,
                span: body.span,
            },
            body.place(),
        )
    }

    fn stock_gives_width(&self, stock: &RExpr) -> bool {
        let RExprKind::Symbol(Symbol::Def(def @ DefId::Stock(_))) = &stock.kind else {
            // A stock passed in as a param, or an error: decided when solved.
            return true;
        };
        let Some(Syntax::Stock(syntax)) = self.origins.get(def).map(|o| o.syntax) else {
            return true;
        };
        let is_dowel = syntax.body.field("form").is_some_and(
            |field| matches!(&field.value.kind, ExprKind::Name(name) if name.as_str() == "dowel"),
        );
        is_dowel || syntax.body.field("width").is_some()
    }

    /// Lowers the terms of a `place { }` block of a part or hardware.
    pub(crate) fn lower_place(
        &mut self,
        cx: &Cx,
        place: &ast::PlaceBlock,
        hardware: bool,
    ) -> Vec<PlaceTerm> {
        let mut terms = Vec::new();
        for call in &place.terms {
            let head = call.head.as_str();
            let Some(term) = PlacementTerm::from_word(head) else {
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{head}'"),
                    cx.file,
                    call.head.span(),
                    head,
                    PlacementTerm::ALL.map(PlacementTerm::word),
                    "a placement term is one of ground, against, flush, center, spaced, corners, orient",
                );
                continue;
            };
            if hardware && !term.applies_to_hardware() {
                self.error_help(
                    Code::TypeMismatch,
                    format!("'{head}' doesn't apply to hardware"),
                    cx.file,
                    call.span,
                    "hardware is placed on a face of its piece with center, flush, against or spaced",
                );
                continue;
            }
            let length = |name: &'static str| Expect::ty(Ty::Length).named(name);
            let anchor = |name: &'static str| Expect::ty(Ty::Anchor).named(name);
            // The term's own argument table, shared with the editor.
            let args = self.split_args(
                cx,
                head,
                &call.args,
                &term.named_arguments(),
                term.positional_count(),
            );
            let kind = match term {
                PlacementTerm::Ground => PlaceTermKind::Ground,
                PlacementTerm::Against | PlacementTerm::Flush => {
                    let extra = if term == PlacementTerm::Against {
                        "gap"
                    } else {
                        "inset"
                    };
                    let Some(face) = args.positional.first() else {
                        self.missing_argument(cx, head, "a face", call.span);
                        continue;
                    };
                    let face = self.value(cx, face, &anchor("face"));
                    let offset = args.get(extra).map(|e| self.value(cx, e, &length(extra)));
                    if term == PlacementTerm::Against {
                        PlaceTermKind::Against { face, gap: offset }
                    } else {
                        PlaceTermKind::Flush {
                            face,
                            inset: offset,
                        }
                    }
                }
                PlacementTerm::Center => {
                    if args.positional.is_empty() {
                        let (Some(from), Some(to)) = (args.get("from"), args.get("to")) else {
                            self.missing_argument(
                                cx,
                                head,
                                "an item and an axis, or from: and to:",
                                call.span,
                            );
                            continue;
                        };
                        PlaceTermKind::CenterBetween {
                            from: self.value(cx, from, &anchor("from")),
                            to: self.value(cx, to, &anchor("to")),
                        }
                    } else {
                        let [item, axis] = args.positional.as_slice() else {
                            self.missing_argument(cx, head, "an item and an axis", call.span);
                            continue;
                        };
                        let item = self.value(cx, item, &Expect::any().named("center"));
                        if !matches!(item.ty, Ty::Part | Ty::Region | Ty::Unknown) {
                            self.error(
                                Code::TypeMismatch,
                                format!("'center' needs a part or a region, found {}", item.ty),
                                cx.file,
                                item.span,
                            );
                        }
                        let axis = self.value(
                            cx,
                            axis,
                            &Expect::ty(Ty::Word(WordSet::Axis)).named("axis"),
                        );
                        PlaceTermKind::CenterOn { item, axis }
                    }
                }
                PlacementTerm::Spaced => {
                    let (Some(spacing), Some(from), Some(to)) =
                        (args.positional.first(), args.get("from"), args.get("to"))
                    else {
                        self.missing_argument(cx, head, "a spacing, from: and to:", call.span);
                        continue;
                    };
                    PlaceTermKind::Spaced {
                        spacing: self.value(
                            cx,
                            spacing,
                            &Expect::ty(Ty::Spacing)
                                .with_words(&[WordSet::Spacing])
                                .named("spacing"),
                        ),
                        from: self.value(cx, from, &anchor("from")),
                        to: self.value(cx, to, &anchor("to")),
                    }
                }
                PlacementTerm::Corners => {
                    let Some(part) = args.positional.first() else {
                        self.missing_argument(cx, head, "a part", call.span);
                        continue;
                    };
                    PlaceTermKind::Corners {
                        part: self.value(cx, part, &Expect::ty(Ty::Part).named("corners")),
                        inset: args
                            .get("inset")
                            .map(|e| self.value(cx, e, &length("inset"))),
                    }
                }
                PlacementTerm::Orient => {
                    let direction =
                        |name: &'static str| Expect::ty(Ty::Word(WordSet::Direction)).named(name);
                    let Some(length) = args.get("length") else {
                        self.missing_argument(cx, head, "length:", call.span);
                        continue;
                    };
                    PlaceTermKind::Orient {
                        length: self.value(cx, length, &direction("length")),
                        width: args
                            .get("width")
                            .map(|e| self.value(cx, e, &direction("width"))),
                    }
                }
            };
            terms.push(PlaceTerm {
                kind,
                span: call.span,
            });
        }
        terms
    }

    /// `'<head>' is missing '<name>'` for a named argument (`what` is
    /// written `name:`), or `'<head>' is missing <what>` for a positional one
    /// (`what` is `a part`).
    pub(crate) fn missing_argument(&mut self, cx: &Cx, head: &str, what: &str, span: Span) {
        let single = what
            .strip_suffix(':')
            .filter(|name| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let (message, help) = match single {
            Some(name) => (
                format!("'{head}' is missing '{name}'"),
                format!("write {head}(..., {name}: ...)"),
            ),
            None => (
                format!("'{head}' is missing {what}"),
                format!("write {head}(...) with {what}"),
            ),
        };
        self.error_help(Code::MissingField, message, cx.file, span, help);
    }

    // -----------------------------------------------------------------------
    // Joints, hardware and assemblies
    // -----------------------------------------------------------------------

    pub(super) fn lower_joint(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Joint,
        origin: Origin<'_>,
    ) -> JointDef {
        let array = self.lower_array(cx, &syntax.name, syntax.array.as_ref());
        let what = format!("joint '{}'", syntax.name.as_str());
        let call = self.lower_kind_call(
            cx,
            &syntax.call,
            Some(BaseKind::Joint),
            &what,
            syntax.name.as_str(),
        );
        JointDef {
            decl: self.decl(def, &syntax.name, origin),
            array,
            call,
        }
    }

    pub(super) fn lower_hardware(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Hardware,
        origin: Origin<'_>,
    ) -> HardwareDef {
        let array = self.lower_array(cx, &syntax.name, syntax.array.as_ref());
        let what = format!("hardware '{}'", syntax.name.as_str());
        let call = self.lower_kind_call(cx, &syntax.call, None, &what, syntax.name.as_str());
        let place_block = syntax.body.as_ref().and_then(|b| b.place.as_ref());
        let place = place_block
            .map(|p| self.lower_place(cx, p, true))
            .unwrap_or_default();
        HardwareDef {
            decl: self.decl(def, &syntax.name, origin),
            array,
            call,
            place,
            place_span: place_block.map(|p| p.span),
        }
    }

    /// A kind call of a joint (`base` = joint) or hardware (`base` = None).
    fn lower_kind_call(
        &mut self,
        cx: &Cx,
        call: &ast::KindCall,
        base: Option<BaseKind>,
        what: &str,
        declared: &str,
    ) -> KindCallDef {
        let item = what.split(' ').next().unwrap_or("kind");
        let expect = Expect::ty(Ty::Kind(KindTy { base, kind: None })).named(item);
        // `hardware knob : knob(...)`: the declaration's own name hides the
        // kind it means to use, so say that rather than a type mismatch.
        let kind = if call.kind.rest.is_empty() && call.kind.root.as_str() == declared {
            self.error_help(
                Code::TypeMismatch,
                format!("'{declared}' here is this {item} itself, not a kind"),
                cx.file,
                call.kind.span,
                format!(
                    "the {item} is also called '{declared}', which hides the kind '{declared}'; give the {item} another name"
                ),
            );
            RExpr {
                kind: RExprKind::Error,
                ty: Ty::Unknown,
                span: call.kind.span,
            }
        } else {
            self.reference(cx, &call.kind, &expect)
        };
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "picks out only the variants it names: any other, a new one included, is not one of them"
        )]
        let kind_ty = match &kind.ty {
            Ty::Kind(kind_ty) => Some(*kind_ty),
            _ => None,
        };
        let known = kind_ty.and_then(|k| k.kind);
        let roles: Vec<(String, RoleType, Span)> = match known {
            Some(id) => self.kind_role_specs(id),
            None => Vec::new(),
        };
        let mut positional = None;
        let mut named: Vec<Setting> = Vec::new();
        let mut misspelt: Vec<String> = Vec::new();
        let mut extra_positional = false;
        for arg in &call.args {
            match &arg.name {
                None => {
                    if positional.is_some() {
                        let help = match roles.get(1) {
                            Some((role, _, _)) => {
                                format!("name every other role and setting: {role}: ...")
                            }
                            None => "name every setting: depth: ...".to_owned(),
                        };
                        self.error_help(
                            Code::TypeMismatch,
                            "a kind call takes at most one argument without a name",
                            cx.file,
                            arg.span,
                            help,
                        );
                        extra_positional = true;
                        continue;
                    }
                    let expect = match roles.first() {
                        Some((name, ty, _)) => role_expect(*ty).named(name.clone()),
                        None if known.is_some() => {
                            self.error(
                                Code::TypeMismatch,
                                format!(
                                    "the kind '{}' has no roles",
                                    known
                                        .map(|k| self.kind_display(k, cx.file))
                                        .unwrap_or_default()
                                ),
                                cx.file,
                                arg.span,
                            );
                            continue;
                        }
                        None => Expect::any(),
                    };
                    positional = Some(self.value(cx, &arg.value, &expect));
                }
                Some(name) => {
                    if named.iter().any(|s| s.name == name.as_str())
                        || (roles.first().is_some_and(|(r, _, _)| r == name.as_str())
                            && positional.is_some())
                    {
                        self.error_help(
                            Code::DuplicateName,
                            format!("'{}' is declared twice", name.as_str()),
                            cx.file,
                            name.span(),
                            format!("give '{}' once; remove one of them", name.as_str()),
                        );
                        continue;
                    }
                    let expect = if let Some((_, ty, _)) =
                        roles.iter().find(|(r, _, _)| r == name.as_str())
                    {
                        role_expect(*ty).named(name.as_str())
                    } else {
                        match kind_ty {
                            Some(kind_ty) if known.is_some() => {
                                match self.setting_ty(cx, kind_ty, name) {
                                    Some(ty) => Expect::ty(ty).named(name.as_str()),
                                    None => {
                                        misspelt.push(name.as_str().to_owned());
                                        continue;
                                    }
                                }
                            }
                            _ => {
                                if name.as_str() == stave_schema::kinds::FACE_SETTING {
                                    Expect::ty(Ty::Word(WordSet::Face)).named("face")
                                } else {
                                    Expect::any().named(name.as_str())
                                }
                            }
                        }
                    };
                    let value = self.value(cx, &arg.value, &expect);
                    named.push(Setting {
                        name: name.as_str().to_owned(),
                        name_span: name.span(),
                        value,
                    });
                }
            }
        }
        // A misspelt argument already reported as unknown is not missing too.
        let misspelling = |wanted: &str| {
            misspelt
                .iter()
                .any(|m| !crate::suggest::closest(m, [wanted]).is_empty())
        };
        if let Some(id) = known {
            for (index, (role, _, _)) in roles.iter().enumerate() {
                // An extra argument without a name was most likely meant
                // for a role, and is already reported.
                let given = (index == 0 && positional.is_some())
                    || named.iter().any(|s| &s.name == role)
                    || misspelling(role)
                    || extra_positional;
                if !given {
                    self.error_help(
                        Code::MissingField,
                        format!("{what} is missing '{role}'"),
                        cx.file,
                        call.span,
                        format!("name the part for '{role}' in the call: {role}: ..."),
                    );
                }
            }
            for missing in self.required_kind_params(id) {
                if !named.iter().any(|s| s.name == missing) && !misspelling(&missing) {
                    self.error_help(
                        Code::MissingField,
                        format!("{what} is missing '{missing}'"),
                        cx.file,
                        call.span,
                        format!("give it in the call: {missing}: ..."),
                    );
                }
            }
        }
        KindCallDef {
            kind,
            positional,
            named,
            span: call.span,
        }
    }

    fn kind_role_specs(&self, kind: KindId) -> Vec<(String, RoleType, Span)> {
        match self.origins.get(&DefId::Kind(kind)).map(|o| o.syntax) {
            Some(Syntax::Kind(syntax)) => super::kinds::role_specs(syntax),
            _ => Vec::new(),
        }
    }

    fn required_kind_params(&self, kind: KindId) -> Vec<String> {
        match self.origins.get(&DefId::Kind(kind)).map(|o| o.syntax) {
            Some(Syntax::Kind(syntax)) => super::kinds::kind_params(syntax)
                .into_iter()
                .filter(|p| p.spec.default().is_none())
                .filter(|p| {
                    // A hardware entry declared as a param but also written as a field.
                    syntax.body.field(p.name.as_str()).is_none()
                })
                .map(|p| p.name.as_str().to_owned())
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn lower_assembly(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Assembly,
        origin: Origin<'_>,
    ) -> AssemblyDef {
        let array = self.lower_array(cx, &syntax.name, syntax.array.as_ref());
        let kind = match &syntax.def {
            ast::AssemblyDef::Instance { component, body } => {
                let symbol = self.lookup_ref(cx, component);
                let component_id = match symbol {
                    Some(Symbol::Def(DefId::Component(id))) => Some(id),
                    Some(other) => {
                        let found = self.symbol_ty(other);
                        self.error(
                            Code::TypeMismatch,
                            format!(
                                "'{}' needs a component, found {found}",
                                syntax.name.as_str()
                            ),
                            cx.file,
                            component.span,
                        );
                        None
                    }
                    None => {
                        // Report through the normal path for a good message.
                        self.reference(cx, component, &Expect::any());
                        None
                    }
                };
                match component_id {
                    Some(id) => {
                        let args = self.lower_assembly_args(cx, id, body, syntax.name.as_str());
                        AssemblyKind::Instance {
                            component: id,
                            args,
                        }
                    }
                    None => AssemblyKind::Invalid,
                }
            }
            ast::AssemblyDef::Mirror(mirror) => {
                AssemblyKind::Mirror(self.lower_mirror(cx, mirror, &Ty::Unknown))
            }
        };
        AssemblyDef {
            decl: self.decl(def, &syntax.name, origin),
            array,
            kind,
        }
    }

    fn lower_assembly_args(
        &mut self,
        cx: &Cx,
        component: crate::ids::ComponentId,
        body: &ast::FieldBlock,
        name: &str,
    ) -> Vec<(crate::ids::ParamDefId, RExpr)> {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "picks out only the variants it names: any other, a new one included, is not one of them"
        )]
        let params: Vec<(String, crate::ids::ParamDefId, bool)> = self
            .component_items
            .get(component.index())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| match item {
                DefId::Param(id) => {
                    let has_default = matches!(
                        self.origins.get(&item).map(|o| o.syntax),
                        Some(Syntax::Param(p)) if p.spec.default().is_some()
                    );
                    Some((self.def_name(item), id, has_default))
                }
                _ => None,
            })
            .collect();
        let component_name = self.def_name(DefId::Component(component));
        let mut args: Vec<(crate::ids::ParamDefId, RExpr)> = Vec::new();
        let mut seen: BTreeMap<String, Span> = BTreeMap::new();
        let mut misspelt: Vec<String> = Vec::new();
        for field in &body.fields {
            let key = field.key.as_str();
            // An argument for a param left out for a syntax error is
            // skipped, not unknown.
            if self.broken_in_component(component, key) {
                continue;
            }
            let Some((_, id, _)) = params.iter().find(|(n, _, _)| n == key) else {
                let candidates: Vec<String> = params.iter().map(|(n, _, _)| n.clone()).collect();
                misspelt.push(key.to_owned());
                self.unknown(
                    Code::UnknownField,
                    format!("the component '{component_name}' has no field '{key}'"),
                    cx.file,
                    field.key.span(),
                    key,
                    candidates,
                    format!(
                        "its params are: {}",
                        params
                            .iter()
                            .map(|(n, _, _)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
                continue;
            };
            if let Some(first) = seen.get(key) {
                self.report(
                    Diagnostic::error(
                        Code::DuplicateName,
                        format!("'{key}' is declared twice"),
                        Label::new(cx.file, field.key.span()),
                    )
                    .with_label(Label::with_message(cx.file, *first, "first written here"))
                    .with_help(format!("write '{key}' once; remove one of them")),
                );
                continue;
            }
            seen.insert(key.to_owned(), field.key.span());
            let ty = self.symbol_ty(Symbol::Def(DefId::Param(*id)));
            let value = self.value(cx, &field.value, &Expect::ty(ty).named(key));
            args.push((*id, value));
        }
        for (param, _, has_default) in &params {
            let misspelling = misspelt
                .iter()
                .any(|m| !crate::suggest::closest(m, [param.as_str()]).is_empty());
            if !has_default && !seen.contains_key(param) && !misspelling {
                self.error_help(
                    Code::MissingField,
                    format!("assembly '{name}' is missing '{param}'"),
                    cx.file,
                    body.span,
                    format!("the component '{component_name}' needs it: {param}: ..."),
                );
            }
        }
        args
    }

    // -----------------------------------------------------------------------
    // Scenes
    // -----------------------------------------------------------------------

    /// A scene's `show:` or `hide:`: parts, assemblies or hardware, one
    /// or a list. Anything else would draw nothing, so it is a
    /// `type-mismatch` here rather than an empty scene later.
    fn scene_items(&mut self, cx: &Cx, expr: &ast::Expr, field: &str) -> RExpr {
        let value = self.value(cx, expr, &Expect::any().named(field));
        if !is_scene_items(&value.ty) {
            self.error_help(
                Code::TypeMismatch,
                format!(
                    "'{field}' needs parts, assemblies or hardware, found {}",
                    value.ty
                ),
                cx.file,
                value.span,
                format!("name the parts, assemblies or hardware to {field}, or a list of them"),
            );
        }
        value
    }

    pub(super) fn lower_scene(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Scene,
        origin: Origin<'_>,
    ) -> SceneDef {
        let title = syntax.title.as_ref().map(|t| self.text(cx, t).literal());
        let what = format!("scene '{}'", syntax.name.as_str());
        let field_list: Vec<&ast::Field> = syntax
            .entries
            .iter()
            .filter_map(|e| match e {
                ast::SceneEntry::Field(f) => Some(f),
                ast::SceneEntry::Pose { .. } => None,
            })
            .collect();
        let found = self.field_table(cx, field_list, fields::SCENE, &what, syntax.span);
        let mut poses = Vec::new();
        for entry in &syntax.entries {
            if let ast::SceneEntry::Pose {
                target,
                value,
                span,
            } = entry
            {
                let target_expr = path_expr(target);
                let target = self.value(cx, &target_expr, &Expect::any().named("pose"));
                if !matches!(target.ty, Ty::Assembly(_) | Ty::Unknown) {
                    self.error_help(
                        Code::TypeMismatch,
                        format!("'pose' needs an assembly, found {}", target.ty),
                        cx.file,
                        target.span,
                        "only assemblies with hinges or slides move",
                    );
                }
                let value = self.value(
                    cx,
                    value,
                    &Expect::any().with_words(&[WordSet::Motion]).named("pose"),
                );
                if !matches!(
                    value.ty,
                    Ty::Word(WordSet::Motion) | Ty::Angle | Ty::Length | Ty::Unknown
                ) {
                    self.error(
                        Code::TypeMismatch,
                        format!(
                            "'pose' needs open, closed, an angle or a length, found {}",
                            value.ty
                        ),
                        cx.file,
                        value.span,
                    );
                }
                poses.push(PoseDef {
                    target,
                    value,
                    span: *span,
                });
            }
        }
        let show = found
            .get("show")
            .map(|f| self.scene_items(cx, &f.value, "show"));
        let hide = found
            .get("hide")
            .map(|f| self.scene_items(cx, &f.value, "hide"));
        let explode = found
            .get("explode")
            .map(|f| self.field_value(cx, f, fields::SCENE));
        let section = found
            .get("section")
            .map(|f| self.field_value(cx, f, fields::SCENE));
        SceneDef {
            decl: self.decl(def, &syntax.name, origin),
            title,
            poses,
            show,
            hide,
            explode,
            section,
        }
    }
}

/// `true` if a value of this type names things a scene can show or hide:
/// parts, assemblies, hardware, or a list of them.
fn is_scene_items(ty: &Ty) -> bool {
    match ty {
        Ty::Part | Ty::Parts | Ty::Assembly(_) | Ty::Hardware | Ty::Unknown => true,
        Ty::List(inner) | Ty::Array(inner) => is_scene_items(inner),
        Ty::Length
        | Ty::Angle
        | Ty::Ratio
        | Ty::Count
        | Ty::Bool
        | Ty::Text
        | Ty::Stock
        | Ty::Region
        | Ty::Regions
        | Ty::Profile
        | Ty::Anchor
        | Ty::Mount
        | Ty::Shape
        | Ty::Boards
        | Ty::Splices
        | Ty::Envelope
        | Ty::Model
        | Ty::Spacing
        | Ty::Patch
        | Ty::Word(_)
        | Ty::Kind(_)
        | Ty::Tool
        | Ty::Recipe
        | Ty::Joint
        | Ty::Fit => false,
    }
}

pub(crate) fn role_expect(ty: RoleType) -> Expect {
    match ty {
        RoleType::Part => Expect::ty(Ty::Part),
        RoleType::Parts => Expect::ty(Ty::Parts),
        RoleType::Region => Expect::ty(Ty::Region),
    }
}

/// The kinds named in a list expression (tools).
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "picks out only the variants it names: any other, a new one included, is not one of them"
)]
pub(crate) fn kind_ids(expr: &RExpr) -> Vec<KindId> {
    match &expr.kind {
        RExprKind::List(items) => items.iter().flat_map(kind_ids).collect(),
        RExprKind::Symbol(Symbol::Def(DefId::Kind(id))) => vec![*id],
        _ => Vec::new(),
    }
}

/// The recipes named in a list expression.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "picks out only the variants it names: any other, a new one included, is not one of them"
)]
pub(crate) fn recipe_ids(expr: &RExpr) -> Vec<RecipeId> {
    match &expr.kind {
        RExprKind::List(items) => items.iter().flat_map(recipe_ids).collect(),
        RExprKind::Symbol(Symbol::Def(DefId::Recipe(id))) => vec![*id],
        _ => Vec::new(),
    }
}
