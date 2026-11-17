//! Focused parser tests: recovery, newline rules at the grammar level,
//! string interpolation, contextual words and keywords used as names.

use stave_lang::ast::*;
use stave_lang::{Code, Parse, SourceMap, parse_file, parse_patch_file};

fn parse(text: &str) -> Parse<File> {
    let mut sources = SourceMap::new();
    let file = sources.add("test.stave", text).unwrap();
    let parse = parse_file(file, text);
    assert_eq!(parse.syntax().to_string(), text, "lossless");
    parse
}

fn clean(text: &str) -> File {
    let parse = parse(text);
    assert!(parse.diagnostics.is_empty(), "{:#?}", parse.diagnostics);
    parse.ast.unwrap()
}

fn messages(parse: &Parse<File>) -> Vec<String> {
    parse
        .diagnostics
        .iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect()
}

fn names(file: &File) -> Vec<&str> {
    file.items
        .iter()
        .filter_map(|item| item.kind.name())
        .map(Ident::as_str)
        .collect()
}

fn design(item: &Item) -> &DesignItem {
    match &item.kind {
        ItemKind::Design(design) => design,
        other => panic!("not a design item: {other:?}"),
    }
}

fn field<'a>(file: &'a File, part: &str, key: &str) -> &'a Expr {
    for item in &file.items {
        if let ItemKind::Design(DesignItem::Part(found)) = &item.kind
            && found.name.as_str() == part
            && let PartDef::FromStock { body, .. } = &found.def
        {
            return &body.field(key).expect("field").value;
        }
    }
    panic!("no part {part}");
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

#[test]
fn an_unclosed_bracket_costs_one_error() {
    let parse = parse(
        "stave 1\n\
         region front_opening = opening(\n\
         \x20 left: side_l.right,\n\
         \x20 top:  top.bottom\n\
         region bay[drawers, doors] = split(front_opening, up, at: divider)\n\
         part shelf : oak { length: 3mm }\n",
    );
    assert_eq!(
        messages(&parse),
        ["unclosed-bracket: this \"(\" is never closed"]
    );
    let file = parse.ast.unwrap();
    assert_eq!(names(&file), ["bay", "shelf"]);
    assert_eq!(parse.broken.len(), 1);
    assert_eq!(parse.broken[0].name.as_deref(), Some("front_opening"));
    assert_eq!(parse.broken[0].tag, Some(ItemTag::Region));
}

#[test]
fn a_missing_brace_ends_the_body_at_the_next_item() {
    let parse = parse(
        "stave 1\n\
         part side_l : oak {\n\
         \x20 length: 3mm\n\
         \x20 place {\n\
         \x20   ground\n\
         \x20 }\n\
         /// The top.\n\
         part top : oak {\n\
         \x20 length: 4mm\n\
         }\n",
    );
    assert_eq!(
        messages(&parse),
        ["missing-brace: the body of part 'side_l' is missing its closing \"}\""]
    );
    let diagnostic = &parse.diagnostics[0];
    assert_eq!(diagnostic.notes, ["the next declaration starts on line 7"]);
    assert_eq!(diagnostic.help.as_deref(), Some("add \"}\" before line 7"));
    let file = parse.ast.unwrap();
    assert_eq!(names(&file), ["top"]);
    assert_eq!(file.items[0].docs, ["The top."]);
}

#[test]
fn a_missing_brace_inside_a_component_body() {
    let parse = parse(
        "stave 1\n\
         component Drawer {\n\
         \x20 part front : s {\n\
         \x20   length: 1mm\n\
         \x20 part back : s { length: 2mm }\n\
         }\n\
         kind k : joint { }\n",
    );
    assert_eq!(
        messages(&parse),
        ["missing-brace: the body of part 'front' is missing its closing \"}\""]
    );
    let file = parse.ast.unwrap();
    assert_eq!(names(&file), ["Drawer", "k"]);
    let ItemKind::Component(component) = &file.items[0].kind else {
        panic!()
    };
    // Only the broken part is left out; the component and its other item stay.
    assert_eq!(component.items.len(), 1);
    assert_eq!(component.items[0].node.name().as_str(), "back");
    assert_eq!(parse.broken[0].name.as_deref(), Some("Drawer.front"));
}

