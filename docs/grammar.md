# The `.stave` language

Version 1, draft 3.

A `.stave` file describes a woodworking project as **intent**: parameters,
stock, parts, joints, hardware and how parts relate to each other. Everything
derived (positions, meshes, cut list, build manual, order of work) is computed
and never stored.

Draft 3 applies `docs/reviews/2026-11-16-language-review-2.md`. What changed
from draft 2 is listed in that file's "Resolution" section.

Design rules:

1. **Every length carries a unit.** `19mm`, `3/4in`, `1-1/2in`. A bare number is
   a count or a ratio, never a length.
2. **No coordinates, ever.** There is no point, vector or tuple literal. Parts
   are positioned only by relationships to named anchors (faces and
   mid-planes), optionally offset by real measurements. See [Placement](#placement).
3. **Everything has a name and is referenced by it**: parts, joints, hardware,
   regions, checks, roles. Nothing is identified by its position in a list.
   Names from another module are always qualified: `casework.Drawer`.
4. **One fact, once.** A measurement is written in one place; everything that
   depends on it refers to it. See [Sizes](#sizes).
5. **One construct per job.** Declarations use `keyword name : kind`; settings
   of a joint or a piece of hardware are arguments of its kind call; a `{ }`
   body holds fields, and `place { }` holds placement. Lists are always `[ ]`.
6. **A newline ends a field**, so an error is reported on the line that has it.
   See [Newlines](#newlines).
7. **A folder is a module.** Every `.stave` file in a folder shares its names;
   other folders are imported. See [Modules](#modules).

---

## Lexical grammar

```ebnf
(* Spaces, tabs, carriage returns and comments separate tokens and are
   otherwise ignored. Newlines are tokens; see "Newlines". *)
line_comment  = "//" , { ? any char except LF ? } ;      (* not "///" *)
block_comment = "/*" , { ? any char ? } , "*/" ;         (* does not nest *)
DOC           = "///" , { ? any char except LF ? } ;     (* a token; see "Doc comments" *)
NL            = LF ;                                     (* see "Newlines" *)

WORD          = ident_start , { ident_char } ;
ident_start   = "A".."Z" | "a".."z" | "_" ;
ident_char    = ident_start | DIGIT ;
KEYWORD       = ? a WORD in the keyword list below ? ;
IDENT         = ? a WORD that is not a KEYWORD ? ;
NAME          = IDENT | KEYWORD ;                        (* any word; see "Names" *)

STRING        = '"' , { str_char | escape | interp } , '"' ;  (* str_char: any char except '"', "\", "{" and LF *)
interp        = "{" , NAME , { "." , NAME } , "}" ;      (* "{depth}", "{piece.thickness}"; see "Strings" *)
escape        = "\" , ( '"' | "\" | "n" | "t" | "{" | "u{" , HEX , { HEX } , "}" ) ;

INT           = DIGIT , { DIGIT | "_" } ;
DECIMAL       = INT , "." , DIGIT , { DIGIT } ;          (* "1..2" is INT ".." INT *)

(* A quantity is ONE token: no whitespace inside it. *)
QUANTITY      = magnitude , unit ;
magnitude     = INT , "-" , INT , "/" , INT              (* mixed fraction: 1-1/2in *)
              | INT , "/" , INT                          (* fraction: 3/4in *)
              | DECIMAL                                  (* 18.5mm *)
              | INT ;                                    (* 19mm *)
unit          = "mm" | "cm" | "m" | "in" | "ft" | "deg" ;

NUMBER        = DECIMAL | INT ;                          (* unitless: counts, ratios *)

(* Punctuation and operators *)
PUNCT         = "{" | "}" | "(" | ")" | "[" | "]" | "," | ":" | "." | ".." | "="
              | "==" | "!=" | "<" | "<=" | ">" | ">=" | "+" | "-" | "*" | "/"
              | "&&" | "||" | "!" ;
```

**Keywords** (reserved; an `IDENT` can't be one of these):

```
stave project library import private kind recipe param let region stock part
joint hardware assembly profile component scene require patch add set remove
place pose at within locked fit true false
```

A keyword where a new name is expected is `error[reserved-word]`. A keyword
**is** allowed wherever the grammar says `NAME`: after a `.` and before a `:`
(a field name, a named argument, a role name). So `into.stock`,
`side_l.place.deep`, `split(bay, up, at: divider)` and `kind dado : joint` are
all fine.

Face and direction words (`top`, `left`, `front`, `up`, ...), type names and
built-in function names are **not** keywords, so a part may be called `top` or
`back`.

**Contextual words** are ordinary `IDENT`s that have a special meaning in one
grammar position only:

| Word | Special only |
|---|---|
| `as` | after the string of an `import` |
| `mirror`, `about` | directly after the `=` of a `part` or `assembly`, and as that call's named argument |
| `joint`, `slide`, `hinge`, `pull`, `fastener`, `tool` | as the base kind after `kind name :` (`joint` is also a keyword) |
| `roles`, `cut`, `makes`, `member` | at the start of an entry in a `kind` body, when not followed by `:` |
| `mark`, `cut` | at the start of an entry in a `recipe` body, when not followed by `:` |
| `same` | as the whole content of an index: `drawer_bay[same]` |
| `to`, `in` | after the item of a patch's `add`, and in `rename ... to` |
| `replace`, `unset`, `rename` | at the start of a patch operation |

### Quantities

A number directly followed by a letter is always a quantity; the letters must
be a unit, otherwise it is `error[unknown-unit]`. The lexer tries the longest
`QUANTITY` first:

| Source | Tokens | Meaning |
|---|---|---|
| `3/4in` | `QUANTITY(3/4 in)` | three-quarters of an inch |
| `3/4` | `INT(3)` `/` `INT(4)` | the ratio 3/4 |
| `1-1/2in` | `QUANTITY(1 1/2 in)` | one and a half inches |
| `1 - 1/2in` | `INT(1)` `-` `QUANTITY(1/2 in)` | type error: a count minus a length |
| `2in-1/2in` | `QUANTITY(2in)` `-` `QUANTITY(1/2in)` | 1-1/2in |
| `1/3in` | `QUANTITY(1/3 in)` | exactly a third of an inch (values are exact fractions) |
| `3inch` | error | `unknown unit 'inch' (expected one of mm, cm, m, in, ft, deg)` |

A fraction's denominator can't be zero (`error[syntax]`).

### Newlines

A newline is a token (`NL`), except that the lexer **drops** it in exactly
these two cases:

1. inside `( )` or `[ ]`, at any depth;
2. directly after one of these tokens (comments in between don't count):

   ```
   ,  (  [  +  -  *  /  ==  !=  <  <=  >  >=  &&  ||  !  ..  within  DOC
   ```

Nothing else drops a newline. In particular a newline after `:`, `=` or `{` and
before `}` is kept; the grammar's block rule accepts newlines there. `within`
is the only keyword that suppresses a following newline.

Case 2 never drops the newline before a line whose first token is an **item
keyword** (see [Unclosed brackets](#unclosed-brackets)) not followed by `:`,
or a doc comment, unless the line before ends with a doc comment. No expression continues with
a declaration, so this never changes the meaning of a correct file; it keeps
a forgotten operand local: in `let a = 1 +` followed by `let b = 2`, the
error is the missing value after `+`, and `b` is still declared.

So a long expression continues onto the next line when the line ends with an
operator, a comma or an open bracket, and a field always ends at the end of
its line:

```
length: cabinet_height
        - base.thickness          // error: line starts with an operator; end the
                                  // previous line with "-" instead
length: cabinet_height -
        base.thickness            // fine
```

A missing value is reported on its own line: `length:` followed by a newline is
`error[missing-value]: 'length' has no value`, not a confusing error on the
next field. The same holds after `=`.

### Unclosed brackets

Because newlines are dropped inside `( )` and `[ ]`, a forgotten closing
bracket would otherwise swallow every line after it. One rule keeps the damage
local:

> When a line's first token is an **item keyword** and the next token is not
> `:`, or a doc comment, every `(` and `[` still open is closed before that
> line, and each one is reported as `error[unclosed-bracket]`. Lexing and
> parsing continue normally.

Item keywords are the words that start a declaration: `import`, `project`,
`library`, `private`, `kind`, `recipe`, `stock`, `profile`, `component`,
`scene`, `param`, `let`, `region`, `part`, `joint`, `hardware`, `assembly`,
`require`, `patch`.

```
region front_opening = opening(
  left: side_l.right,
  top:  top.bottom              // the ")" is missing
region bay[drawers, doors] = split(front_opening, up, at: divider)
```

```
error[unclosed-bracket]: this "(" is never closed
  --> carcass.stave:1:31
   = note: the next declaration starts on line 4
   = help: add ")" at the end of line 3
```

`bay` and everything after it parse and check as usual.

The rule never changes the meaning of a correct file: inside brackets only
expressions and arguments are allowed, and none of them starts with an item
keyword or contains a doc comment. The "next token is not `:`" condition leaves keywords used as argument
names alone (`stock: oak_19,` at the start of a line inside a call).

For the same reason a `}` closes every `(` and `[` opened inside its block,
and the end of the file closes every one still open; each is reported as
`error[unclosed-bracket]` in the same way (the note then says where the
block or the file ends). No expression can contain a `}`, so this too never
changes the meaning of a correct file.

A bracket closed by any of these three is closed at the line where it was
most likely forgotten: the first line break after it (outside any bracket
opened later) where one line ends with a value (a name, number, quantity,
string, `)` or `]`) and the next starts with another (a name, number,
quantity, string or `!`). Two values are never written side by side, so
there the expression can't go on. When there is no such line break, the
bracket is closed just before the line that closed it. So a `)` forgotten at
the end of one placement term is one error, its help names that line, and
the next term is still read:

```
place {
  flush(base.left, inset: 10mm      // the ")" is missing
  against(base.top)
}
```

A missing `}` is recovered by the parser instead, because braces depend on
context: a component body may contain items, but a part, joint or hardware
body may not. When a body meets an entry it can't contain that starts with an
item keyword (`part` inside a part body), the body ends there with
`error[missing-brace]`, and the item is parsed as the next item of the
enclosing scope.

### Doc comments

`///` starts a doc comment. Each line is one `DOC` token (the newline after it
is dropped). Doc comments attach to the next item; inside a `kind` body they
may also precede a `param`, a `require`, a `makes` or a `member`. Anywhere
else a doc comment is `error[syntax]`. Docs reach the inspector and the build manual.

### Strings

In the text of `note:`, `check:`, and a `require` message, `{name}` inserts a
value: a param or role of the surrounding kind, or a param of the surrounding
module or component, optionally followed by members (`{piece.thickness}`).
Lengths are formatted in the project's units. Write `\{` for a literal brace.

The lexer splits every string into text and `interp` parts, each with its own
span, so names inside strings are resolved, renamed, coloured and
go-to-definition'd like any other name. In any other string (`goal:`, a scene
title, an import source), an `interp` is `error[syntax]` with the help
"write \{ for a literal brace".

---

## Syntactic grammar

`block(X)` is a template, expanded for each `X`. It is the only place where
`{ }` and separators appear, so every block follows the same rule: entries are
separated by one or more newlines or commas; separators before the first entry
and after the last are optional.

```ebnf
sep           = NL | "," ;
block(X)      = "{" , { sep } , [ X , { sep , { sep } , X } ] , { sep } , "}" ;
                (* so "{ }", "{ depth: 9mm }" and "{ a: 1, b: 2 }" on one line all parse *)

(* ---------- Files ---------- *)
file          = { NL } , header , { NL , { NL } , import } , { NL , { NL } , item } , { NL } ;
patch_file    = { NL } , header , { NL , { NL } , patch } , { NL } ;
                (* every import, item and patch starts on a new line; imports come first *)
header        = "stave" , INT ;                          (* the format version: 1; any other is error[not-supported] *)
import        = "import" , STRING , [ "as" , IDENT ] ;  (* applies to this file only *)

(* ---------- Items ---------- *)
item          = { DOC } , [ "private" ] , item_kind ;
item_kind     = project | library | kind | recipe | stock | profile | component
              | scene | design_item ;
design_item   = param | let | region | part | joint | hardware | assembly | require ;

project       = "project" , STRING , block(field) ;
library       = "library" , STRING , [ block(field) ] ;

kind          = "kind" , IDENT , ":" , base_kind , block(kind_entry) ;
base_kind     = "joint" | "slide" | "hinge" | "pull" | "fastener" | "tool" ;
kind_entry    = { DOC } , ( param | require | makes | member )
              | roles | cut | field ;
member        = "member" , IDENT ;                       (* a hardware kind's named piece: member cup *)
roles         = "roles" , block(role) ;
role          = NAME , ":" , IDENT ;                     (* piece: Part   into: Parts *)
cut           = "cut" , call , block(call) ;             (* cut end(piece, facing: into) { tenon(...) } *)
makes         = "makes" , "part" , IDENT , ":" , ref , block(part_entry) ;

recipe        = "recipe" , IDENT , ":" , IDENT , block(recipe_entry) ;
                (* recipe notch_by_hand : notch { }   — how to make one cut operation *)
recipe_entry  = "mark" , block(call)                     (* marking-out steps *)
              | "cut" , block(call)                      (* cutting steps *)
              | field ;                                  (* tools: [...], rank: 10, needs: [...], check: "..." *)

param         = "param" , IDENT , [ ":" , IDENT ] , [ "=" , expr ] ,
                [ "within" , domain ] , [ "locked" ] ;   (* a type, a default, or both *)
domain        = add_expr , ".." , add_expr               (* continuous: 600mm..1200mm, both ends included *)
              | list ;                                   (* one of: [350mm, 400mm, 450mm] *)
let           = "let" , IDENT , "=" , expr ;

region        = "region" , IDENT , [ array ] , "=" , expr ;
stock         = "stock" , IDENT , block(field) ;

part          = "part" , IDENT , [ array ] ,
                ( ":" , ref , block(part_entry)          (* cut from stock *)
                | "=" , mirror ) ;                       (* the mirror image of another part *)
part_entry    = field | place ;
mirror        = "mirror" , "(" , expr , "," , "about" , ":" , expr , [ "," ] , ")" ;

joint         = "joint" , IDENT , [ array ] , ":" , kind_call ;
hardware      = "hardware" , IDENT , [ array ] , ":" , kind_call , [ block(place) ] ;
kind_call     = ref , "(" , [ args ] , ")" ;
                (* the call names the kind, its parts by role, and its settings:
                   joint back_rabbets : rabbet(back, into: [side_l, side_r], depth: 9mm) *)

assembly      = "assembly" , IDENT , [ array ] ,
                ( ":" , ref , block(field)               (* instance of a component: casework.Drawer *)
                | "=" , mirror ) ;                       (* the mirror image of another assembly *)

profile       = "profile" , IDENT , block(profile_entry) ;
profile_entry = "at" , expr , ":" , expr                 (* station: value *)
              | field ;

component     = "component" , IDENT , block(component_item) ;
component_item = { DOC } , design_item ;

scene         = "scene" , IDENT , [ STRING ] , block(scene_entry) ;
scene_entry   = "pose" , path , "=" , expr               (* pose door_l = open *)
              | field ;                                  (* show: drawer[top] *)

require       = "require" , IDENT , ":" , expr , STRING ;

array         = "[" , ( IDENT , "," , IDENT , { "," , IDENT } , [ "," ]
                                                         (* two or more names: labels *)
                      | expr ) , "]" ;                   (* otherwise: a count *)

(* ---------- Fields, placement, references ---------- *)
field         = NAME , ":" , expr ;                      (* exactly one expression *)
place         = "place" , block(call) ;                  (* one placement term per line *)
call          = IDENT , [ "(" , [ args ] , ")" ] ;       (* a head fixed by position: ground, flush(...) *)

ref           = IDENT , { "." , NAME } ;                 (* oak_19, casework.Drawer, japanese.ari_tsugi *)
path          = IDENT , { "." , NAME | "[" , index , "]" } ;
index         = "*"                                      (* every element *)
              | "same"                                   (* the element with the same label or number *)
              | expr ;                                   (* a label, or a number for a counted array *)

(* ---------- Expressions (lowest to highest precedence) ---------- *)
expr          = or_expr ;
or_expr       = and_expr , { "||" , and_expr } ;
and_expr      = cmp_expr , { "&&" , cmp_expr } ;
cmp_expr      = add_expr , [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) , add_expr ] ;
add_expr      = mul_expr , { ( "+" | "-" ) , mul_expr } ;
mul_expr      = unary , { ( "*" | "/" ) , unary } ;
unary         = ( "-" | "!" ) , unary | postfix ;
postfix       = primary , { "." , NAME | "[" , index , "]" | "(" , [ args ] , ")" } ;
args          = arg , { "," , arg } , [ "," ] ;
arg           = [ NAME , ":" ] , expr ;                  (* named when a NAME is directly followed by ":" *)
list          = "[" , [ expr , { "," , expr } , [ "," ] ] , "]" ;

primary       = QUANTITY | NUMBER | STRING | "true" | "false" | "fit"
              | IDENT | list | "(" , expr , ")" ;

(* ---------- Patches (AI and suggestion output) ---------- *)
patch         = "patch" , STRING , block(patch_op) ;
patch_op      = "add" , item , [ "to" , ref ] , [ "in" , STRING ]
              | "add" , import , [ "in" , STRING ]
              | "replace" , item
              | "set" , path , "=" , expr
              | "unset" , path
              | "remove" , ( item_tag , path | "import" , STRING , [ "in" , STRING ] )
              | "rename" , item_tag , path , "to" , IDENT ;
item_tag      = "kind" | "recipe" | "param" | "let" | "region" | "stock" | "part" | "joint"
              | "hardware" | "assembly" | "profile" | "component" | "scene" | "require" ;
```

Notes for the parser:

- **Two tokens of lookahead decide everything.** At the start of a block
  entry, a word directly followed by `:` is always a field name, even when it
  is a keyword or a contextual word (`cut: ...` is a field, `cut end(...) { }`
  is a cut). An argument is named when a `NAME` is directly followed by `:`.
  An `array` holds labels when an `IDENT` is directly followed by `,`.
  `[same]` is the elementwise index only when `same` is directly followed by
  `]`.
- Positional arguments come before named ones; a positional argument after a
  named one is `error[syntax]`.
- A `block(part_entry)` and a `block(place)` hold at most one `place`; a second
  one is `error[syntax]`. A joint has no body: `joint x : dado(...) { ... }` is
  `error[syntax]` with the help "a joint's settings go inside the parentheses".
- `mirror` takes exactly a source and `about:`. `part x = <anything else>` is
  `error[syntax]`: a derived part is always a mirror image.
- There are no object literals. `{ }` appears only after a declaration header.
  Inside an expression, everything is a call: `euro_hinge(opening_angle: 110deg)`.
- An empty `block` is fine: `scene closed "Closed" { }`.

---

## Names

There are four namespaces. Which one a word belongs to is decided by **where it
stands**, so the same word can be used in several of them without a clash.

| # | Namespace | Where | Resolved against |
|---|---|---|---|
| 1 | **Items** | a bare `IDENT` in an expression, a `ref`, the root of a `path` | the enclosing `kind` or `component` body, then the module, then the modules imported by this file (as `module.name`), then the prelude |
| 2 | **Members and argument names** | a `NAME` after `.` or before `:` | the thing on the left of the `.`, or the call or declaration that takes the argument or field |
| 3 | **Call heads fixed by position** | the head of a `call` in `place { }`, after `cut`, inside a `cut` block, inside `mark { }` and `cut { }` of a recipe; the base kind after `kind x :`; the operation after `recipe x :` | the fixed vocabulary of that position (tables below); never items |
| 4 | **Bare enum words** | a bare `IDENT` where the expected type has a closed list of words: `up`, `left`, `evenly`, `open`, `plywood`, `through`, `length`, `mm` | the expected type's words |

Rules:

1. **Items.** A name is declared once per scope: twice in one module (in any
   of its files) or twice in one component or kind body is
   `error[duplicate-name]`, naming both places. An inner scope may reuse an
   outer name; the inner one wins. Params, roles and made parts of a kind
   share one scope.
2. **The prelude is the outermost scope**, so a module may declare an item with
   the name of a prelude item (a part called `square`, a param called `knob`).
   If both are a kind, or both a recipe, it is `warning[shadows-prelude]`,
   because uses in that module now mean the local one.
3. **Built-in functions** (`between`, `mid`, `opening`, `cover`, `box`, ...)
   are looked up only when a bare name is **called** and no item with that name
   is in scope as a kind. So `param opening : Region` and `opening(left: ...)`
   coexist. A `kind` or `component` can't take a built-in function's name
   (`error[duplicate-name]`), because both would be callable.
4. **Enum words win where they are expected.** In `center(base, across)` the
   second argument expects an axis, so `across` is the axis even if an item
   called `across` exists; in `hide: back` the field expects items, so `back`
   is the part. It is `error[ambiguous-name]` only when the position accepts
   both readings: for example a `let through = 5mm` used as `depth: through`,
   where a depth may be a length or the word `through`.
5. **Members** never clash with items: `drawer[top].top` is the `top` face of
   the element labelled `top`.
6. **Names are resolved where they are written.** The body of a component or
   kind sees the module it is declared in (and the prelude), never the module
   that uses it. What a user passes in arrives through params and roles.
7. A module name can't be a keyword or contain characters outside `IDENT`; such
   a folder needs `as`: `import "./hardware" as hw`,
   `import "./my-shop" as shop`. Without it: `error[reserved-word]`. A module
   name can't equal an item name in the importing module
   (`error[duplicate-name]`).

### Arrays

- `part shelf[3]` declares a counted array: `shelf[0]`, `shelf[1]`, `shelf[2]`.
  The count is a constant (a count, or a `let` or `param` without a range), or
  a param with a `within` list, so the checker can try every count. A param
  with a continuous range (`within 2..6`) is `error[type-mismatch]`.
- `assembly drawer[bottom, top] : Drawer { ... }` declares a labelled array:
  `drawer[bottom]`, `drawer[top]`. Two or more names in a declaration are
  labels; anything else is a count. A single label isn't allowed (just name
  the part).
- **A labelled array is indexed by label only.** In `drawer[top]` the word in
  brackets is looked up among the array's labels and nowhere else, so labels
  never clash with items. `drawer[0]` on a labelled array is
  `error[unknown-name]`. A counted array is indexed by a `Count` expression.
- **`x[*]` always means every element.** Inside a list it flattens:
  `[stile[*], rail[*]]` is one list of four parts. Nested lists don't exist;
  a list inside a list is flattened the same way.
- **`x[same]` is elementwise.** Inside the declaration of an array (anywhere in
  its header, body or `place`), `other[same]` is the element of `other` with
  the same label, or the same number, as the element being declared. The two
  arrays must have the same labels in the same order, or the same count;
  otherwise `error[type-mismatch]`. Outside an array declaration `[same]` is an
  error.

  ```
  region drawer_bay[bottom, top] = split(bay[drawers], up)
  assembly drawer[bottom, top] : casework.Drawer {
    opening: drawer_bay[same]     // drawer[bottom] gets drawer_bay[bottom], and so on
  }
  ```

- The same label twice in one declaration (`part side[left, left]`) is
  `error[duplicate-name]`, reported by the parser.
- An unindexed array name inside its own declaration means the element being
  declared; anywhere else an unindexed array name is `error[type-mismatch]`
  with the help "write `name[*]` for all of them".

---

## Modules

**A folder is a module.** Every `.stave` file directly inside a folder belongs to
that folder's module and shares its names. File names and order don't matter:
splitting or merging files never changes meaning. Subfolders are separate
modules.

Each module has exactly one `project` item (a project module) or one `library`
item (a library module), in any of its files.

```
clothing_cabinet/          // project module (see examples/clothing_cabinet)
  cabinet.stave            //   project, params, requires, scenes
  stock.stave
  carcass.stave
  doors.stave
  drawers.stave
  stave.lock               //   only when there are remote imports
  casework/                // library module, used as casework.*
    library.stave          //   library "Casework"
    frame_panel_door.stave
    drawer.stave
  hardware/                // library module; 'hardware' is a keyword, so: import "./hardware" as hw
    library.stave          //   library "Shop hardware"
    handles.stave
```

### Imports

```
import "./casework"
import "./hardware" as hw
import "github:some-maker/stave-catalog/slides@v2.1.0" as slides

assembly drawer[bottom, top] : casework.Drawer {
  opening:   drawer_bay[same]
  slide:     slides.side_mount_500
  pull_kind: hw.bar_pull_160
}
```

- You import a **folder**, never single names. Everything in it is used through
  the module's name: `casework.Drawer`, `hw.brass_knob_26`. There is no
  wildcard or unqualified import, so a name's origin is visible at every use.
- The module name is the folder's last segment, or the name after `as`.
- An import applies to the file it is written in, so each file shows what it
  depends on.
- Imports between modules can't form a cycle (`error[import-cycle]`, listing it).

### The prelude

Every public item of the three standard modules is available in every file
without an import or a prefix:

| Module | Folder | Provides |
|---|---|---|
| `std:joinery` | `std/joinery` | joint kinds (`dado`, `rabbet`, `groove`, `dowel`, `round_tenon`, `socket`, `screw`, `cope_and_stick`, `mortise_tenon`, `half_lap`) and the standard recipes |
| `std:hardware` | `std/hardware` | hardware kinds (`side_mount_slide`, `euro_hinge`, `butt_hinge`, `knob`, `bar_pull`) and fasteners (`dowel_pin`, `wood_screw`) |
| `std:tools` | `std/tools` | tool kinds (`square`, `chisel`, `router`, `drill`, `lathe`, ...) |

Beginners write `dado(...)`, not `joinery.dado(...)`. Anything from outside
`std` is always qualified (`japanese.ari_tsugi`), so a joint from someone else
is visible as such. The three modules are written in `.stave` like any other
library, may not declare the same name twice between them, and change only
with the format version in the `stave` header. They can also be imported
explicitly (`import "std:joinery"`, `import "std:hardware" as stdhw`) to reach
an item that a local name hides.

### Sources

| Source | Example | Notes |
|---|---|---|
| relative path | `"./hardware"`, `"../shared/joinery"` | relative to the importing file |
| standard library | `"std:joinery"` | ships with Stave, versioned with it |
| GitHub | `"github:owner/repo/path@v2.1.0"` | a tag or a commit; `@main` is a warning and `stave build` refuses it |
| any URL | `"https://example.com/stave/stock#sha256=9f2c..."` | must carry a content hash; may point at a folder archive or a single file (a module of one file) |

- **`stave.lock`**, one per project, records every remote module's resolved
  commit and content hash, including modules that remote modules import.
- **Fetching happens only on request:** `stave fetch`, or a confirm button in
  the app. Checking never touches the network; it reads the local cache.
- **Imports can't run code.** `.stave` is declarative, so a module is data.
- **An AI patch can't add a remote import or model without confirmation.**
- In this version only relative paths and `std:` resolve. A remote source
  parses and is then `error[not-supported]`.

### What a library may contain

Libraries provide reusable declarations: `component`, `stock`, `profile`,
`kind`, `recipe`, `param`, `let` and `require`. A `part`, `assembly`, `region`,
`joint`, `hardware` or `scene` at the top level of a library is
`error[not-in-library]`, because those need placing in a project. (Inside a
component body they're fine.)

Items are public by default. Mark helpers `private` to keep them out of other
modules and out of autocomplete:

```
private let groove_offset = 8mm
```

### Placement stays inside a module

A part can only be placed against parts and regions of its own module, or
against what a component was handed as a param. Projects grow by components,
not by cross-folder references.

---

## Declarations and their fields

A field that isn't in the table of its declaration is `error[unknown-field]`; a
missing required field is `error[missing-field]`; a field written twice is
`error[duplicate-name]`. "Readable" means other expressions can read it as a
member (`oak_19.thickness`). Types are listed under [Types](#types-units-and-arithmetic).

### `project "Title" { }`

| Field | Type | Required | Meaning |
|---|---|---|---|
| `goal` | `Text` | no | one sentence about what is being built; shown on the cover |
| `units` | `mm` \| `cm` \| `m` \| `in` \| `ft` | no (default `mm`) | units used in output |
| `resolution` | `Length` | no (default `0.5mm`) | the cut list rounds to this, and says where it rounded |
| `style` | `Text` | no | a hint for suggestions |
| `tools` | list of `Tool` | no | the tools you have. Left out: every tool is assumed |
| `methods` | list of `Recipe` | no | recipes to prefer, most preferred first |

### `library "Title" { }`

| Field | Type | Required | Meaning |
|---|---|---|---|
| `about` | `Text` | no | one line of description |
| `license` | `Text` | no | credited in the build manual |

### `param name [: Type] [= default] [within domain] [locked]`

The same form everywhere: in a project or library module, in a `component` and
in a `kind`.

- At least one of the type and the default is required. With only a default,
  the type is the default's type. With both, they must agree.
- A param **without a default must be supplied**: as a field of the `assembly`
  body (component params) or as an argument of the kind call (kind params). A
  param at the top level of a module must have a default.
- A default may refer to the other params of its scope, to items of the
  module, and (in a kind) to roles: `param depth = into.thickness / 3`. Order
  doesn't matter, as everywhere else (a module's files have no order either);
  a value worked out from itself, directly or through others, is
  `error[type-mismatch]`.
- `within` gives the allowed values. For a module param it is the range the
  design must work over (see [Checks](#checks-and-require)). For a component
  or kind param it limits what a user may pass: an argument outside it is
  `error[out-of-range]`.
- `locked` stops smart edits from changing the value.

### `let name = expr`

A named value. Not a knob: it has no range and can't be passed in.

### `stock name { }`

| Field | Type | Required | Readable | Meaning |
|---|---|---|---|---|
| `species` | species word | yes | yes | `white_oak`, `red_oak`, `hard_maple`, `soft_maple`, `walnut`, `cherry`, `ash`, `beech`, `birch`, `pine`, `spruce`, `fir`, `cedar`, `hinoki`, `poplar`, `mahogany`, `teak` |
| `form` | `solid` \| `plywood` \| `mdf` \| `dowel` | yes | yes | what you buy |
| `thickness` | `Length` | yes, except `dowel` | yes | finished thickness |
| `diameter` | `Length` | `dowel` only, required | yes | diameter of round stock |
| `width` | `Length` | no | yes | stock sold at a fixed width (a square blank, a strip). Parts cut from it take this width |
| `max_width` | `Length` | no | yes | the widest board you can get; wider parts are planned as glue-ups |
| `max_length` | `Length` | no | yes | the longest board you can get; a part whose cut length is longer is `error[type-mismatch]` |

### `part name : stock { }` and `makes part name : stock { }`

| Field | Type | Required | Readable | Meaning |
|---|---|---|---|---|
| `length` | `Length` or `fit` | yes | yes | along the grain, by default |
| `width` | `Length` or `fit` | yes, unless the stock has `width:` or is a `dowel` | yes | |
| `thickness` | `Length` | no (default: the stock's) | yes | thinner than the stock; the cut list notes the planing. Thicker than the stock is `error[type-mismatch]` |
| `grain` | `length` \| `width` | no (default `length`) | no | which way the grain runs |
| `shape` | `Shape` | no | no | `turned(profile)`; the part's length is then the profile's: write `length: fit` (any other length is `error[type-mismatch]`) |
| `face_side` | face word | no | no | the reference face; see [Faces to cut](#faces-to-cut) |
| `face_edge` | face word | no | no | the reference edge |
| `note` | `Text` | no | no | printed on the part's work card |
| `place { }` | | yes | | see [Placement](#placement) |

Also readable on every part: `diameter` (round parts only), `stock`, and the
six faces `top`, `bottom`, `left`, `right`, `front`, `back`. Sizes read from a
part are its **drawn** sizes, before joints add to them (see [Sizes](#sizes)).

### `region name = expr`

No fields. Readable: the six faces, and the sizes `across`, `up` and `deep`.

### `assembly name : component { }`

The fields are the component's params, by name. A param without a default must
be given. Readable through the assembly: every item of the component
(`door_l.stile[hinge]`, `drawer[top].front`) and its params.

### `profile name { }`

| Entry | Type | Required | Meaning |
|---|---|---|---|
| `measure` | `diameter` \| `radius` | yes | what the station values are |
| `smooth` | `Bool` | no (default `false`) | `false`: straight lines between stations. `true`: one smooth curve through the stations |
| `at <station>: <value>` | `Length`: `Length` | two or more | the size at a distance from the start of the part. Stations must not decrease (`error[type-mismatch]`, whether or not a part uses the profile) |

Two stations at the same distance make a **shoulder**: a sharp step. With
`smooth: true`, a shoulder splits the curve, and a segment between two stations
of equal value stays straight, so a cylinder stays a cylinder. Readable:
`length` (the last station).

### `scene name "Title" { }`

| Entry | Type | Meaning |
|---|---|---|
| `pose <target> = <value>` | `open` \| `closed` \| `Angle` \| `Length` | sets a motion; see [Scenes](#scenes) |
| `show` | an item or a list | show only these |
| `hide` | an item or a list | hide these |
| `explode` | `Bool` | pull the parts apart along their joints' insert directions |
| `section` | `Anchor` | cut the view at a plane: what lies above it (`up`), to its right (`across`) or in front of it (`deep`) isn't drawn, and the wood it cuts is drawn flat |

### `require name : condition "message"`

The condition is a `Bool` expression. See [Checks](#checks-and-require).

### `kind name : joint { }`

| Entry | Type | Required | Meaning |
|---|---|---|---|
| `roles { }` | `Part` or `Parts` each | no (default `piece: Parts`, `into: Parts`) | the parts the joint connects |
| `param` | | no | settings; see `param` |
| `require` | | no | rules for valid use, checked for every pair |
| `insert` | list of assembly steps | no | how the parts come together |
| `cut_first` | a role, or a list of roles | no | the order in which the roles are cut |
| `adds` | `extend(...)` or a list of them | no | what the joint adds to each part's cut size |
| `uses` | a fastener kind value, or a list | no | fasteners the joint consumes |
| `cut selector { }` | | no | material removed |
| `makes part` | | no | a part the joint creates |
| `note` | `Text` | no | one piece of advice for the build manual |

Every joint kind also accepts two settings in its call that it doesn't
declare: `glue: Bool` (default `true`) and `methods:` (a list of recipes to
prefer for this joint's cuts). A kind can't declare a role, param or made part
named `glue` or `methods`.

### `kind name : slide | hinge | pull | fastener | tool { }`

A required entry may be written as a field (`clearance: 12.7mm`) or declared as
a `param` of the same name, which makes it a setting of the family
(`param length = 500mm within [...]`). Fields may refer to the kind's params.
All entries are readable (`slide.clearance`, `hinge_kind.cup_diameter`).

| Base | Roles (fixed) | Required | Optional |
|---|---|---|---|
| `slide` | `piece: Part` (the drawer side), `in: Region` (the opening) | `length`, `clearance`, `travel`: `Length`; `envelope`: `Envelope` | `model`, `moving`, `license` |
| `hinge` | `piece: Part` (the door stile, the lid), `on: Part` (the cabinet side, the box) | `opening_angle`: `Angle`; and either, for a cup hinge, `cup_diameter`, `cup_depth`, `cup_inset`, `plate_setback`: `Length`; `plate`: `Envelope` (a box); or, for a butt hinge, `leaf`: `Envelope` (a box) | `base_overlay`, `plate_height` (default `0mm`), `overlay_adjustment` (default `0mm`): `Length`; `model`, `moving`, `license` |
| `pull` | `piece: Part` | `envelope` | `centres`, `diameter`, `screw_diameter` (default `4mm`): `Length`; `model`, `license` |
| `fastener` | none | `diameter`, `length`: `Length` | `envelope`, `model`, `license` |
| `tool` | none | | `title`: `Text`; `powered`: `Bool` (default `false`) |

A hinge kind is a **cup hinge** (it has the `cup_` entries, `plate_setback`
and `plate`) or a **butt hinge** (it has `leaf`), never both. A hinge kind
with neither is `error[missing-field]` naming `leaf`; a cup hinge missing one
of its entries is `error[missing-field]` naming it; one with both is
`error[type-mismatch]`.

The two kinds move differently. A **butt hinge** is a pin: the door turns
about a fixed line. A **cup hinge** is a four-bar linkage: an arm on the
mounting plate, a boss in the cup and two links between them, so the door
moves forward and out while it turns and clears the side it covers. Every
cup hinge moves on Stave's one built-in linkage, that of the common 110°
full-overlay clip-on hinge, fixed to its cup (its pins are catalogue data
of the mechanism, never written in a `.stave` file; see
`docs/hardware/cup-hinge-linkage.md`). Its `opening_angle` is at most
`110deg` (`error[out-of-range]` otherwise; wide-angle hinges have another
mechanism, not modelled yet).

The optional entries are a cup hinge's catalogue overlay: `base_overlay` is
the overlay with the cup at the door's very edge on a 0mm plate (the
catalogue's base, or fixed distance), `plate_height` the mounting plate's
height (the catalogue's distance) and `overlay_adjustment` how far the arm's
side screw moves the door each way. When a kind gives `base_overlay`, the
door must overlay `on` by `base_overlay + cup_inset - plate_height`, give or
take `overlay_adjustment` (`error[hinge-overlay]` otherwise, with the
`cup_inset` or `plate_height` that fits).

`model` is a `Model`, `moving` and `license` are `Text`. Hardware kinds accept
`param`, `require` and `member` entries; they can't have `roles`, `cut`,
`makes` or the joint fields. See [Hardware](#hardware).

#### Hardware members

A hinge, slide or pull kind may name its pieces, each with a
doc comment a beginner can read. A viewer numbers them, labels them on the
drawing and shows the doc comment as the caption, in the order written:

```
kind euro_hinge : hinge {
  ...
  /// The mounting plate: screwed to the inside of the cabinet side, it
  /// holds the hinge on the side and never moves.
  member plate
  /// The cup: a steel cup pressed into a 35mm hole bored in the back of
  /// the door. The boss in its middle is where the links are pinned.
  member cup
}
```

The words are fixed per base, because Stave knows which piece each one is
(the mechanism itself, the linkage's pins, stays catalogue data and is never
written in a file):

| Base | Members |
|---|---|
| cup hinge | `plate`, `arm`, `links`, `cup` |
| butt hinge | `leaves`, `pin` |
| `slide` | `cabinet_member`, `drawer_member` |
| `pull` | `knob`, `bar`, `screw`, `screws` (a knob and its screw, a bar and its screws) |

Members are optional, each at most once (`error[duplicate-name]`). Any other
word is `error[unknown-field]`: `a cup hinge has no member 'cupp'`, with the
nearest member word as help; fasteners, tools and joints have no members
(`a fastener kind has no member 'head'`).

### `recipe name : operation { }`

| Entry | Type | Required | Meaning |
|---|---|---|---|
| `tools` | list of `Tool` | yes | every tool the recipe needs |
| `rank` | `Count` | no (default `100`) | lower is preferred |
| `needs` | list | no | what must still be true of the part; see [Recipes](#work-cards-and-recipes) |
| `mark { }` | marking steps | no | |
| `cut { }` | cutting steps | yes | |
| `check` | `Text` | no | what tells you the cut is right |

---

## Placement

`place { }` is the **only** way to position a part or a piece of hardware. It
holds one placement term per line. There is no syntax for coordinates.

### Directions, axes and faces

| Axis | Directions | Meaning |
|---|---|---|
| `up` | `up`, `down` | floor to ceiling |
| `across` | `left`, `right` | left to right, seen from the front |
| `deep` | `front`, `back` | front to back |

Each part has six faces named by the world direction they face once placed:
`top`, `bottom`, `left`, `right`, `front`, `back`. They are the faces of the
part's **drawn box**: its length, width and thickness as written, before joints
add to it. A turned part's box is as wide as its largest diameter.

A part also has three axes of its own: `length`, `width` and `thickness`.
Joint kinds use these, because a joint doesn't know how its parts are placed.

### Anchors

| Anchor | Example | Meaning |
|---|---|---|
| face | `base.top`, `door.left` | a face of a part or of a region |
| mid-plane | `mid(base, across)` | the plane halfway between `base.left` and `base.right` |

### Placement terms

| Term | Fixes | Meaning |
|---|---|---|
| `ground` | all three axes | the root that every other part is placed from. It may be turned with `orient`, like any part. The floor is wherever the lowest part ends up |
| `against(face)` | 1 axis | this part's **opposite** face touches the face: `against(base.top)` stands the part on the base |
| `against(face, gap: d)` | 1 axis | as above, then moved `d` away from the face, on the face's outward side |
| `flush(face)` | 1 axis | this part's **same-named** face lies in the face's plane: `flush(base.left)` lines up the two left faces |
| `flush(face, inset: d)` | 1 axis | as above, then moved `d` the other way from where the face points: into the part or region it belongs to |
| `center(item, axis)` | 1 axis | centred on the mid-plane of a part or region along that world axis |
| `center(from: a, to: b)` | 1 axis | centred between two parallel anchors |
| `spaced(spacing, from: a, to: b)` | 1 axis, for each element | arrays only: distributes the elements, in declaration order, from anchor `a` to anchor `b` |
| `corners(part, inset: d)` | 2 axes (`across`, `deep`) | arrays only: the array must have exactly the labels `front_left`, `front_right`, `back_left`, `back_right` (in any order). Each element sits in the corner of `part` its label names, its two outer faces `d` in from the part's faces |
| `orient(length: dir, width: dir)` | rotation | `length: down` means the part's length runs downward from its start (station 0 of a profile is at the top). Thickness takes the remaining axis |

`against` and `flush` take a face, not a mid-plane. An anchor's axis decides
which axis a term fixes: `flush(base.left)` fixes `across`.

**Spacing values** for `spaced`:

| Spacing | Meaning |
|---|---|
| `evenly` | equal gaps everywhere, including before the first and after the last element |
| `ends` | the first element flush with `from`, the last flush with `to`, equal gaps between |
| `ends(inset: d)` | as `ends`, with the first and last element moved `d` inward |
| `pitch(d)` | the first element flush with `from`, then a fixed centre-to-centre distance `d` |
| `gap(d)` | the first element flush with `from`, then a fixed gap `d` between elements |

With `pitch` and `gap`, an element that passes `to` is `error[overlap]`.
"Flush with `from`" means the element's edge nearest `from` lies in `from`'s
plane. When `from` lies after `to` along the axis, the elements run from
`from` toward `to` all the same (the first element at `from`).

**`orient`.** Both `length:` and `width:` are required and must be
perpendicular (`error[type-mismatch]` otherwise). `width:` may be left out only
for a round part (dowel stock, or a `turned` shape); its width then runs
`right`, or `back` when the length runs `left` or `right`. Without an `orient`
term the part lies flat: `orient(length: right, width: back)`.

### Placement rules (checked)

1. Exactly one part of a project is placed with `ground` (`error[ground]`).
   Parts inside components are never `ground`.
2. Every anchor belongs to something placed earlier in dependency order. A
   cycle is `error[placement-cycle]`, listing it.
3. Each axis is fixed by exactly one term. A missing axis is
   `error[axis-not-set]` with what to add; a second term on an axis is
   `error[axis-set-twice]`. At most one `orient`.
4. **Joints never position parts.** A joint only checks that the parts it
   connects actually meet, and adds to their cut sizes. The one exception is a
   part the joint makes itself (a key, a wedge), which the kind places.
5. Because of rules 2 to 4, positions are worked out directly in dependency
   order, without a solver, and every error points at one term.

Rake and splay (angled legs) are out of scope for version 1.

### Hardware placement

A piece of hardware is mounted on a **face of its `piece`**. Every hardware
call accepts `face:` (a face word) to name it; left out, the base kind decides:

| Base | Mount face (default) | Fixed by the base | Set by `place { }` |
|---|---|---|---|
| `slide` | the `left` or `right` face of `piece`, whichever is nearer the same-named face of the `in` region | everything: the slide runs along the piece's length, front ends level, centred on the piece's width | nothing; a `place` is `error[axis-set-twice]` |
| `hinge` | the `back` face of `piece` | a cup hinge: the cup sits `cup_inset` in from the long edge of the mount face that is nearer `on`; the plate sits on the inside face of `on`. A butt hinge: the leaves start at the pin edge (see below) | the one axis along the piece's length (the cup's or the leaves') |
| `pull` | the `front` face of `piece` | nothing | the two axes of the mount face |

The mount face fixes the axis it is normal to. The terms are the same as for
parts (`center`, `flush`, `against`, `spaced` for hardware arrays), applied to
the hardware's envelope. `orient`, `ground` and `corners` don't apply to
hardware.

An envelope lies on the mount face like this: its `thickness` (or a cylinder's
`length`) stands off the face; its `length` runs along the piece's length; its
`width` along the remaining axis. A slide's "front ends level" means its
envelope starts at the piece's front end (the low end of the piece's length
when the length doesn't run front to back). Fasteners and tools are never
placed.

**Members.** Hinges and slides are two pieces of metal: one moves with the
door or drawer, one is screwed to the carcass and stays put.

- A **cup hinge** is its **cup**, its **plate**, its **arm** and two
  **links**. The cup is a cylinder, `cup_diameter` across and `cup_depth`
  deep, sunk into the mount face (the back of the stile), its rim
  `cup_inset` in from the stile's long edge nearer the middle of `on`;
  `place { }` positions the cup along the piece's length. The plate is the
  `plate` box, screwed to the face of `on` that looks toward the middle of
  the door (the inside of the cabinet side): its `thickness` stands off that
  face, its `length` runs along the door's length centred on the cup, and
  its `width` runs back from the face of `on` the door closes against, its
  middle `plate_setback` back from it (the screw line; 37mm on 32mm-system
  cabinets). Neither member reaches into `on`'s thickness or past its outer
  face. The arm lies on the plate, from the plate's back to the door's back
  face, and its front end (the knuckle) reaches into the cup when the door
  is closed; it carries two pins. The cup carries the boss and its two pins;
  each link joins a pin in the arm to a pin in the boss. The arm and its
  pins are where the built-in mechanism puts them relative to the cup, so a
  side screw setting (see `overlay_adjustment`) moves the arm on the plate.
  The cup moves with the door; the plate and arm stay; each link turns
  about its pin in the arm. The arm, its knuckle and the links must stay
  out of the wood of the door and of `on` at every angle as the door
  opens; only the cup's bore is taken out of the door (its rim lies on the
  face and removes no wood). The hinge stops at its widest angle on a
  limiter inside the arm, steel on steel; nothing is excused as a stop.
- A **butt hinge** is two **leaves**, each the `leaf` box: its `length`
  runs along the pin (the piece's length), its `width` from the pin to the
  leaf's edge, its `thickness` is the leaf's. The mount face of `piece` must
  lie against a face of `on` (a lid's underside on the top edge of the box's
  back, a door's edge against the cabinet side). The **pin edge** is the long
  edge of the mount face where the piece's neighbouring face lies in the same
  plane as a face of `on`: the side where the two parts are flush and the
  knuckle shows. If neither edge (or both) is flush, it is the edge nearer the
  middle of `on`. One leaf is housed in a mortise in the mount face, the other
  in a mortise in the face of `on` it closes against, each as deep as the leaf
  is thick, both reaching `width` in from the pin edge; so the parts close
  face to face. Each mortise is a `notch` cut by the hinge. The leaf in
  `piece` moves with it; the leaf in `on` stays.
- A **slide** with a box envelope is two members, each half the envelope's
  thickness: the **drawer member** against the piece, which travels with
  the drawer, and the **cabinet member** against the `in` region's side,
  which stays.

### Mirroring

```
part side_r = mirror(side_l, about: mid(base, across))
assembly door_r = mirror(door_l, about: mid(base, across))
```

Mirroring is defined on **results**, not on text:

- The source is solved first. The mirror image is its geometry reflected
  through the `about` plane: every part, cut, made part and piece of hardware.
- A mirrored **part** is a new part with the same stock and sizes and the
  reflected position. Joints name it like any other part
  (`into: [side_l, side_r]`), and its cuts and what joints add to it come from
  the joints that name it, exactly as for any part; in a symmetric design
  they are the mirror image of its source's. Its frame (its own length,
  width and thickness, and its reference face and edge) is its source's,
  reflected.
- A mirrored **assembly** has the same item names as its source
  (`door_r.stile[hinge]` is the image of `door_l.stile[hinge]`; labels keep
  their names even when `left` ends up on the right). Each joint and piece of
  hardware inside it that refers to something **outside** the assembly (a
  param such as `hinge_side: side_l`) is re-bound to the part or region that
  occupies the reflected place: `side_r`. If nothing occupies that place
  exactly, it is `error[mirror]`, naming the item that has no counterpart.
- Arguments are not rewritten: there is no textual swap of `left` and `right`.
- The cut list marks a mirrored part as the mirror image of its source, and
  the work plan marks the pair out together.

---

## Sizes

**The rule: write the size you see; joints add their own engagement.** For a
shelf in dados, the drawn length is the distance between the sides; the dado
depth is added to the cut length by the joint. The cut list always shows the
final size.

Each part therefore has two sizes along each of its axes:

- the **drawn size**: what the field says. Placement, anchors and `between` use
  the drawn box.
- the **cut size**: the drawn size plus what joints add (`adds:`). Geometry,
  collision checks and the cut list use the cut size.

`fit` asks Stave to work out a drawn size:

| Where `fit` is written | What decides the size |
|---|---|
| `length` or `width` of a part that is the `piece` (first role) of one or more joints | the clear distance, along that axis, between the parts it is joined into |
| `length` of a part with `shape: turned(profile)` | the profile's last station |

For the first case, take the term that places the part along that world axis.
With `center(...)`, the size is the distance between the nearest joined part
wholly on one side of the centre and the nearest one wholly on the other side.
With `against(...)` or `flush(...)`, one end is known and the size reaches to
the nearest joined part beyond it. Joined parts that straddle the centre are
ignored, so a door panel grooved into four frame members gets its length from
the rails and its width from the stiles. If a side has no joined part, it is
`error[fit-undecided]`, saying which side. The joined parts are every part
named in the other roles of the joint, whether or not they meet it. The part
stays where its term puts it: with `center(...)` it is centred on the term's
centre, so it fills its gap exactly only when that centre is the middle of
the gap (`center(from: a.right, to: b.left)` makes sure of it).

When no joint decides a size, measure it: `length: between(base, top)`.

A part's thickness comes from its stock. A part may set a thinner
`thickness:` (a key planed down from thicker stock).

---

## Regions

A region is a named box of space, used for openings and bays. It has the six
faces and mid-planes of a part, and the sizes `across`, `up` and `deep`. It is
never cut and is always defined by one expression:

```
region front_opening = opening(left: side_l.right, right: side_r.left,
                               bottom: base.top, top: top.bottom,
                               front: side_l.front, back: back.front)
region bay[drawers, doors]     = split(front_opening, up, at: divider)
region drawer_bay[bottom, top] = split(bay[drawers], up)
region door = cover(opening, mount, thickness: frame.thickness)
```

| Function | Result |
|---|---|
| `opening(left:, right:, bottom:, top:, front:, back:)` | the box bounded by six anchors, one per side. All six are required |
| `split(region, axis, at: part)` | two regions, on either side of the part along `axis`, each ending at the part's face |
| `split(region, axis)` | as many equal regions as the declaration has elements (so the region being declared must be an array), with nothing between them |
| `cover(region, mount, thickness: d)` | the space a door or drawer front fills: the region's front face, grown or shrunk at each edge as the mount says, `d` deep |

`split` labels its pieces in the direction of its axis: for `up` the first
label is the lowest piece, for `across` the leftmost, for `deep` the front one.

**Mounts** say how a front relates to its opening:

| Mount | Meaning |
|---|---|
| `overlay(left:, right:, top:, bottom:, meeting:)` | the front lies in front of the opening (its back face on the opening's front face) and reaches past each edge by the given amount; negative is a reveal. Each defaults to `0mm` |
| `inset(reveal:)` | the front sits inside the opening, its front face flush with the opening's, with an even gap all round |

`meeting:` replaces the value on every edge that the opening **shares with a
neighbouring region of the same `split`**, when that split has no part between
its pieces (between two doors, between two drawers). So one mount works for
every front in a row. Without `meeting:`, every edge takes its own value.

---

## Joints and hardware

Every joint and piece of hardware has a name, written first. The kind is a
call whose arguments name the parts by role and give the settings:

```
joint shelf_dados  : dado([divider, shelf], into: [side_l, side_r], depth: 9mm)
joint back_rabbets : rabbet(back, into: [side_l, side_r], depth: 9mm)
hardware slides[left, right] : slide(side[same], in: opening)
hardware handle : knob_kind(stile[latch]) {
  place {
    center(stile[latch], across)
    center(stile[latch], up)
  }
}
```

- **All settings go in the call.** A joint has no body. A hardware body holds
  only `place { }`.
- Roles and params of a kind share one namespace, so every argument name means
  exactly one thing. An unknown argument is `error[unknown-field]`; a role or
  a param without a default that is not given is `error[missing-field]`.
- **The positional argument binds the first declared role** (`piece` for the
  standard joints and for hardware). At most one argument is positional; every
  other role and every setting is named.

### Kind values

A kind call **without any role** is a *kind value*: the kind with some settings
filled in. A bare kind name is a kind value with nothing filled in.

```
param slide     : SlideKind                              // a component asks for one
slide:            side_mount_slide(length: slide_length) // a project supplies it
hardware slides[left, right] : slide(side[same], in: opening)   // the component uses it
```

Using a kind value in a `joint` or `hardware` declaration supplies the roles
and may add settings. Giving the same setting twice is
`error[duplicate-name]`. A kind value's settings and fields are readable:
`slide.length`, `slide.clearance`.

### Roles and pairs

- A role is `Part` (exactly one part) or `Parts` (one part or a list).
- A kind with a `Parts` role must have exactly two roles. Such a joint applies
  to **pairs**: one part from the first role and one from the second that
  **meet**. A kind with three or more roles has only `Part` roles; it is one
  connection between exactly those parts.
- Two parts **meet** when their drawn boxes touch over an area (not just along
  an edge) or overlap. They *butt* when they only touch, and *cross* when they
  overlap.
- **Every listed part must meet at least one counterpart**, otherwise it is
  `error[no-contact]`, naming the part. In a joint with three or more roles,
  every part must meet at least one of the others.
- Everything in the kind is evaluated **per pair**: `into.thickness` is the
  thickness of this pair's `into` part, param defaults and `require`s are
  worked out for each pair, and cuts are made for each pair. A setting given
  in the call is the same for all pairs.
- A failing `require` of a kind is an error on the line that uses the joint.

### Defining joints

Joint kinds are not built into the checker. Each one is a `kind ... : joint`
written in `.stave`, usually in a library. The standard joints in `std:joinery`
are written this way too.

```
/// Ari tsugi: a dovetailed end joint. The tail drops into a socket.
kind ari_tsugi : joint {
  roles {
    piece: Part                      // carries the tail
    into:  Part                      // carries the socket
  }
  param neck  = piece.width / 3
  param depth = piece.width / 2
  param angle = 10deg
  require fits : depth <= into.length / 4 "the dovetail is too deep for this beam"

  insert:    [push(piece, along: thickness)]
  cut_first: piece
  adds:      extend(piece, by: depth)

  cut end(into, facing: piece) {
    dovetail_socket(neck: neck, depth: depth, angle: angle)
  }
  cut end(piece, facing: into) {
    dovetail_tail(neck: neck, depth: depth, angle: angle)
  }

  note: "Trace the finished tail onto {into} to mark the socket"
}
```

**`adds:`** — `extend(role, by: d)` lengthens the role's part by `d` at each
end or edge **where it butts against its counterpart in this pair**, toward
the counterpart. The contact decides the direction, so the same kind extends a
shelf in its length and a back panel in its width. `extend` on a pair that
crosses is `error[type-mismatch]`. A list extends several roles:
`adds: [extend(piece, by: d), extend(into, by: d)]`. A turned part is extended
at its end diameter; the joint's cuts then shape the extension. When two
joints extend the same end of a part, the extensions add up.

**`uses:`** — a fastener kind value, with `count:` saying how many each pair
takes (default 1): `uses: dowel_pin(diameter: diameter, length: 2 * depth,
count: count)`. Fasteners appear in the hardware list. They are never placed
and take no part in collision checks.

**`cut_first:`** — a role, or a list of roles in order. The listed roles are
cut in that order, each before the roles after it; roles not listed come after
the listed ones, and parts the joint makes come last. Each later part is
fitted to the earlier ones. `cut_first: into` cuts the mortise or dado first,
then fits the tenon or shelf to it. Left out, the joint puts no order on its
cuts.

**`makes part`** — a part the joint creates (a key, a wedge, a pin), once per
pair. It is declared like any part, appears in the cut list as
`<joint>.<name>`, and is placed **relative to the joint's own parts**:

| Term in a made part's `place` | Meaning |
|---|---|
| `orient(length: a, width: b, of: role)` | the made part's length runs along the role part's axis `a`, its width along axis `b` (`length`, `width` or `thickness`). Required |
| `center(selector)` | centred, on all three axes, in the place a face selector names |
| `center(selector, axis)` | centred there along one axis of the `of` part |
| `against(selector)`, `against(selector, gap: d)`, `flush(selector)`, `flush(selector, inset: d)` | as for parts, against the face the selector names |

Each axis must be fixed exactly once, as everywhere else. The place a
selector names is a plane and a footprint: for parts that butt, the plane
is where their drawn boxes touch; for parts that cross, it is the middle of
their overlap along the face's axis. Along the face, `center` centres on the
overlap of the two parts' cut boxes.

```
makes part key : key_stock {
  length:    piece.thickness + 2 * 10mm
  width:     step
  thickness: key_thickness
  place {
    orient(length: thickness, width: width, of: piece)
    center(crossing(piece, into))
  }
}
```

**Fit and contact.** After every cut is made, the parts of a joint must fit
exactly as drawn: surfaces may **touch**, but wood may not pass through wood.
Two parts touch when their surfaces are within the **contact tolerance** of
0.05mm; anything deeper is `error[overlap]`. So a tight joint (a round section
bearing on four faces, a shelf pushed into a dado) is written at exact sizes
and checks, while a joint whose cuts are too small or in the wrong place is
caught. A joint that needs room to move says so with a param and makes its
cuts that much larger (`clearance:` in `groove` lets a solid panel swell).

There is no way to switch this check off. A joint that uses a cut operation the
geometry can't make yet is `warning[not-supported]`, and the fit of its parts
is checked once it can.

#### Faces to cut

A cut is measured on one face of one role's part, from that face's own edges.
These are measurements within a part, like a profile's stations; world
placement stays coordinate-free.

Every part has a **reference face** (one of the two faces across its
thickness) and a **reference edge** (one of the two faces across its width).
All marking-out measures from them. By default they are the ones that face
`up`, `right` or `front` once placed; a mirrored part mirrors its source's; a
part can choose with `face_side:` and `face_edge:`.

| Selector | The face | Measuring |
|---|---|---|
| `end(p, facing: q)` | the end of `p` nearest `q` | `width` and `thickness` across the end; depth runs along `p`'s length |
| `side(p, normal: width \| thickness, from: q)` | the reference face (`normal: thickness`) or reference edge (`normal: width`) of `p`; `opposite: true` takes the face opposite to it | `along` from the end nearest `q`; `across` from the reference edge (or reference face); `flip: true` measures `across` from the opposite edge |
| `crossing(p, q)` | the face of `p` that `q` touches or enters. The **footprint** is the outline of `q`'s cut-size box on that face | within the footprint, in `q`'s own axes |

When `p` and `q` cross right through each other, `crossing(p, q)` is `p`'s
reference face if it is parallel to both lengths, otherwise `p`'s reference
edge. `normal: width | thickness` names the face instead (the face across
`p`'s width or thickness), for a cut that goes in from another face: a hole
bored down through a stick that another stick passes through vertically is
`crossing(first, lock, normal: thickness)`. `opposite: true` takes the face
opposite to the chosen one.

#### Cut operations

`through` as a depth means all the way through the part.

| Operation | On | Removes |
|---|---|---|
| `notch(depth:)` | crossing | the whole footprint, to `depth` (dados, rabbets, grooves, halvings) |
| `slot(width:, depth:, angle:, from_centre:)` | crossing | a slot `width` wide through the footprint's centre and out past its edges, to the faces of the part. `angle` (default `0deg`) turns it in the face, measured from the entering part's length axis toward its width axis (or from its width axis toward its thickness axis when its length runs into the face). With `from_centre: true` it starts at the footprint's centre and runs out one side only, in the direction of `angle` |
| `pocket(width:, thickness:, depth:)` | crossing, end, side | a rectangular mortise, centred; `width` and `thickness` are in the entering part's axes |
| `tenon(length:, width:, thickness:)` | end | everything around a centred rectangular tenon |
| `spigot(length:, diameter:)` | end | everything around a centred round tenon |
| `round(diameter:, flats:, flats_angle:)` | crossing | turns the part round, to `diameter`, over the length where the other part passes (the footprint's extent along this part's length), leaving square sections either side. With `flats:` it keeps two flat faces that distance apart, their normal at `flats_angle` (default `0deg`) from the part's width axis toward its thickness axis |
| `drill(diameter:, depth:, count:)` | crossing, end, side | `count` round holes (default 1), evenly spaced along the longer side of the footprint or end (equal gaps, including before the first and after the last, as `spaced(evenly)`) and centred across it |
| `dovetail_socket(neck:, depth:, angle:)` | end | a dovetail socket, cut through the thickness |
| `dovetail_tail(neck:, depth:, angle:)` | end | everything around a dovetail tail |
| `outline(marks: [...], depth:, from_depth:)` | side | the shape marked out with `mark(along:, across:)` points, cut from `from_depth` (default `0mm`, the face itself) to `depth` below the face. With `from_depth` greater than zero the cut is hidden inside the part: a recess for a hidden stub tenon |

Turning a part to a profile (`shape: turned(profile)`) is the eleventh operation,
`turning`. It isn't written in a `cut` block, but recipes are written for it
like for the others.

#### Assembly steps

Each step in `insert:` moves one part along or about one of its own axes
(`length`, `width`, `thickness`).

| Step | Meaning |
|---|---|
| `push(p, along: axis)` | move `p` along its axis from wherever it is free until seated |
| `push(p, along: axis, by: d)` | move `p` exactly `d` along its axis into its seated position (a scarf laid one stub length short, then slid home) |
| `push(p, along: axis, angle: a)` | move `p` along a direction turned by `a` from `axis` toward the next axis (`length` → `width` → `thickness` → `length`); `push(lock, along: width, angle: 45deg)` slides along the diagonal of its section |
| `rotate(p, by: angle, about: axis)` | turn `p` in place (about its axis through the middle of its section), e.g. to lock a twist joint |
| `drive(p, along: axis)` | knock in a part the joint makes (a key, wedge or pin) |

A step's axes are the moving part's axes **as placed in the finished joint**,
even when an earlier step turns it, so every direction is fixed by the
drawing and never depends on the order of steps.

**How a sequence is checked: in reverse.** The checker starts from the
finished joint, where every part is at its drawn position, and runs the steps
backwards, last step first, each one in the opposite direction. That gives
every step a definite start: a `by:` step moves exactly `d`; a step without
`by:` moves until the part is clear of the others. Every move is **swept
continuously**, not checked only at its ends: a slide is checked along its
whole path and a turn through its whole angle. If any part would pass through
another (beyond the contact tolerance) at any point, it is `error[order]`
naming the step and the parts. The direction of each step (which way along or
about the axis) is the one that is free; if neither is, that is the error.

A joint's sequence is checked with the parts of one pair (and the parts it
makes) only: the rest of the design is built around them.

Reversing works because each step's start is the previous step's end: the
assembled state is known, and disassembly is just the steps undone. It is also
how the build manual's assembly pictures are drawn.

### Hardware

A `kind` with a hardware base declares a product or a family of products: its
catalog data for the checks, and optionally a 3D model for the viewport.

```
/// A full-extension slide; a family, chosen by length.
kind side_mount_slide : slide {
  param length = 500mm within [250mm, 300mm, 350mm, 400mm, 450mm, 500mm, 550mm, 600mm]
  clearance: 12.7mm
  travel:    length
  envelope:  box(length: length, width: 45mm, thickness: 12.7mm)
}

/// One exact product.
kind brass_knob_26 : pull {
  diameter: 26mm
  envelope: cylinder(diameter: 26mm, length: 30mm)
  model:    model("./models/brass_knob_26.glb", align: node("mount"))
}
```

What each base does:

- **`slide`** — checks that the gap between the piece's mount face and the
  facing side of the `in` region equals `clearance` (`error[slide-gap]`), and
  gives the assembly it is in a motion: it travels toward the front by up to
  `travel`.
- **`hinge`** — a cup hinge bores its cup: one `drill(diameter: cup_diameter,
  depth: cup_depth)` in the mount face. It gives the assembly a motion: the
  door opens by up to `opening_angle` on the hinge's linkage, its one input
  the angle the door has turned; at each angle the door turns by that angle
  and moves wherever its two links put it (forward, away from the cabinet,
  and sideways), and at `0deg` it is exactly where it is placed. The
  linkage's plane is across the long edge of `piece` that is on the face
  opposite the mount face and nearer `on`; its free side is toward the
  middle of the door and it swings toward the face opposite the mount face.
  A butt hinge cuts its two leaf mortises (`notch(depth:` the leaf's
  thickness`)`, one in `piece` and one in `on`) and turns about its pin
  edge, the free side swinging away from the mount face (a lid lifts). A
  hinge has no `envelope:`: its cup, `plate`, arm and links, or its two
  leaves, are its envelope (see [Hardware placement](#hardware-placement)).
- **`pull`** — bores its screw holes through the piece: one hole of
  `screw_diameter` in the middle of the envelope, or two holes `centres` apart
  along its length when `centres` is given.
- **`fastener`** — only ever used through a joint's `uses:`.
- **`tool`** — listed in a project's `tools:` and a recipe's `tools:`.

**Envelopes** are what fit and collision checks use: `box(length:, width:,
thickness:)` or `cylinder(diameter:, length:)`. An envelope may overlap the
hardware's own role parts (a hinge's cup is sunk into its door); overlapping
anything else is `error[overlap]`.

**Models are for looks only.**

- **Format:** glTF 2.0 binary (`.glb`), one file, at most 20 MB.
- **Sources:** the same forms as imports. A relative path is relative to the
  file that declares the kind.
- `align: bounds` sets the model's bounding box onto the envelope (a mismatch
  over 0.5mm is `warning[model-mismatch]`); `align: node("mount")` uses a named
  node as the mounting point. glTF's convention is used: +Y is `up`, +Z is
  `front`, +X is `right`, units are metres.
- `moving:` names the node that moves with the hardware's motion.
- A missing model is drawn as its envelope with a label.

---

## Work cards and recipes

Every part gets a **work card**: the steps to turn stock into that part, drawn
on the part itself. Work cards are generated from the cuts that joints and
hardware declare. A **recipe** says how to make one operation with a given set
of tools.

```
/// A housing cut by hand: saw the walls, chop the waste, level the floor.
recipe notch_by_hand : notch {
  tools: [marking_knife, square, marking_gauge, tenon_saw, chisel, mallet, router_plane]
  rank:  20
  mark {
    knife(walls)                         // square a line across the face at each wall
    gauge(depth, on: open_edges)         // gauge the depth on every edge the notch runs out at
    hatch(waste)
  }
  cut {
    saw(walls, to: depth)
    chop(waste, with: chisel)
    pare(floor, to: depth, with: router_plane)
  }
  check: "the mating part pushes in by hand and leaves no gap at the shoulders"
}
```

The word after `recipe name :` is one of the eleven operations: `notch`, `slot`,
`pocket`, `tenon`, `spigot`, `round`, `drill`, `dovetail_socket`, `dovetail_tail`,
`outline`, `turning`.

### Tools

A tool is a `kind ... : tool`. The standard ones are in `std:tools`; a library
declares its own the same way (`kind sumitsubo : tool { title: "sumitsubo (ink
line)" }`). `tools:` lists are lists of tool kinds, so a misspelt tool is
`error[unknown-name]`.

### Choosing a recipe

For each cut, in this order:

1. **Candidates** are the recipes for that operation in the project's module,
   in every module the project's files import, in the module that declares the
   joint's kind, and in `std:joinery`.
2. Keep those whose `tools:` are all in the project's `tools:` (all of them,
   if the project has no `tools:` field).
3. Take the first that is named in the joint's `methods:` setting; otherwise
   the first named in the project's `methods:`; otherwise the one with the
   lowest `rank:`. Equal ranks are broken by the recipe's module and name, in
   alphabetical order.

A recipe named in `methods:` that is for another operation is simply not a
candidate for this cut, so one list can cover several operations. A cut with
no usable recipe is `warning[no-recipe]`, listing the tools the best-ranked
recipe is missing; the cut still appears on the work card, without steps.
Recipes are chosen before the order of work is planned; there is no fallback
to another recipe if the order can't be satisfied.

### Features

Each operation names the parts of its geometry (features) and its measurements
that a recipe's steps can refer to. A feature is a place on the part (a step
marks, saws or pares it); a measurement is a number (a step sets a tool to it).
**An operation's features and measurements never share a name**, so in a step
like `gauge(flats, on: flat_faces)` every word has one meaning:

| Operation | Features | Measurements |
|---|---|---|
| `notch` | `walls`, `floor`, `waste`, `open_edges` | `depth` |
| `slot` | `walls`, `floor`, `waste`, `open_edges` | `width`, `depth` |
| `pocket` | `walls`, `floor`, `waste`, `outline` | `width`, `thickness`, `depth` |
| `tenon` | `cheeks`, `shoulders`, `waste` | `length`, `width`, `thickness` |
| `spigot` | `shoulder`, `waste`, `centre` | `length`, `diameter` |
| `round` | `shoulders`, `waste`, `centre`, `flat_faces` | `diameter`, `flats`, `length` |
| `drill` | `centre` | `diameter`, `depth` |
| `dovetail_socket` | `walls`, `floor`, `waste`, `outline` | `neck`, `depth`, `angle` |
| `dovetail_tail` | `cheeks`, `shoulders`, `waste`, `outline` | `neck`, `depth`, `angle` |
| `outline` | `outline`, `waste` | `depth` |
| `turning` | `shape`, `stations`, `blank`, `centre` | `length`, `largest` |

### Steps

The first argument of a step is positional. Every step also accepts
`with: <tool>` (one of the recipe's `tools:`), which names the tool on the
card, and `note: "..."`.

**Marking steps** (in `mark { }`):

| Step | Meaning |
|---|---|
| `knife(feature)` | a knife line |
| `pencil(feature)` | a pencil line |
| `line(feature)` | a line made with the tool named by `with:` |
| `gauge(measurement, on: feature)` | a gauged line at that distance from the reference face or edge |
| `square_around(feature)` | carry the line round all four faces |
| `centre_mark(feature)` | crossing lines and an awl dimple |
| `transfer(feature)` | trace the feature from the finished mating cut. The mating cut is then made first |
| `hatch(feature)` | scribble on the waste |
| `template(feature)` | a 1:1 printed template |

**Cutting steps** (in `cut { }`): `saw`, `chop`, `pare`, `plane`, `rout`,
`bore`, `turn`, `shave`. Each takes a feature and accepts:

| Argument | Type | Meaning |
|---|---|---|
| `to:` | a measurement or a feature | where to stop: `saw(walls, to: depth)`, `saw(cheeks, to: shoulders)` |
| `passes:` | `Count` | how many passes to take |
| `guide:` | a feature | what the fence or guide follows |
| `diameter:` | a measurement (`bore` only) | the bit to use |

Sawing is always on the waste side of the line.

### Needs

A recipe can say what must still be true of the part when it runs, which
constrains the order of operations:

| Need | Meaning |
|---|---|
| `flat_face` | the face being cut is still flat (a router or drill rides on it), so this cut comes before cuts that remove that face |
| `square_stock` | the part is still a square blank, so this comes before every other cut that shapes the whole part: `turning` and `outline` |
| `before(operation)` | this cut comes before every cut of that operation on the same part |
| `after(operation)` | this cut comes after every cut of that operation on the same part |

A cut **removes a face** when nothing of that face is left to measure from:
`tenon`, `spigot` and `dovetail_tail` remove the end they are cut on, a
`through` `outline` removes the edges it crosses, and `turning` removes every
face. `notch`, `slot`, `pocket`, `drill` and `dovetail_socket` never remove a
face.

Order between the parts of one joint comes from the joint's `cut_first:`.

### Reference face and edge

Each part's reference face and edge (see [Faces to cut](#faces-to-cut)) are
marked on its card, and every measurement on the card starts from them.

---

## Order of operations

The order of every step, from milling the stock to fitting the hardware, is
worked out by Stave. It is never written in the file: it follows from the
joints, the recipes and the parts' shapes. A joint's `note:` adds advice to the
manual; it never adds or orders steps.

**1. Phases.** The build always runs in this order: mill the stock (flatten,
square and cut to size); mark (reference marks first); cut the joinery; dry fit
each sub-assembly; prepare surfaces that are hard to reach once glued; glue up,
sub-assemblies first; fit hardware; finish. Within the glue-up, assemblies are
ordered by their joints' `insert:` steps.

**2. Rules that must hold** (each one is an edge in a dependency graph):

- Marking a feature comes before cutting it.
- **Reference faces survive.** A cut never removes a face or edge that a later
  step measures from: everything measured from it is marked first.
- **The hole first, then fit the peg**, as each joint's `cut_first:` says.
- A recipe with a `transfer` step comes after the mating cut.
- Recipe `needs:` hold (`flat_face`, `square_stock`, `before`, `after`).
- Matched parts (a part and its mirror image) are marked in one step.

**3. Among valid orders, the least effort.** Orders are scored and the cheapest
wins: fewest tool-setting changes across the whole project, then fewest tool
changes, then least flipping and re-clamping, then risky cuts early. Ties are
broken the same way every time.

**4. Explained and checked.** Every step records why it is where it is. **If no
valid order exists, the design can't be built**, and that is an error on the
part that causes it:

```
error[order]: board: there is no valid order for its cuts
  --> project.stave:44:1
   = the dado must come after the hole (notch_after_holes needs after(drill))
   = the hole must come after the dado (hole_after_notches needs after(notch))
   = help: choose another recipe for one of them with methods:
```

---

## Built-in functions

These are looked up when a bare name is called (see [Names](#names), rule 3).

| Function | Returns | Meaning |
|---|---|---|
| `between(a, b)` | `Length` | the clear distance between two parts, or between two parallel anchors. Two parts must be apart along exactly one axis |
| `extent(items, axis)` | `Length` | overall drawn size of a list of parts along a world axis. With `of: part`, `axis` is one of that part's own axes (`length`, `width`, `thickness`), meaning the world axis it runs along once placed. In a joint kind this measures the other part along this one's direction: `extent(into, thickness, of: piece)` is how much of `into` a screw driven through `piece`'s thickness can bite into (its thickness for a face, its width for an edge) |
| `mid(item, axis)` | `Anchor` | the mid-plane of a part or region |
| `opening(...)`, `split(...)`, `cover(...)` | `Region` | see [Regions](#regions) |
| `overlay(...)`, `inset(reveal:)` | `Mount` | see [Regions](#regions) |
| `turned(profile)` | `Shape` | turned on a lathe to a profile |
| `box(length:, width:, thickness:)` | `Envelope` | a hardware envelope |
| `cylinder(diameter:, length:)` | `Envelope` | a hardware envelope; its axis stands off the mount face |
| `model(source, align:)` | `Model` | a `.glb` model; `align: bounds` or `align: node("name")` |
| `ends(inset:)`, `pitch(d)`, `gap(d)` | `Spacing` | used by `spaced`, with the bare words `evenly` and `ends` |

Words that look like functions but are fixed by position, not built-in
functions: placement terms, face selectors, cut operations, `mark(...)` inside
`outline`, assembly steps, `extend`, recipe steps, `before`/`after`, and
`mirror`.

---

## Scenes

A scene is a named view of the same design: motion states and visibility.
Scenes never change the design. Every project also has the built-in scene
`rest` ("As built"), with everything in its rest position; declaring a scene
called `rest` is `error[duplicate-name]`.

```
scene all_open "All open" {
  pose door_l = open
  pose door_r = open
  pose drawer[top] = open
}

scene drawer_bench "Top drawer on the bench" {
  show:    drawer[top]
  explode: true
}
```

- Placement sets the **rest position** (doors closed, drawers shut).
- **Motions belong to assemblies.** The slides, or the hinges, declared inside
  a component give each instance one motion: the whole assembly moves, as one
  piece, relative to everything outside it. All hinges of an assembly must
  share one pivot line and all slides one direction (`error[motion]`
  otherwise); an assembly with both hinges and slides has no motion.
- `pose` names the assembly and sets `open` (the hinge's `opening_angle`, the
  slide's `travel`), `closed`, an angle (hinges) or a length (slides). A slide
  travels along its piece's length, out through the front of its `in` region.
  A mirrored assembly has the mirror image of its source's motion.
- Every scene is collision checked, and every motion is checked through its
  whole travel, not only at its ends (`error[motion]`): a door is swung from
  closed to `opening_angle` and a drawer slid from shut to `travel`, and the
  message names the first angle or distance at which it hits something. Each
  motion is checked from the rest position and from every scene's poses. A
  door on cup hinges follows its linkage (it turns and moves at once), and
  the hinges' own arms and links are checked against the door and the side
  too; the first contact is found to within the 0.05mm contact tolerance,
  never stepped over. A door on butt hinges turns about its pin.
- `explode: true` draws the scene with every part moved out from its rest
  position along the direction its joint is assembled (the reverse of the
  joint's last `insert` step), so the joinery shows. Assemblies move out
  first, each as a whole (a drawer forward along its slides, clear of its
  depth; a door or lid off its hinges, toward the side it swings to; any
  other along the joint that attaches it), then the parts inside each
  assembly, and the parts in none, move apart about where it went. A part
  that would
  leave two of its pairs in opposite directions (a rail tenoned into two
  stiles) stays, and the parts it joins move away from it instead; a
  part's moves in different directions add up, a part coming out of a
  part that moves goes with it, and hardware goes with its part.
  Exploding is a drawing only: it is never collision checked.

---

## Checks and `require`

`require` states something about the design that must stay true:

```
require hanger_room : cabinet_depth - back.thickness >= 540mm "room for coat hangers"
```

**A require must hold over the whole declared range** of every param it
depends on, not just at the current values. When it fails only elsewhere in
the range, the error names a failing value and offers the fix:

```
error[require-range]: 'hanger_room' does not hold for every allowed value: room for coat hangers
  --> cabinet.stave:27:1
   = it fails when cabinet_depth = 450mm
   = help: tighten the range: param cabinet_depth = 560mm within 546mm..650mm
```

Requires are what an AI patch or a smart edit is judged against: a change that
breaks one is rejected, with the message.

Checking runs in layers; a layer runs only if the ones before it found no
errors in the same item.

1. **Syntax**: this grammar.
2. **Names and types**: names resolve, units match, fields and roles exist.
3. **Placement and sizes**: every axis set once, `fit` resolvable, no cycles.
4. **Fit and geometry**: parts meet where joints say they do and overlap
   nowhere else (using cut sizes and cuts); slides have their clearance.
5. **Motion**: every scene, and every motion through its whole travel, is
   collision free.
6. **Requires and ranges** (z3): every `require` holds, and layers 3 and 4 hold,
   for every value inside each `within` range.
7. **Buildability**: an order of operations exists. See
   [Order of operations](#order-of-operations).

### Diagnostics

A diagnostic is printed compiler-style: `error[code]: message`, then
`--> file:line:column`, then notes and a concrete `help:` line. Codes and the
shape of their messages are fixed, because tests and AI tools match on them.

| Layer | Code | Message |
|---|---|---|
| 1 | `syntax` | `expected <what>, found <token>`; several expectations are joined with "or" |
| 1 | `missing-value` | `'<name>' has no value` |
| 1 | `reserved-word` | `'<word>' is a reserved word and can't be used as a name` |
| 1 | `unknown-unit` | `unknown unit '<unit>' (expected one of mm, cm, m, in, ft, deg)` |
| 1 | `unclosed-bracket` | `this "<bracket>" is never closed` |
| 1 | `missing-brace` | `<the body of part 'side_l'> is missing its closing "}"` |
| 2 | `unknown-name` | `unknown name '<name>'` |
| 2 | `duplicate-name` | `'<name>' is declared twice` |
| 2 | `ambiguous-name` | `'<name>' could be <this> or <that> here` |
| 2 | `unit-mismatch` | `can't <add> <a length> and <an angle>`, or `'<field>' needs <a length>, found <a count>` |
| 2 | `type-mismatch` | `'<field>' needs <a part>, found <a region>` |
| 2 | `unknown-field` | `<a part> has no field '<name>'` |
| 2 | `missing-field` | `<what> is missing '<name>'` |
| 2 | `out-of-range` | `'<param>' must be within <domain>, found <value>` |
| 2 | `not-in-library` | `a library can't contain a <part> at the top level` |
| 2 | `import-cycle` | `modules import each other: <a> -> <b> -> <a>` |
| 2 | `unknown-module` | `can't find module '<source>'` |
| 2, 4 | `not-supported` | `<what> is not supported yet` (an error for remote imports and for a format version other than 1, a warning for geometry) |
| 3 | `ground` | `no part is placed with 'ground'`, `'<a>' and '<b>' are both placed with 'ground'`, or `'<a>' is placed with 'ground' inside a component` |
| 3 | `axis-not-set` | `<item>: position along '<axis>' is not set` |
| 3 | `axis-set-twice` | `<item>: position along '<axis>' is set twice` |
| 3 | `placement-cycle` | `<a> and <b> are placed against each other` |
| 3 | `fit-undecided` | `<part>: nothing decides its <length>` |
| 3 | `mirror` | `<item> has no counterpart on the other side of the mirror` |
| 4 | `no-contact` | `<joint>: <part> meets none of the parts it should join` |
| 4 | `overlap` | `<a> and <b> overlap` |
| 4 | `slide-gap` | `<hardware>: <piece> is <gap> from the opening, but the slide needs <clearance>` |
| 4 | `hinge-overlay` | `<hardware>: <piece> overlays <on> by <overlay>, but the hinge gives <low> to <high>` |
| 5 | `motion` | `<a> hits <b> when <assembly> is <open>` |
| 6 | `require` | `'<name>' fails: <message>` |
| 6 | `require-range` | `'<name>' does not hold for every allowed value: <message>` |
| 6 | `range` | `<what> stops working when <param> <is below> <value>` |
| 7 | `order` | `<part>: there is no valid order for its cuts`, or for a joint's insert steps `<joint>: <step> is blocked by <part>` |

Warnings and notes: `shadows-prelude`, `no-recipe`, `not-supported`,
`model-mismatch`.

In an `overlap` message the parts of one joint pair are named in the
joint's role order (`rail and leg`), a piece of hardware before a part, and
otherwise in declaration order. When parts of an assembly run into something
outside it, that is one error for the assembly and the other item
(`door_l and top overlap`), reported where the assembly is placed (its
`Mount` argument when it has one, otherwise its name) with the
component's parts labelled; its mirror image is a note on the same error.
A motion that hits something is reported at the same place. In a `motion` message for a scene, `<open>`
is the pose: `open`, `open 90deg` (a hinge turned to an angle) or
`out 200mm` (a slide travelled a length); when two assemblies move, both are
named (`door_l is open and drawer[top] is out 200mm`). For a motion swept
through its travel, `<open>` is where it first hits: `opened past 87.3deg`
(rounded down to a tenth of a degree) or `pulled out past 120mm`, and a note
gives how far the hinges open or the slides travel; each assembly is
reported once, from rest or else from the first scene whose poses make it
hit. A cup hinge's own arm or link that runs into wood is named with its
hinge: `the outer link of door_l.hinges[lower] hits door_l.stile[hinge] when
door_l is opened past 108.2deg`, once per assembly. In an `order` message, `<step>` is written as in the source
(`push(rail, along: width)`).

In messages, an item is written as in the source (`slides[left]`,
`drawer[top].front`), and a length in the project's units with trailing zeros
trimmed (`12.7mm`). In a `syntax` message, `<token>` is the token's text in
single quotes (`'shelf'`, `':'`), or `end of line`, `end of file`, `a string`
or `a doc comment`.

**The broken-design corpus.** Each folder in `tests/broken/` is a small project
whose first line is `// expect: error[<code>] <message>` (or
`// expect: warning[<code>] <message>` for a warning). The checker must
produce a diagnostic with exactly that code and message (it may produce
others). A design for a check that isn't built yet says so on its second
line, `// pending: <reason>`; until the check lands it must check without any
diagnostic, and the line comes off when it does. `docs/test-coverage.md` maps
every rule of this document and every code above to the tests that pin it
down.

---

## Tooling and editors

The language is designed to be served by a language server (`stave-lsp`, built
on `stave-app` like the CLI and the GTK app) and edited in any editor. These
requirements bind the implementation.

**A lossless syntax tree.** The parser produces a full-fidelity tree that keeps
every byte of the source: whitespace, newlines, comments, doc comments and the
pieces of erroneous code. Printing it gives back the file unchanged. The typed
syntax tree (`ast`) is a view over it. The formatter, renames, quick fixes and
patches edit this tree, so they never lose comments or layout.

**Recovery points.** Every item starts with a keyword on a new line, every field
ends at the end of its line, and blocks use braces. The parser resynchronises at
these points ([Unclosed brackets](#unclosed-brackets) covers brackets), so one
mistake produces one error and the rest of the file still parses, resolves and
checks. A half-typed line while editing is the normal case, not an edge case.

**Positions.** Spans are byte offsets into a file. Line and column, and the
UTF-16 offsets the language server protocol uses, are computed at the edge
(the CLI renderer, the language server), never stored in the tree.

**Fast and slow diagnostics.** Checks are split by cost:

| Tier | Layers | When |
|---|---|---|
| Fast | 1 syntax, 2 types and names | on every change, within one keystroke |
| Slow | 3 placement and sizes, 4 fit, 5 motion, 6 requires and ranges (z3), 7 buildability | in the background after typing pauses; cancelled and restarted by the next change |

Results are computed incrementally (query-based, as in rust-analyzer's `salsa`):
a change to one file re-runs only what depends on it, and determinism makes
every result cacheable.

**Modules in an editor.** A module is every `.stave` file in its folder, so the
server uses the text of open, unsaved editor buffers in place of their files on
disk. The standard library is served as read-only documents (`stave-std:///joinery/dado.stave`),
so go-to-definition works on `dado`, `side_mount_slide` and every built-in
kind. Remote modules are never fetched by the server; an unfetched one is a
diagnostic with a "fetch" code action.

**What an editor gets:**

| Feature | From |
|---|---|
| Diagnostics | the fast and slow tiers above, with codes from the [Diagnostics](#diagnostics) table |
| Quick fixes | each diagnostic's `help:` line, as an edit when it is mechanical ("add `as hw`", "write `name[*]`", "add `)`") |
| Completion | the position decides the vocabulary: placement terms at the start of a line in `place { }`; cut operations after `cut`; base kinds after `kind name :`; the declaration's field table in a field position; the expected type's words in an argument (`up`, `evenly`, `plywood`); a module's public items after `module.`; labels inside `[ ]` |
| Signature help | the roles and params of the kind or component being called, with defaults |
| Hover | doc comments, types, and values worked out in the project's units |
| Inlay hints | computed values next to their source: `length: fit` shows `= 844mm`, `between(base, top)` its value, every size its cut length |
| Go to definition, references, rename | every name resolves to exactly one declaration (including names inside strings and qualified names across modules) |
| Document symbols, folding | items, and the blocks inside them |
| Semantic colouring | the namespace of each word: item kinds (part, joint, param, region, ...), members, fixed-position call heads, enum words, units |
| Formatting | `stave fmt`, applied to the lossless tree (see "Layout" below) |
| Code actions from AI | a checked patch from an AI or the suggestion engine is offered as a workspace edit |
| Code lens | "show in 3D" on parts and assemblies, "show work card" on parts |

**Layout.** `stave fmt` keeps every comment and every line break (a newline
ends a field, so lines are never joined or split) and normalises the rest:

- two spaces of indentation per open `{`; a line that continues an expression
  is indented one step past the line that opened its bracket when the
  bracket ends that line, otherwise just after the bracket, and outside
  brackets it lines up with the value after the line's first `:` or `=`;
- one space between tokens, except none inside `( )` and `[ ]`, before `,`,
  around `.` and `..`, after a prefix `-` or `!`, before the `(` of a call
  or the `[` of an index or array, and before the `:` of a field, argument,
  role or station; a declaration's `:` has a space on each side
  (`part side_l : oak_19`);
- consecutive lines that each start with a field, a named argument or a
  profile station, in the same block or argument list, line their values up
  one space after the longest key; a blank line or another kind of entry
  ends the run;
- consecutive one-line items with the same keyword in the same scope (a
  file, a component or a kind body), with no blank, comment or doc comment
  line between them, pad their names so that the token after the name lines
  up one space after the longest: the `=` or `:` of params, the `:` of
  joints, kinds and requires, the `{` of stocks, the `=` of regions. A name
  includes its array labels (`bay[drawers, doors]`). Nothing after that
  token is aligned (not `within`, not `locked`), except trailing comments;
- a comment at the end of a line keeps its column when there is room;
- at most one blank line in a row, none at the start or end of a block or
  of the file.

---

## Smart edits

The mouse tools in the app edit the design through relationships. A **resize**
drag on a part asks: which unlocked values, if changed, give this new size?

- Every size is an expression that leads back to params. Dragging the top
  drawer 50mm wider traces its box width through `opening.across` to
  `cabinet_width`.
- z3 finds the change. Among the unlocked params the size depends on, it picks
  the smallest change to the param closest to the dragged part, keeps every
  `require` and `within` range, and offers the other answers as alternatives.
- **Locked values stop the chain**: params marked `locked`, stock thickness, and
  hardware sizes from the catalog (such as slide clearance).

  ```
  param reveal = 1.5mm locked
  ```

- **A plain number is edited in place.** If the size is written as a number
  (`width: 400mm`), the drag changes that number and nothing else follows.
- The result is an ordinary patch (`set cabinet_width = 950mm`).

**Move** drags edit the measured offset of one placement term, and **snap**
replaces a term with `flush` or `against` on the face the part was dropped on.

---

## Types, units and arithmetic

- **Values are exact fractions.** A literal such as `1/3in` or `0.1mm` is an
  exact rational number of nanometres; `1in / 3` and `spaced(evenly)` never
  lose precision. Values are rounded **once**, when the cut list is made, to
  the project's `resolution:`, and the cut list says where it rounded.
- Stored results (the solved design) hold lengths as whole nanometres and
  angles as whole micro-degrees.
- Arithmetic: `length ± length`, `length × ratio`, `ratio × length`,
  `length ÷ ratio`, `length ÷ length → ratio`, `ratio ± × ÷ ratio`, the same
  for angles, unary `-`. Comparisons need the same type on both sides and give
  a `Bool`; `&&`, `||` and `!` take `Bool`s (anything else is
  `error[type-mismatch]`: `'&&' needs true or false, found a length`). Anything
  else (`count ÷ length`, `length + angle`, `count - length`) is
  `error[unit-mismatch]`. Dividing by zero is `error[type-mismatch]`
  (`division by zero`).
- An integer literal is a `Count`; a `Count` can be used wherever a `Ratio` is
  expected.

The complete list of types. A `param` may name any of them.

| Type | Values |
|---|---|
| `Length` | `19mm`, `3/4in`, `between(a, b)` |
| `Angle` | `110deg` |
| `Ratio` | `0.5`, `2 / 3` |
| `Count` | `4` |
| `Bool` | `true`, `false`, comparisons |
| `Text` | `"..."` |
| `Part` | one part |
| `Parts` | one part or a list of parts |
| `Stock` | a `stock` item |
| `Region` | a region |
| `Profile` | a `profile` item |
| `Anchor` | a face (`base.top`) or mid-plane (`mid(base, across)`) |
| `Axis` | `up`, `across`, `deep` |
| `Direction` | `up`, `down`, `left`, `right`, `front`, `back` |
| `Mount` | `overlay(...)`, `inset(...)` |
| `Shape` | `turned(profile)` |
| `Envelope` | `box(...)`, `cylinder(...)` |
| `Model` | `model(...)` |
| `Spacing` | `evenly`, `ends`, `ends(inset: d)`, `pitch(d)`, `gap(d)` |
| `JointKind` | a kind value of base `joint` |
| `HardwareKind` | a kind value of base `slide`, `hinge` or `pull` |
| `SlideKind`, `HingeKind`, `PullKind`, `FastenerKind` | a kind value of that base |
| `Tool` | a kind of base `tool` |
| `Recipe` | a `recipe` item |

Closed word lists that aren't param types (they only appear where a field or
argument expects them): face words (`top`, `bottom`, `left`, `right`, `front`,
`back`), part axes (`length`, `width`, `thickness`), units, species, forms,
`diameter` \| `radius`, `open` \| `closed`, `through`, `bounds`.

### What z3 can check over a range

z3 works with exact real arithmetic over the params. For a param with a
`within` range, the checker rejects expressions it can't verify over the whole
range, and says so: a param multiplied by a param, division by a param, and
`spaced(evenly)` over an array whose count is a param. Each assertion is tracked
by its source span, so a failure points at the line that caused it.

---

## Patches

A patch is how an AI, a suggestion or a mouse tool changes a design. It uses
the same grammar as the design.

```
patch "Add a shoe shelf" {
  add part shoe_shelf : oak_19 {
    length: fit
    width:  cabinet_depth - back.thickness
    place {
      against(divider.top, gap: 200mm)
      center(base, across)
      flush(side_l.front)
    }
  } in "carcass.stave"
  add joint shoe_shelf_dados : dado(shoe_shelf, into: [side_l, side_r], depth: 9mm)
  set shelf_dados.depth = 8mm
  set side_l.place.deep = flush(base.back, inset: 6mm)
  unset drawer.pull_kind
  rename part side_l to left_side
}
```

| Operation | Does |
|---|---|
| `add <item>` | adds an item to the project's module. `to <component>` adds it inside a component (`add part ... to casework.Drawer`); `in "file.stave"` picks the file, otherwise the file of the first item it refers to |
| `add import "..." in "file.stave"` | adds an import to a file |
| `replace <item>` | replaces the item of the same kind and name as a whole. This is how things without a name of their own are changed: a profile's stations, a kind's cuts, a scene's poses |
| `set <path> = <expr>` | sets a value, adding it if it isn't there yet |
| `unset <path>` | removes a field, an argument or a placement term, so its default applies |
| `remove <tag> <path>` | removes an item; the path may reach into a component (`remove part casework.Drawer.back`) |
| `remove import "..." in "file.stave"` | removes an import |
| `rename <tag> <path> to <name>` | renames an item and every reference to it |

What a `set` or `unset` path can address:

| Path | Addresses |
|---|---|
| `cabinet_width` | the default of a `param`, or the value of a `let` |
| `shelf.width`, `oak_19.thickness`, `all_open.explode` | a field of an item |
| `shelf_dados.depth`, `shelf_dados.into` | a setting or role in a joint's or hardware's kind call |
| `drawer.pull_kind`, `drawer[top].slide` | an argument of an assembly |
| `side_l.place.up`, `.place.across`, `.place.deep` | the placement term that fixes that axis (each axis has exactly one). A term that fixes two axes (`corners`) is replaced as a whole |
| `side_l.place.orient` | the `orient` term |
| `casework.Drawer.box_height` | the same, inside a component of a local module |

A patch may edit modules that are local folders. A module from a remote source
is read-only (`error[not-supported]`). After a patch is applied, the whole
design is checked again; a patch that introduces an error is rejected with
that error.