#[test]
fn a_field_ends_at_its_line() {
    let parse = parse(
        "stave 1\n\
         part p : oak {\n\
         \x20 length: cabinet_height\n\
         \x20         - base.thickness\n\
         \x20 width: 2mm\n\
         }\n\
         let ok = 1\n",
    );
    assert_eq!(messages(&parse), ["syntax: expected a field, found '-'"]);
    assert_eq!(names(parse.ast.as_ref().unwrap()), ["ok"]);

    let file = clean(
        "stave 1\n\
         part p : oak {\n\
         \x20 length: cabinet_height -\n\
         \x20         base.thickness\n\
         }\n",
    );
    assert!(matches!(
        field(&file, "p", "length").kind,
        ExprKind::Binary {
            op: BinaryOp::Sub,
            ..
        }
    ));
}

#[test]
fn a_missing_value_is_reported_on_its_own_line() {
    for (text, message) in [
        (
            "part p : oak {\n  length:\n  width: 3mm\n}\n",
            "'length' has no value",
        ),
        ("part p : oak { length: }\n", "'length' has no value"),
        ("let overhang =\nlet b = 1\n", "'overhang' has no value"),
        ("param depth =\n", "'depth' has no value"),
        ("region r =\n", "'r' has no value"),
        ("part p = \n", "'p' has no value"),
        ("joint j : dado(a, depth: )\n", "'depth' has no value"),
    ] {
        let parse = parse(&format!("stave 1\n{text}"));
        assert_eq!(
            messages(&parse),
            [format!("missing-value: {message}")],
            "{text}"
        );
    }
}

#[test]
fn one_mistake_one_error() {
    for (text, message) in [
        (
            "require wide_enough shelf.width >= 250mm \"m\"\n",
            "syntax: expected ':', found 'shelf'",
        ),
        (
            "part joint : oak { }\n",
            "reserved-word: 'joint' is a reserved word and can't be used as a name",
        ),
        (
            "let x = 3inch + 2 * 3mm\n",
            "unknown-unit: unknown unit 'inch' (expected one of mm, cm, m, in, ft, deg)",
        ),
        (
            "let b = 1 < 2 < 3\n",
            "syntax: expected end of line, found '<'",
        ),
        (
            "joint j : dado(a) { depth: 3mm }\n",
            "syntax: expected end of line, found '{'",
        ),
        (
            "part x = foo(a)\n",
            "syntax: expected 'mirror', found 'foo'",
        ),
        (
            "part x = mirror(a, b)\n",
            "syntax: expected 'about:', found 'b'",
        ),
        (
            "let a = f(x: 1, 2)\n",
            "syntax: expected a named argument, found '2'",
        ),
        ("let a = f(1 2)\n", "syntax: expected ',' or ')', found '2'"),
        (
            "part a[x,] : s { }\n",
            "syntax: expected a second label, found ']'",
        ),
        (
            "kind k : knob { }\n",
            "syntax: expected 'joint' or 'slide' or 'hinge' or 'pull' or 'fastener' or 'tool', found 'knob'",
        ),
        (
            "part p : s {\n  place { ground }\n  place { ground }\n}\n",
            "syntax: expected a field, found 'place'",
        ),
        (
            "hardware h : knob(a) { length: 3mm }\n",
            "syntax: expected 'place', found 'length'",
        ),
        (
            "param p\n",
            "syntax: expected ':' or '=', found end of line",
        ),
        (
            "project \"P\" { goal: \"a {b}\" }\n",
            "syntax: expected plain text, found '{b}'",
        ),
        ("let a = 1 @\n", "syntax: expected end of line, found '@'"),
        (
            "part p : s {\n  /// doc\n  length: 3mm\n}\n",
            "syntax: expected a field, found a doc comment",
        ),
        (
            "let a = 1/0in\n",
            "syntax: expected a denominator other than zero, found '1/0in'",
        ),
    ] {
        let parse = parse(&format!("stave 1\n{text}let after = 1\n"));
        assert_eq!(messages(&parse), [message.to_owned()], "{text}");
        let file = parse.ast.unwrap();
        assert_eq!(names(&file).last(), Some(&"after"), "{text}");
    }
}

#[test]
fn a_missing_header_is_reported_and_items_still_parse() {
    let parse = parse("part p : s { length: 1mm }\n");
    assert_eq!(messages(&parse), ["syntax: expected 'stave', found 'part'"]);
    assert!(parse.ast.is_none());
}

#[test]
fn imports_come_first() {
    let parse = parse("stave 1\nlet a = 1\nimport \"./x\"\n");
    assert_eq!(
        messages(&parse),
        ["syntax: expected a declaration, found 'import'"]
    );
    let file = clean("stave 1\nimport \"./casework\"\nimport \"./hardware\" as hw\n\nlet a = 1\n");
    assert_eq!(file.imports.len(), 2);
    assert_eq!(
        file.imports[1].alias.as_ref().map(Ident::as_str),
        Some("hw")
    );
}

// ---------------------------------------------------------------------------
// Strings
// ---------------------------------------------------------------------------

#[test]
fn interpolation_parts_have_spans() {
    let text =
        "stave 1\nkind k : joint {\n  note: \"Halve {first} and {piece.thickness} \\{x}\"\n}\n";
    let file = clean(text);
    let ItemKind::Kind(kind) = &file.items[0].kind else {
        panic!()
    };
    let note = &kind.body.field("note").unwrap().value;
    let ExprKind::String(string) = &note.kind else {
        panic!()
    };
    let slice = |span: Span| &text[span.start() as usize..span.end() as usize];
    let parts: Vec<String> = string
        .parts
        .iter()
        .map(|part| match part {
            StringPart::Text {
                text: resolved,
                span,
            } => format!("text {resolved:?} at {:?}", slice(*span)),
            StringPart::Interp {
                root,
                members,
                span,
            } => {
                assert_eq!(slice(root.span()), root.as_str());
                for member in members {
                    assert_eq!(slice(member.span()), member.as_str());
                }
                format!(
                    "interp {}{} at {:?}",
                    root,
                    members.iter().map(|m| format!(".{m}")).collect::<String>(),
                    slice(*span)
                )
            }
        })
        .collect();
    assert_eq!(
        parts,
        [
            "text \"Halve \" at \"Halve \"",
            "interp first at \"{first}\"",
            "text \" and \" at \" and \"",
            "interp piece.thickness at \"{piece.thickness}\"",
            "text \" {x}\" at \" \\\\{x}\"",
        ]
    );
    assert_eq!(string.interpolations().count(), 2);
}

#[test]
fn interpolation_is_allowed_only_in_notes_checks_and_require_messages() {
    clean(
        "stave 1\nrequire r : a < b \"{a} is too big\"\nrecipe x : notch { check: \"{depth}\" }\n",
    );
    for text in [
        "import \"./{x}\"\n",
        "scene s \"{x}\" { }\n",
        "project \"{x}\" { }\n",
        "part p : s { shape: model(\"{x}\") }\n",
    ] {
        let parse = parse(&format!("stave 1\n{text}"));
        assert_eq!(
            messages(&parse),
            ["syntax: expected plain text, found '{x}'"],
            "{text}"
        );
        assert_eq!(
            parse.diagnostics[0].help.as_deref(),
            Some(
                "write \\{ for a literal brace; {name} works only in note:, check: and require messages"
            )
        );
    }
}

// ---------------------------------------------------------------------------
// Contextual words and keywords as names
// ---------------------------------------------------------------------------

#[test]
fn contextual_words_are_ordinary_names_elsewhere() {
    let file = clean(
        "stave 1\n\
         part cut : s { length: 1mm }\n\
         let same = as + to + in + mirror + about + roles + makes + mark + replace + unset + rename\n\
         let slide = hinge\n\
         part about : s { tool: pull, fastener: x[same], roles: y[as] }\n",
    );
    assert_eq!(names(&file), ["cut", "same", "slide", "about"]);
}

#[test]
fn contextual_words_in_their_positions() {
    let file = clean(
        "stave 1\n\
         import \"./hardware\" as hw\n\
         part side_r = mirror(side_l, about: mid(base, across))\n\
         assembly door_r = mirror(door_l, about: plane)\n\
         kind k : slide {\n\
         \x20 roles { piece: Part, in: Region }\n\
         \x20 roles: 3\n\
         \x20 cut end(piece, facing: in) { tenon(length: 3mm) }\n\
         \x20 cut: 4\n\
         \x20 cut dado: crossing(piece, in) { notch(depth: 1mm) }\n\
         \x20 mount { face: back, along: open }\n\
         \x20 mount: 7\n\
         \x20 makes part key : s { length: 1mm }\n\
         \x20 makes: 5\n\
         \x20 /// The cup.\n\
         \x20 member cup\n\
         \x20 member: 6\n\
         }\n\
         recipe r : notch {\n\
         \x20 mark { knife(walls) }\n\
         \x20 cut { saw(walls) }\n\
         \x20 mark: 1\n\
         }\n\
         assembly drawer[bottom, top] : casework.Drawer { opening: bay[same], all: x[*], one: x[same + 1] }\n",
    );
    assert_eq!(file.imports[0].alias.as_ref().unwrap().as_str(), "hw");
    let ItemKind::Kind(kind) = &file.items[2].kind else {
        panic!()
    };
    assert_eq!(kind.base.value, BaseKind::Slide);
    let entries: Vec<&str> = kind
        .body
        .entries
        .iter()
        .map(|entry| match entry {
            KindEntry::Roles(_) => "roles",
            KindEntry::Cut(cut) => match &cut.name {
                Some(name) => {
                    assert_eq!(name.as_str(), "dado");
                    "named cut"
                }
                None => "cut",
            },
            KindEntry::Mount(mount) => {
                assert_eq!(mount.fields.len(), 2);
                "mount"
            }
            KindEntry::Makes(_) => "makes",
            KindEntry::Member(member) => {
                assert_eq!(member.docs, ["The cup."]);
                assert_eq!(member.node.name.as_str(), "cup");
                "member"
            }
            KindEntry::Field(field) => field.key.as_str(),
            KindEntry::Mechanism(_) => "mechanism",
            KindEntry::Param(_) | KindEntry::Require(_) => "other",
        })
        .collect();
    assert_eq!(
        entries,
        [
            "roles",
            "roles",
            "cut",
            "cut",
            "named cut",
            "mount",
            "mount",
            "makes",
            "makes",
            "member",
            "member"
        ]
    );
    let ItemKind::Recipe(recipe) = &file.items[3].kind else {
        panic!()
    };
    assert!(matches!(recipe.body.entries[0], RecipeEntry::Mark(_)));
    assert!(matches!(recipe.body.entries[1], RecipeEntry::Cut(_)));
    assert!(matches!(recipe.body.entries[2], RecipeEntry::Field(_)));

    let DesignItem::Assembly(assembly) = design(&file.items[4]) else {
        panic!()
    };
    let AssemblyDef::Instance { body, .. } = &assembly.def else {
        panic!()
    };
    let index = |key: &str| match &body.field(key).unwrap().value.kind {
        ExprKind::Index { index, .. } => index.clone(),
        other => panic!("{other:?}"),
    };
    assert!(matches!(index("opening"), Index::Same(_)));
    assert!(matches!(index("all"), Index::All(_)));
    assert!(matches!(index("one"), Index::At(_)));
}

#[test]
fn members_with_bodies_shapes_and_every_mechanism_entry() {
    let file = clean(
        "stave 1\n\
         kind h : hinge {\n\
         \x20 /// The plate.\n\
         \x20 member plate\n\
         \x20 /// The arm.\n\
         \x20 member arm {\n\
         \x20   with:          on\n\
         \x20   shape body:    box(along: 16mm, across: -2.9mm..15mm, depth: -(s + 18mm)..0mm)\n\
         \x20   shape knuckle: box(along: 16mm, across: 8mm..15mm, depth: 0mm..12.4mm)\n\
         \x20 }\n\
         \x20 member cup { with: piece, shape: cylinder(diameter: 35mm, depth: 0mm..13mm) }\n\
         \x20 mechanism {\n\
         \x20   pivot arm, link at point(across: 13.5mm, depth: 10.9mm)\n\
         \x20   slot cup, link from point(depth: 1mm) to point(depth: 2mm)\n\
         \x20   rail cup, arm along along\n\
         \x20   couple cup to arm: 1/2\n\
         \x20   pin_diameter: 3mm\n\
         \x20   drive: turn(cup, from: 0deg, to: 110deg)\n\
         \x20   stop:  110deg \"the limiter stops it\"\n\
         \x20 }\n\
         }\n",
    );
    let ItemKind::Kind(kind) = &file.items[0].kind else {
        panic!()
    };
    let members: Vec<&Documented<KindMember>> = kind
        .body
        .entries
        .iter()
        .filter_map(|entry| match entry {
            KindEntry::Member(member) => Some(member),
            _ => None,
        })
        .collect();
    assert!(members[0].node.body.is_none());
    assert_eq!(members[1].docs, ["The arm."]);
    let arm = members[1].node.body.as_ref().unwrap();
    let MemberEntry::Field(with) = &arm.entries[0] else {
        panic!()
    };
    assert_eq!(with.key.as_str(), "with");
    let MemberEntry::Shape(body) = &arm.entries[1] else {
        panic!()
    };
    assert_eq!(body.name.as_ref().unwrap().as_str(), "body");
    let ExprKind::Call { args, .. } = &body.value.kind else {
        panic!()
    };
    assert!(matches!(args[1].value.kind, ExprKind::Range { .. }));
    assert!(matches!(args[2].value.kind, ExprKind::Range { .. }));
    // `shape:` is a field and gives the member's one unnamed shape.
    let cup = members[2].node.body.as_ref().unwrap();
    assert!(matches!(&cup.entries[1], MemberEntry::Shape(shape) if shape.name.is_none()));
    let Some(KindEntry::Mechanism(mechanism)) = kind.body.entries.last() else {
        panic!()
    };
    let entries: Vec<&str> = mechanism
        .entries
        .iter()
        .map(|entry| match entry {
            MechanismEntry::Pivot { members, .. } => {
                assert_eq!(members.each_ref().map(Ident::as_str), ["arm", "link"]);
                "pivot"
            }
            MechanismEntry::Slot { .. } => "slot",
            MechanismEntry::Rail { along, .. } => {
                assert_eq!(along.as_str(), "along");
                "rail"
            }
            MechanismEntry::Couple { member, to, .. } => {
                assert_eq!((member.as_str(), to.as_str()), ("cup", "arm"));
                "couple"
            }
            MechanismEntry::Drive { .. } => "drive",
            MechanismEntry::Stop { message, .. } => {
                assert!(matches!(&message.parts[..], [StringPart::Text { text, .. }] if text == "the limiter stops it"));
                "stop"
            }
            MechanismEntry::Field(field) => field.key.as_str(),
        })
        .collect();
    assert_eq!(
        entries,
        [
            "pivot",
            "slot",
            "rail",
            "couple",
            "pin_diameter",
            "drive",
            "stop"
        ]
    );
}

#[test]
fn a_mechanism_entry_missing_a_word_is_one_error() {
    let missing_at = parse(
        "stave 1\nkind h : hinge {\n  mechanism {\n    pivot arm, link point(across: 1mm)\n  }\n}\n",
    );
    assert_eq!(
        messages(&missing_at),
        ["syntax: expected 'at', found 'point'"]
    );
    let missing_message =
        parse("stave 1\nkind h : hinge {\n  mechanism {\n    stop: 110deg\n  }\n}\n");
    assert_eq!(
        messages(&missing_message),
        ["syntax: expected a string, found end of line"]
    );
}

#[test]
fn keywords_are_names_after_a_dot_and_before_a_colon() {
    let file = clean(
        "stave 1\n\
         kind dado : joint {\n\
         \x20 roles { piece: Parts, into: Parts, part: Part }\n\
         \x20 param depth = into.stock.thickness / 3\n\
         }\n\
         region bay[drawers, doors] = split(front, up, at: divider, place: x, stock: y)\n\
         part p : s { stock: x, place: y, at: 3, part: q, fit: fit }\n\
         let deep = side_l.place.deep + a.part.at\n",
    );
    let DesignItem::Region(region) = design(&file.items[1]) else {
        panic!()
    };
    let ExprKind::Call { args, .. } = &region.value.kind else {
        panic!()
    };
    let arg_names: Vec<&str> = args
        .iter()
        .filter_map(|arg| arg.name.as_ref().map(Name::as_str))
        .collect();
    assert_eq!(arg_names, ["at", "place", "stock"]);
    assert_eq!(*field(&file, "p", "fit"), *field(&file, "p", "fit"));
    assert!(matches!(field(&file, "p", "fit").kind, ExprKind::Fit));

    let DesignItem::Let(binding) = design(&file.items[3]) else {
        panic!()
    };
    let ExprKind::Binary { lhs, .. } = &binding.value.kind else {
        panic!()
    };
    let path = lhs.as_path().unwrap();
    let members: Vec<String> = path
        .segments
        .iter()
        .map(|segment| match segment {
            PathSegment::Member(name) => name.to_string(),
            PathSegment::Index(_) => "[]".to_owned(),
        })
        .collect();
    assert_eq!(members, ["place", "deep"]);
}

#[test]
fn a_keyword_where_a_new_name_goes_is_reserved() {
    for text in [
        "part joint : s { }\n",
        "let fit = 1\n",
        "region r[top, at] = x\n",
        "kind part : joint { }\n",
        "import \"./x\" as stock\n",
    ] {
        let parse = parse(&format!("stave 1\n{text}"));
        assert_eq!(parse.diagnostics.len(), 1, "{text}");
        assert_eq!(parse.diagnostics[0].code, Code::ReservedWord, "{text}");
    }
}

// ---------------------------------------------------------------------------
// The whole grammar
// ---------------------------------------------------------------------------

#[test]
fn every_item_form_parses() {
    let file = clean(
        "\n\nstave 1\n\
         import \"std:joinery\"\n\
         project \"P\" { goal: \"g\", units: mm }\n\
         library \"L\"\n\
         library \"L\" { about: \"x\" }\n\
         /// A kind.\n\
         private kind k : fastener { diameter: 3mm, length: 4mm }\n\
         recipe r : turning { tools: [lathe] }\n\
         stock s { species: pine, form: solid, thickness: 19mm }\n\
         profile f { measure: radius\n at 0mm: 3mm }\n\
         component C {\n\
         \x20 /// Doc.\n\
         \x20 param a : Region\n\
         \x20 param b : Count = 3 within [1, 2, 3] locked\n\
         \x20 param c = 1mm within 1mm..2mm + 1mm\n\
         \x20 let d = -a.up * (2 + 3) / 4 - !true\n\
         \x20 region e[3] = split(a, up)\n\
         \x20 part g[2] : s { length: fit, place { ground } }\n\
         \x20 joint h[x, y] : dado(g[*], into: [g[0], g[1]],)\n\
         \x20 hardware i : knob(g[0]) { place { center(g[0], across) } }\n\
         \x20 hardware j : slide(g[0], in: e[0])\n\
         \x20 assembly l : D { }\n\
         \x20 require m : a.up >= 1mm && b != 2 || c <= 3mm \"msg\"\n\
         }\n\
         scene sc { pose a[top] = open, explode: true }\n\
         scene sd \"Title\" { }\n\n\n",
    );
    assert_eq!(file.imports.len(), 1);
    assert_eq!(file.items.len(), 10);
    assert_eq!(file.items[3].docs, ["A kind."]);
    assert!(file.items[3].private.is_some());
    let ItemKind::Component(component) = &file.items[7].kind else {
        panic!()
    };
    assert_eq!(component.items.len(), 11);
    assert_eq!(component.items[0].docs, ["Doc."]);
    let DesignItem::Param(param) = &component.items[1].node else {
        panic!()
    };
    assert!(matches!(param.within, Some(Domain::OneOf { .. })));
    assert!(param.locked.is_some());
    let DesignItem::Param(param) = &component.items[2].node else {
        panic!()
    };
    let Some(Domain::Range { hi, .. }) = &param.within else {
        panic!()
    };
    assert!(matches!(
        hi.kind,
        ExprKind::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
}

#[test]
fn precedence_and_associativity() {
    let file = clean("stave 1\nlet a = 1 - 2 - 3 * 4 / 5 < 6 && !x || y\n");
    let DesignItem::Let(binding) = design(&file.items[0]) else {
        panic!()
    };
    fn shape(expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Binary { op, lhs, rhs } => {
                format!("({} {} {})", shape(lhs), op.symbol(), shape(rhs))
            }
            ExprKind::Unary { op, operand } => format!("{}{}", op.symbol(), shape(operand)),
            ExprKind::Number(Number::Integer(n)) => n.to_string(),
            ExprKind::Name(name) => name.to_string(),
            other => format!("{other:?}"),
        }
    }
    assert_eq!(
        shape(&binding.value),
        "(((((1 - 2) - ((3 * 4) / 5)) < 6) && !x) || y)"
    );
}

#[test]
fn patch_files_parse() {
    let text = "stave 1\n\
                patch \"Add a shoe shelf\" {\n\
                \x20 add part shoe_shelf : oak_19 {\n\
                \x20   length: fit\n\
                \x20 } in \"carcass.stave\"\n\
                \x20 add part k : s { length: 1mm } to casework.Drawer\n\
                \x20 add import \"./casework\" in \"doors.stave\"\n\
                \x20 replace profile bun_foot { measure: radius }\n\
                \x20 set side_l.place.deep = flush(base.back, inset: 6mm)\n\
                \x20 unset drawer[top].pull_kind\n\
                \x20 remove part casework.Drawer.back\n\
                \x20 remove import \"./x\" in \"y.stave\"\n\
                \x20 rename part side_l to left_side\n\
                }\n";
    assert!(stave_lang::is_patch_source(text));
    assert!(!stave_lang::is_patch_source("stave 1\npart a : s { }\n"));
    let mut sources = SourceMap::new();
    let file = sources.add("p.stave", text).unwrap();
    let parse = parse_patch_file(file, text);
    assert!(parse.diagnostics.is_empty(), "{:#?}", parse.diagnostics);
    assert_eq!(parse.syntax().to_string(), text);
    let patches = parse.ast.unwrap().patches;
    let ops: Vec<&str> = patches[0]
        .ops
        .iter()
        .map(|op| match &op.kind {
            PatchOpKind::Add { to: Some(_), .. } => "add to",
            PatchOpKind::Add { file: Some(_), .. } => "add in",
            PatchOpKind::Add { .. } => "add",
            PatchOpKind::AddImport { .. } => "add import",
            PatchOpKind::Replace { .. } => "replace",
            PatchOpKind::Set { .. } => "set",
            PatchOpKind::Unset { .. } => "unset",
            PatchOpKind::Remove { tag, .. } => tag.keyword(),
            PatchOpKind::RemoveImport { .. } => "remove import",
            PatchOpKind::Rename { to, .. } => to.as_str(),
        })
        .collect();
    assert_eq!(
        ops,
        [
            "add in",
            "add to",
            "add import",
            "replace",
            "set",
            "unset",
            "part",
            "remove import",
            "left_side"
        ]
    );

    let broken = "stave 1\npatch \"p\" {\n  explode part a\n  set a = 1\n}\n";
    let file = sources.add("q.stave", broken).unwrap();
    let parse = parse_patch_file(file, broken);
    assert_eq!(parse.diagnostics.len(), 1);
    assert_eq!(
        parse.diagnostics[0].message,
        "expected 'add' or 'replace' or 'set' or 'unset' or 'remove' or 'rename', found 'explode'"
    );
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let text = format!(
        "stave 1\nlet a = {}1{}\nlet b = 2\n",
        "(".repeat(500),
        ")".repeat(500)
    );
    let parse = parse(&text);
    assert_eq!(parse.diagnostics.len(), 1);
    assert_eq!(names(parse.ast.as_ref().unwrap()), ["b"]);
}

#[test]
fn junk_never_hangs_the_parser() {
    for text in [
        "",
        "stave",
        "stave 1\n}}}\n",
        "stave 1\n{{{\n",
        "stave 1\n)))]]]\n",
        "stave 1\npart",
        "stave 1\npart p : s {",
        "stave 1\n\"",
        "stave 1\n/* ",
        "stave 1\nkind k : joint { cut }",
        "stave 1\n@#$%^",
        "stave 1\npatch \"x\" { add }",
        "stave 1\ncomponent C { part p : s { place { ground",
        "\u{feff}stave 1",
    ] {
        let parse = parse(text);
        if text.starts_with("stave 1\n") && text.len() > 8 {
            assert!(!parse.diagnostics.is_empty(), "{text:?}");
        }
    }
}
