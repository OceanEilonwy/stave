# The `.stave` language

Version 1, draft 3.

A `.stave` file describes a woodworking project as **intent**: parameters,
stock, parts, joints, hardware and how parts relate to each other. Everything
derived (positions, meshes, cut list, build manual, order of work) is computed
and never stored.

Draft 3 applies `docs/reviews/2026-11-16-language-review-2.md`. What changed
from draft 2 is listed in that file's "Resolution" section.

Graphical editing ([`docs/editing.md`](editing.md), decided 2026-11-18)
adds to draft 3: param kinds written as a prefix (`locked param`,
`solved param`; see [`param`](#param-name--type--default-within-domain-with-locked-or-solved-before-it)),
a root region made from sizes (`box(across:, up:, deep:)`; see
[Placement rules](#placement-rules-checked)), relative sizes for `split`
(`shares:`; see [Regions](#regions)), `Part`, `Parts` and `Anchor`
component params (see [`assembly`](#assembly-name--component--)), smart
edits over the param kinds (see [Smart edits](#smart-edits)) and fixes
carried as patches (see [Fixes](#fixes)). These are specified here ahead
of the code: where a rule is not built yet, the text says so, and
`docs/test-coverage.md` lists it as pending.

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
place pose at within locked solved fit true false
```

`locked` and `solved` start a param of one of the
[param kinds](#param-name--type--default-within-domain-with-locked-or-solved-before-it)
(`solved param overall_width = ...`).

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
| `mechanism` | at the start of an entry in a hardware `kind` body, when not followed by `:` |
| `adjust`, `on` | `adjust` at the start of an entry in a hardware `kind` body, when not followed by `:`; `on` between its two members (`adjust arm on plate: across within 2mm`) |
| `mount` | at the start of an entry in a hardware `kind` body or a member's body, when not followed by `:` |
| `shape` | at the start of an entry in a member's body, when followed by a name and `:` (`shape knuckle: box(...)`); `shape: ...` is a field |
| `pivot`, `slot`, `rail`, `couple` | at the start of an entry in a `mechanism` block, when not followed by `:` |
| `from`, `along` | inside a `slot` (`from point(...) to point(...)`) and a `rail` (`along across`) |
| `mark`, `cut` | at the start of an entry in a `recipe` body, when not followed by `:` |
| `same` | as the whole content of an index: `drawer_bay[same]` |
| `to`, `in` | after the item of a patch's `add`, and in `rename ... to`; `to` also in a `slot` and a `couple`; `in` also between two numbers, a slope: `1 in 8` |
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
`library`, `private`, `locked`, `solved`, `kind`, `recipe`, `stock`,
`profile`, `component`, `scene`, `param`, `let`, `region`, `part`, `joint`,
`hardware`, `assembly`, `require`, `patch`.
`locked` and `solved`, like `private`, come before the keyword of a
declaration (`locked param reveal = 1.5mm`), so a line that starts with one
starts a declaration.

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
item          = { DOC } , [ "private" ] , item_kind ;   (* private first: private solved param ... *)
item_kind     = project | library | kind | recipe | stock | profile | component
              | scene | design_item ;
design_item   = param | let | region | part | joint | hardware | assembly | require ;

project       = "project" , STRING , block(field) ;
library       = "library" , STRING , [ block(field) ] ;

kind          = "kind" , IDENT , ":" , base_kind , block(kind_entry) ;
base_kind     = "joint" | "slide" | "hinge" | "pull" | "fastener" | "tool" ;
kind_entry    = { DOC } , ( param | require | makes | member )
              | roles | cut | mount | mechanism | adjust | field ;
member        = "member" , IDENT , [ ":" , expr ] , [ block(member_entry) ] ;
                (* a kind's piece: member arm { shape: box(...) }, or in a joint kind
                   member pin : dowel_pin(...) { on: contact(piece, into), ... };
                   without a block it is error[missing-field] *)
member_entry  = "shape" , [ IDENT ] , ":" , call_expr    (* shape: cylinder(...), shape knuckle: box(...) *)
              | mount                                    (* mount { depth: from(face(on, toward: piece)) } *)
              | field ;                                  (* with: piece; on:, count:, spaced: in a joint kind *)
mount         = "mount" , block(field) ;                 (* where hardware sits: mount { face: back, along: open } *)
call_expr     = IDENT , "(" , [ args ] , ")" ;           (* an expression that is one call *)
mechanism     = "mechanism" , block(mechanism_entry) ;   (* how a hardware kind's members move *)
mechanism_entry = "pivot" , IDENT , "," , IDENT , "at" , call_expr
                                                         (* pivot arm, inner_link at point(...) *)
              | "slot" , IDENT , "," , IDENT , "from" , call_expr , "to" , call_expr
              | "rail" , IDENT , "," , IDENT , "along" , IDENT
              | "couple" , IDENT , "to" , IDENT , ":" , expr   (* couple middle to drawer_member: 1/2 *)
              | "stop" , ":" , expr , STRING             (* stop: opening_angle "the limiter stops it here" *)
              | field ;                                  (* drive: turn(cup, from: 0deg, to: 110deg) *)
adjust        = "adjust" , IDENT , "on" , IDENT , ":" , IDENT , "within" , expr ;
                                                         (* adjust arm on plate: across within 2mm *)
roles         = "roles" , block(role) ;
role          = NAME , ":" , IDENT ;                     (* piece: Part   into: Parts *)
cut           = "cut" , [ IDENT , ":" ] , call , block(call) ;
                (* cut end(piece, facing: into) { tenon(...) }; named: cut dado: crossing(into, piece) { ... } *)
makes         = "makes" , "part" , IDENT , ":" , ref , block(part_entry) ;

recipe        = "recipe" , IDENT , ":" , IDENT , block(recipe_entry) ;
                (* recipe notch_by_hand : notch { }   — how to make one cut operation *)
recipe_entry  = "mark" , block(call)                     (* marking-out steps *)
              | "cut" , block(call)                      (* cutting steps *)
              | field ;                                  (* tools: [...], rank: 10, needs: [...], check: "..." *)

param         = [ "locked" | "solved" ] , "param" , IDENT , [ ":" , IDENT ] ,
                [ "=" , expr ] , [ "within" , domain ] ; (* a type, a default, or both;
                   the prefix is the param's kind: locked param reveal = 1.5mm *)
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
arg           = [ NAME , ":" ] , expr , [ ".." , expr ] ;
                (* named when a NAME is directly followed by ":"; a range lo..hi is
                   allowed only as an argument of a member's shape *)
list          = "[" , [ expr , { "," , expr } , [ "," ] ] , "]" ;

primary       = slope | QUANTITY | NUMBER | STRING | "true" | "false" | "fit"
              | IDENT | list | "(" , expr , ")" ;
slope         = NUMBER , "in" , NUMBER ;                 (* a Ratio: 1 in 8 is 1/8 *)

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
- `locked` and `solved` are followed only by `param`. Anything else after
  them is `error[syntax]` (`expected 'param', found 'let'`), and so is
  `private` after them (`locked private param` is written
  `private locked param`) or both of them on one param
  (`expected 'param', found 'solved'`, with the help "a param is locked or
  solved, not both"). The trailing `locked` of earlier drafts
  (`param reveal = 1.5mm locked`) is `error[syntax]`
  (`expected end of line, found 'locked'`) with the help
  `write 'locked' first: locked param reveal = 1.5mm` (the help repeats
  the param as written, with `locked` moved to the front).
- A `block(part_entry)` and a `block(place)` hold at most one `place`; a second
  one is `error[syntax]`. A joint has no body: `joint x : dado(...) { ... }` is
  `error[syntax]` with the help "a joint's settings go inside the parentheses".
- `mirror` takes exactly a source and `about:`. `part x = <anything else>` is
  `error[syntax]`: a derived part is always a mirror image.
- There are no object literals. `{ }` appears only after a declaration header.
  Inside an expression, everything is a call: `euro_hinge(opening_angle: 110deg)`.
- An empty `block` is fine: `scene closed "Closed" { }`.
- `stop: value "message"` in a `mechanism` is the one entry that looks like a
  field but carries a message after its value, as a `require` does.
- `shape body: box(...)` (the word `shape`, a name, then `:`) names one of a
  member's shapes; `shape: box(...)` is a field and gives the member's only
  shape.
- `cut dado: crossing(...) { }` (the word `cut`, a name, then `:`) is a named
  cut; `cut crossing(...) { }` is an unnamed one (`warning[unnamed-cut]` in a
  kind; see [Named cuts](#named-cuts)), and `cut: ...` is a field.
- `mount { }` is a block when `mount` is directly followed by `{`;
  `mount: ...` is a field.

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
  hardware/                // library module; 'hardware' is a keyword, so: import "./hardware" as hw
    library.stave          //   library "Shop hardware"
    handles.stave
```

### Imports

```
import "std:casework"
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
| `std:joinery` | `std/joinery` | joint kinds (`dado`, `rabbet`, `groove`, `dowel`, `round_tenon`, `socket`, `screw`, `cope_and_stick`, `mortise_tenon`, `half_lap`, `glued_face`), edge joints for parts made of boards (`glued_edge`, `tongue_and_groove`, `splined_edge`, `chigiri`, `suitsuki_zan`; see [Boards](#boards)), splices (`lap_splice`, `kanawa_tsugi`; see [Splices](#splices)) and the standard recipes |
| `std:hardware` | `std/hardware` | hardware kinds (`side_mount_slide`, `euro_hinge`, `wide_hinge_165`, `butt_hinge`, `knob`, `bar_pull`) and fasteners (`dowel_pin`, `wood_screw`) |
| `std:tools` | `std/tools` | tool kinds: marking and measuring (`pencil`, `marking_knife`, `square`, `tape`, `marking_gauge`, `mortise_gauge`, `sliding_bevel`, `dividers`, `calipers`, `awl`), hand tools (`tenon_saw`, `dovetail_saw`, `chisel`, `mortise_chisel`, `mallet`, `router_plane`, `shoulder_plane`, `block_plane`, `spokeshave`, `rasp`, `brace`, `auger_bit`, `screwdriver`, `clamps`), power tools (`drill`, `depth_stop`, `router`, `straight_bit`, `dovetail_bit`, `table_saw`, `dado_stack`, `bandsaw`) and turning (`lathe`, `roughing_gouge`, `spindle_gouge`, `parting_tool`) |

Beginners write `dado(...)`, not `joinery.dado(...)`. Anything from outside
`std` is always qualified (`japanese.ari_tsugi`), so a joint from someone else
is visible as such. The three modules are written in `.stave` like any other
library, may not declare the same name twice between them, and change only
with the format version in the `stave` header. They can also be imported
explicitly (`import "std:joinery"`, `import "std:hardware" as stdhw`) to reach
an item that a local name hides.

Other standard modules ship with Stave but are not in the prelude: a file
imports them and uses their items qualified, like any library.

| Module | Folder | Provides |
|---|---|---|
| `std:casework` | `std/casework` | cabinet components (`Case`, `Back`, `Divider`, `Shelf`, `HangingRail`, `Feet`, `Drawer`, `FramePanelDoor`), used as `casework.Drawer` after `import "std:casework"` |
| `std:frames` | `std/frames` | table and stand components (`Top`, `Legs`, `Apron`, `Stretcher`, `Cleat`, `Slats`), used as `frames.Legs` after `import "std:frames"` |

A standard module never reaches into a project: everything a component needs
from one (the opening, the stock, the shop's own hardware) is a param.

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
| `boards` | `Boards` | no (default: the prelude's, `joined(by: glued_edge)`) | how the boards of a part made of boards are joined, for every part that doesn't say; see [Boards](#boards) |

### `library "Title" { }`

| Field | Type | Required | Meaning |
|---|---|---|---|
| `about` | `Text` | no | one line of description |
| `license` | `Text` | no | credited in the build manual |
| `boards` | `Boards` | no | in a library in the prelude: how a part's boards are joined when neither the part nor its project says; see [Boards](#boards) |

### `param name [: Type] [= default] [within domain]`, with `locked` or `solved` before it

The same form everywhere: in a project or library module, in a `component` and
in a `kind`.

```
locked param reveal        = 1.5mm
       param overall_depth = 560mm within 550mm..650mm
solved param overall_width = 900mm within 600mm..1200mm
```

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

**Param kinds.** A param is one of three kinds, written as a prefix. They
say only what a [smart edit](#smart-edits) may do with the value; checking
is the same for all three.

| Kind | Written | A smart edit |
|---|---|---|
| plain | `param x = ...` | changes it only when it is the single param the edited size depends on, or when the user picks it from the list of candidates. The new value is worked out with exact arithmetic, without z3 |
| locked | `locked param x = ...` | never changes it: the chain of a smart edit stops there |
| solved | `solved param x = ... within ...` | may solve for it with z3, alone or with other solved params, keeping every `require` and `within`. Its default is where the solve starts |

- `private` comes first when both are written: `private solved param x`.
- `locked` and `solved` need a param that has a default and whose type is a
  quantity: `Length`, `Angle`, `Ratio` or `Count`. On a param without a
  default it is `error[missing-value]`
  (`'opening' has no value`, with the note "a locked param needs a
  default" and the help to give one or remove `locked`). On a param of
  another type (`locked param frame : Stock = oak_19`) it is
  `error[type-mismatch]` (`'locked' needs a length, an angle, a ratio or a
  count, found a stock`). A param without a default is reported first, so
  `locked param opening : Region` is one error, `missing-value`.
- A `solved param` needs a `within` domain, the values the solver may
  choose from: without one it is `error[missing-field]`
  (`solved param 'overall_width' is missing 'within'`).
- The trailing `locked` of earlier drafts (`param reveal = 1.5mm locked`)
  is `error[syntax]`; see the notes under [Syntactic grammar](#syntactic-grammar).

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
| `max_width` | `Length` | no | yes | the widest board you can get. A `solid` part wider than this is made of boards (see [Boards](#boards)); a `plywood` or `mdf` part wider than this is `error[type-mismatch]` |
| `max_length` | `Length` | no, but see the warning below | yes | the longest board (or sheet) you can get; a part whose cut length is longer is `error[type-mismatch]`, unless its `lengths:` splices it into pieces that each fit (see [Splices](#splices)) |

A `solid`, `plywood` or `mdf` stock that a part is cut from and that has no
`max_length` is `warning[missing-field]` at the stock, once: `oak_19 doesn't
say how long its boards are, so Stave can't check part lengths` (`sheets`
for plywood and MDF), with the help `add the longest board you can buy to
oak_19, for example max_length: 2400mm` (`sheet` and `2440mm` for sheets).
Without it no part's length can be checked. Sheets get no default size: a
part may lie either way on a sheet, and `max_width` and `max_length` are
measured along the part's grain, so a standard sheet isn't one clean pair of
limits. Round stock (`dowel`) is left out.

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
| `boards` | `Boards` | no (default: the project's) | no | how its boards are joined, when it is made of boards; see [Boards](#boards) |
| `lengths` | `Splices` | no | no | where it is spliced end to end into pieces, and by what joint, when it is longer than its stock: `spliced(by: kind, at: d, from: end)`; see [Splices](#splices) |
| `loose` | `Bool` | no (default `false`) | no | `true` for a part meant to lift out (a loose shelf, a tray), which nothing needs to hold; see [Held](#held) |
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

**Place params.** A component may take `Region`, `Part`, `Parts` and
`Anchor` params: the things it is placed in, on or between. They are
given in the assembly body like any other param, as an expression of
their type:

| Type | Given as | Example |
|---|---|---|
| `Region` | a region, or an element of a region array | `opening: drawer_bay[same]` |
| `Part` | one part (an element of an array, or a part of another assembly) | `hinge_side: side_l` |
| `Parts` | one part or a list of parts (`x[*]` flattens into it) | `rails: [apron_front, apron_back]` |
| `Anchor` | a face or a mid-plane of a part or region | `on: top.bottom`, `about: mid(base, across)` |

Inside the component they are used where their type is expected: a
`Region` or `Part` param's faces in `place { }` (`against(hinge_side.right)`),
a `Parts` param as a joint's role (`into: rails`), an `Anchor` param as a
placement term's anchor (`against(on)`, which takes a face, so a mid-plane
there is `error[type-mismatch]` as anywhere). Place params are not
quantities, so they can't be `locked` or `solved`. The editor fills these
params by pointing (see [`docs/editing.md`](editing.md)).

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
| `roles { }` | `Part` or `Parts` each | no (default `piece: Parts`, `into: Parts`) | the parts the joint connects; see [Roles and pairs](#roles-and-pairs) |
| `param` | | no | settings; see `param` |
| `require` | | no | rules for valid use, checked for every pair |
| `insert` | list of assembly steps | no | how the parts come together |
| `cut_first` | a role, or a list of roles | no | the order in which the roles are cut |
| `adds` | `extend(...)` or a list of them | no | what the joint adds to each part's cut size |
| `uses` | a fastener kind value, or a list | no | fasteners the joint consumes without placing them |
| `member name [: fastener] { }` | | no | a piece it places where its parts touch, such as a dowel's pins; see [Joint members](#joint-members) |
| `cut [name:] selector { }` | | no | material removed; a name (`cut dado: ...`) is what messages call the cut; see [Named cuts](#named-cuts). `crossing(role, member)` is the wood a member takes out of a part |
| `makes part` | | no | a part the joint creates |
| `note` | `Text` | no | one piece of advice for the build manual |

Every joint kind also accepts two settings in its call that it doesn't
declare: `glue: Bool` (default `true`) and `methods:` (a list of recipes to
prefer for this joint's cuts). A kind can't declare a role, param or made part
named `glue` or `methods`.

### `kind name : slide | hinge | pull | fastener | tool { }`

A hardware kind is a product or a family of products. The base (`slide`,
`hinge`, `pull`) gives the words a message uses ("opened", "pulled out")
and the kind's type (`SlideKind`, `HingeKind`, `PullKind`); it decides
nothing else. What the hardware joins, where it sits, what it is made of,
how it moves and what it cuts are all written in the kind. A `slide`,
`hinge` or `pull` kind may contain:

| Entry | Required | Meaning |
|---|---|---|
| `roles { }` | yes | the parts it joins, as a joint's roles (see [Roles and pairs](#roles-and-pairs)). The first is a `Part`: the **piece** it is mounted on, which takes the positional argument. The others are `Part` or `Region` (a slide's opening) |
| `mount { }` | yes | where it sits on its piece unless the design says otherwise (see [Hardware placement](#hardware-placement)) |
| `param` | no | settings; a param with a `within` list is a family (`side_mount_slide(length: 450mm)`) |
| `require` | no | rules, checked for each piece of hardware where it is placed; `mount` names its mount face as an anchor (see [Adjustment](#adjustment)) |
| `member` | no | its pieces, each with its shapes (see [Shapes](#shapes)) |
| `adjust a on b: axis within d` | no | how far a member can be set on another, on one axis of the frame (see [Adjustment](#adjustment)) |
| `mechanism { }` | no | how its members move (see [Mechanisms](#mechanisms)) |
| `cut [name:] crossing(role, member) { }` | no | the wood it takes out: the bore for a cup, the mortise for a leaf, the hole for a screw (see [Faces to cut](#faces-to-cut)); a name is what messages call the cut (see [Named cuts](#named-cuts)) |
| `uses` | no | fasteners it consumes without placing them, as a joint's `uses:`: counted, never placed |
| `model` | no | a `Model`, for looks |
| `moving`, `license` | no | `Text` |
| any other `name: value` | no | catalogue data (`clearance: 12.7mm`, `cup_diameter: 35mm`); readable from outside as `slide.clearance` |

Every entry is readable on a kind value (`slide.length`,
`hinge_kind.cup_diameter`), params and fields alike. Inside the kind,
expressions may use its roles (`piece.thickness`) and its params, not its
fields. A kind can't have `makes` or the joint fields (`insert`,
`cut_first`, `adds`, `note`).

`fastener` and `tool` kinds keep a fixed set of entries, because the
hardware list and the work plan read them:

| Base | Required | Optional |
|---|---|---|
| `fastener` | `diameter`, `length`: `Length` | `envelope` (read by nothing; see [Hardware](#hardware)), `model`, `license`, `species` |
| `tool` | | `title`: `Text`; `powered`: `Bool` (default `false`) |

A fastener's `species` is a species word, as a stock's is: what it is
made of when it is wooden (`std:hardware`'s `dowel_pin` writes `species:
beech`). A viewer draws a placed fastener (a [joint
member](#joint-members)) in that wood, and one without `species` as
steel. Nothing is checked by it.

A fastener or tool kind has no `roles`, `mount`, `member`, `mechanism` or
`cut`.

The standard kinds in `std:hardware` are written this way; see
[Hardware](#hardware) for them. A cup hinge moves on the mechanism its kind
declares (see [Mechanisms](#mechanisms)): its members, their shapes and its
pins are catalogue data in the kind, written in the kind's own frame.
`euro_hinge` declares the common 110° full-overlay clip-on hinge (see
`docs/hardware/cup-hinge-linkage.md`). How far it opens is its
`opening_angle`, bounded by that param's range and by the mechanism's
`stop`; a mechanism whose links lock before the door opens that far is
`error[motion]`.

A piece of hardware moves only as its kind's `mechanism` says (see
[Mechanisms](#mechanisms)): `butt_hinge` turns on the pin in its knuckle,
just outside the edge where the two parts are flush,
`side_mount_slide` runs on one rail and `euro_hinge` moves on its
four-bar linkage, all solved the same way. A kind with no `mechanism`
doesn't move (a pull, or a hinge kind that leaves it out). A catalogue's
rules are the kind's own: how far a cup hinge's arm can be set on its
plate is an `adjust`, and the gap a slide needs beside the drawer is a
`require` (see [Adjustment](#adjustment)). No rule about a hinge or a slide
is built into the checker.

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
| `ground` | all three axes | the root that every other part is placed from, when the project has no root box (see rule 1 below). It may be turned with `orient`, like any part. The floor is wherever the lowest part ends up |
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

1. **Exactly one root.** A project has exactly one root, which every other
   part and region is placed from: either one part placed with `ground`, or
   one region made with `box(across:, up:, deep:)` (see [Regions](#regions)).
   Anything else is `error[ground]`, reported as follows:

   | Case | Message | Reported at |
   |---|---|---|
   | no part placed with `ground` and no region made with `box` | `nothing is the root: no part is placed with 'ground' and no region is made with 'box'` | the project item |
   | two parts placed with `ground` | `'<a>' and '<b>' are both placed with 'ground'` | the second `ground` |
   | two regions made with `box` | `'<a>' and '<b>' are both made with 'box'` | the second `box` |
   | a part placed with `ground` and a region made with `box` | `'<part>' is placed with 'ground', but '<region>' is made with 'box'; only one can be the root` | the `ground` |
   | a part placed with `ground` inside a component | `'<a>' is placed with 'ground' inside a component` | the `ground` |
   | a region made with `box` inside a component | `'<a>' is made with 'box' inside a component` | the `box` |
   | `box(across:, up:, deep:)` anywhere but as the whole value of a single (not array) region | `box(across:, up:, deep:) can only be the whole value of a region` | the call |

   The help says what to do instead: for two roots, place the second
   against the first (in `base`, `ground` becomes `flush(envelope.bottom)`,
   `flush(envelope.left)` and `flush(envelope.front)`); for none, add a
   root box with the overall sizes or place the lowest part with `ground`;
   inside a component, use a param the assembly is handed (a `Region` or a
   `Part`). Parts and regions inside components are never the root.

   When the root is a box, **the floor is the box's bottom face**, whether or
   not a part stands on it; parts and regions place against the box's faces
   and mid-planes like any region's. When the root is a part, the floor is
   wherever the lowest part ends up.

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

A piece of hardware is mounted on a **face of its piece** (its first role).
Where it sits is the kind's `mount { }` block, then the design's
`place { }` for the axes the kind leaves `open`:

```
kind euro_hinge : hinge {
  roles {
    piece: Part                      // the door's hinge stile
    on:    Part                      // the cabinet side
  }
  ...
  // On the back of the stile, the cup's centre cup_inset and half the
  // cup in from the edge nearer the side; the design says where along it.
  mount {
    face:   back
    across: from(edge(piece, nearer: on), inset: cup_inset + 35mm / 2)
    along:  open
  }
}
```

| Entry | Value | Meaning |
|---|---|---|
| `face` | a face word, or `face(piece, nearer: r)` | the face of the piece it is mounted on. A call's `face:` setting (every slide, hinge and pull call accepts one) overrides it |
| `across` | a mount term, or `open` | where the frame's origin is across the mount face |
| `along` | a mount term, or `open` | where the frame's origin is along the piece's length |

All three are required (`error[missing-field]`), each at most once. The
mount face fixes the third axis: the origin lies on the face. This places
the [kind's frame](#the-kinds-frame), and every member's shapes are drawn
in it.

**Mount terms**, usable in kinds only:

| Term | Meaning |
|---|---|
| `from(anchor, inset: d)` | the origin lies on the anchor, moved `d` (default `0mm`) into the piece: away from an edge or an end toward the middle of the mount face, into the part a face belongs to |
| `center(p)` | the origin lies midway across the mount face of `p` along this axis |

**Anchors**, usable in kinds only. An edge or an end belongs to the mount
face; a face is chosen across the axis the entry is for:

| Anchor | Meaning |
|---|---|
| `edge(p, nearer: q)` | of the two edges of the mount face that run along the piece's length, the one nearer the middle of `q` (the lower one when they are equally near) |
| `edge(p, flush_with: q)` | the edge of the mount face where `p`'s neighbouring face lies in a face of `q` (a butt hinge's pin edge, where the two parts are flush); if neither or both are, the edge nearer the middle of `q` |
| `end(p, toward: dir)` | the end of `p`'s length that faces the direction `dir` (`front`); `p`'s length must run along that direction's axis (`error[type-mismatch]` otherwise) |
| `face(p, toward: q)` | of `p`'s two faces across this entry's axis, the one nearer the middle of `q` |
| `crossing(p, q)` | the face of `p` that `q` touches or enters (as in [Faces to cut](#faces-to-cut)) |
| `face(p, nearer: r)` | for `face:` only: of the two faces across `p`'s thickness, the one nearer the same-named face of region `r` (the lower one when they are equally near) |

The frame's directions on the mount face: `along` runs along the piece's
length (its width when the mount face is an end of the piece), from the end
when `along` is `from(end(...))`, otherwise the way the piece's length
runs; `across` runs from the edge when `across` is `from(edge(...))`, toward
the middle of the face, otherwise toward `right`, `up` or `back`; `depth`
runs into the piece.

**`open`** leaves an axis to the design: the hardware's `place { }` sets it,
with the same terms as for parts (`center`, `flush`, `against`, `spaced` for
hardware arrays), applied to the box around the shapes of the members
carried by the piece (`with:` its first role). A `place` term for an axis
that is not open is `error[axis-set-twice]` ("the hardware's kind already
fixes it"); an open axis with no term is `error[axis-not-set]`. `orient`,
`ground` and `corners` don't apply to hardware. Fasteners and tools are
never placed.

**A member's own `mount { }`** places that member in the design, on the
axes it names (`across`, `along`, `depth`), from an anchor instead of from
the kind's origin. Its shapes are still written in the kind's frame, at
the member's **nominal place**: where the catalogue puts it, with the
rest of the hardware where the kind draws it. Each entry is
`from(anchor, inset: d)`: on that axis, the member's `0mm` lies on the
anchor's plane, moved `d` into the anchor's part (a negative `d` moves it
out of the part). How far the mount moves the member from its nominal
place is its **offset** on that axis: where that point lies in the frame.
An offset must be exactly `0mm`, unless an `adjust` of the kind lets the
member be set there (see [Adjustment](#adjustment)); otherwise it is
`error[adjust]`. The member is drawn and checked where its mount puts it:

```
  /// The mounting plate: screwed to the inside of the cabinet side.
  member plate {
    with:  on
    // 11mm thick plus plate_height, standing off the side's inside face,
    // which nominally lies 5mm + plate_height outside the cup's centre;
    // 36mm wide, its middle plate_setback back from the side's front
    // face, which lies against the back of the door.
    shape: box(
      along:  50mm,
      across: -(5mm + plate_height)..6mm,
      depth:  -(plate_setback + 18mm)..-(plate_setback - 18mm),
    )
    mount {
      across: from(face(on, toward: piece), inset: -(5mm + plate_height))
      depth:  from(face(on, toward: piece))
    }
  }
```

**Every member's shapes are checked**, like any part (see [One collision
model](#one-collision-model)): a cup hinge's arm, knuckle and links must
stay out of the wood of the door and of `on` at rest and at every angle as
the door opens, and the door's wood has room for them only where the kind's
`cut` bored it. The hinge stops at its widest angle on a limiter inside the
arm, steel on steel; nothing is excused as a stop.

**Contact.** Every part a piece of hardware joins must meet it: of the
members a role's part carries (`with:` that role), at least one touches or
enters that part at rest. Otherwise the hardware's piece meets none of the
parts it should join (`error[no-contact]`, `hinge: lid meets none of the
parts it should join`), as for a joint. A butt hinge's `fixed_leaf` lies
just off its mount face, in `on`: if the mount face of the piece does not
lie against `on`, the leaf meets nothing. Likewise a `cut` whose member
neither touches nor enters the part it names is `error[no-contact]`.

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
  Every role of a piece of hardware is re-bound this way, whatever its
  name. The cuts such a joint makes in an outside part (the mortise a
  rail's tenon goes into, in a leg the assembly was handed) are made in
  the counterpart, reflected into its frame. Its fasteners are its
  source's again: each member keeps what it is and how many copies it
  is, and the fastener list counts them once more for the mirror image
  (two cleats of three screws are six to buy). A counterpart turned so that
  its length, width or thickness lies along another axis than the
  reflected part's, or a cut drawn at an angle or as an outline whose
  face the reflection turns over, can't be reflected yet:
  `error[mirror]`, naming the joint and both parts. A `Parts` param is
  re-bound part by part, in the same order; an `Anchor` param is re-bound
  to the face of the counterpart that lies in the reflected plane
  (`side_l.right` becomes `side_r.left`), or to the reflected mid-plane; a
  `Region` param to the region that occupies the reflected box.
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

### Boards

A part of `form: solid` stock whose cut size **across the grain** (its
width, or its length when `grain: width`) is more than its stock's
`max_width` is **made of boards**: `ceil(size / max_width)` boards of equal
width, joined edge to edge, the seams running along the grain. Nothing is
written in the file. Boards are derived, as the cut list is: they are
numbered from the reference edge (from the start of the length when
`grain: width`) and appear as `top.board[1]`, `top.board[2]`, ... in the cut
list and on work cards. Placement, joints, hardware and `fit` see one part;
the collision checks see the part's wood as its boards make it, cuts and
all.

Equal widths are a simplification. A deliberate layout (book-matched
boards, a wide board in the middle) is written as separate parts, joined
with the same edge-joint kinds as ordinary joints.

How the boards are joined is the `boards:` field, on a part or once on the
project for every part that doesn't say:

```
part top : oak_19 {
  boards: joined(by: tongue_and_groove)
  ...
}
project "Side table" {
  boards: joined(by: [tongue_and_groove(glue: false), splined_edge])
}
```

Left out everywhere, it is what the first library in the prelude with a
`boards:` says: `std:joinery`'s `library` block has `boards: joined(by:
glued_edge)`, so it is `joined(by: glued_edge)`. A design whose part is
made of boards when nothing says how they are joined is
`error[unknown-name]`. `by:` takes a joint
kind value, or a list of them, which are all used. Each is an ordinary
`kind ... : joint` (`std:joinery` has `glued_edge`, `tongue_and_groove`,
`splined_edge`, `chigiri` (butterfly keys across each seam) and
`suitsuki_zan` (battens in sliding dovetail housings across all the boards);
the last three need no glue), applied as for a joint between parts:

- A kind with **two roles** joins each **seam**: its first role is the board
  nearer the reference edge, its second the next board. The two butt edge to
  edge, so `crossing(piece, into)` is `piece`'s edge at the seam.
- A kind with **one `Parts` role** is one connection across all of the
  part's boards: the boards together are the whole part (see "Roles and
  pairs"), so the battens of `suitsuki_zan` are as long as the part is wide.
- Requires are checked for every seam; a failing one is an error on the
  `boards:` line (on the part, when the default is used), naming the boards.
  Neighbouring boards are always `edge_to_edge`, so an edge joint's rule
  that asks it holds at every seam.
- `adds:` widens a board toward its neighbour (the tongue of a tongue and
  groove), and the cut list shows each board's width with it.
- Cuts are made on the boards. Parts the kind makes (a spline) appear in the
  cut list as `<part>.<name>[n]`, one per seam (`n` counts seams from 1), or
  `<part>.<name>` for a kind with one `Parts` role.
- `glue: false` in the kind value says the seam is not glued.
- Fasteners a board joint `uses:` aren't listed yet, and a board joint's
  [members](#joint-members) aren't placed between boards yet
  (`warning[not-supported]`). Every cut a solid
  part can have is made on boards: the box-shaped ones (`notch`, `slot`,
  `pocket`, `tenon`) and the angled ones (`outline`, `dovetail_socket`,
  `dovetail_tail`, a `slot` at an angle). `drill`, `spigot` and `round` are
  not made on boards yet: they are `warning[not-supported]`.

A part that is not made of boards ignores `boards:`. A part a joint makes,
a turned part and a dowel are never made of boards. Two limits stay errors,
because the fix is a design choice:

- **Length is not split by itself.** A part longer than its stock's
  `max_length` is `error[type-mismatch]` unless its `lengths:` says where
  to splice it: an end joint carries load, so where it goes is a design
  choice (see [Splices](#splices)).
- **Sheets are not split.** A `plywood` or `mdf` part wider than its stock's
  `max_width` is `error[type-mismatch]`: a sheet isn't glued up from boards.
  Choose a larger sheet, or make the part from several parts.

### Splices

A part longer than the boards you can buy is made of **pieces** joined end
to end. The part says where and with what, in its `lengths:` field, the
way a wide part names its edge joint in `boards:`:

```
part rail : oak_19 {
  length:  3000mm
  width:   90mm
  lengths: spliced(by: lap_splice, at: 1500mm, from: left)
  ...
}
```

`spliced(by:, at:, from:)` takes:

- `by:` the joint kind every splice is made with, as a kind value with its
  own settings (`lap_splice(lap: 300mm)`, `kanawa_tsugi`). It is an
  ordinary `kind ... : joint` with two roles that are parts (the first is
  the piece nearer the `from` end, the second the next piece), applied as
  for a joint between parts: its rules, `adds:`, cuts, made parts,
  `insert:` steps, `cut_first:` and `note:` all work. `std:joinery` has
  `lap_splice` (a half-lap end joint, glued) and `kanawa_tsugi` (a stepped
  scarf with a draw pin).
- `at:` where each splice is: one `Length` or a list of them
  (`at: [2400mm, 4800mm]`), each measured along the part's drawn length
  from the end `from:` names. Each must be more than `0mm` and less than
  the drawn length (`error[out-of-range]`), and no two the same
  (`error[duplicate-name]`).
- `from:` the end of the part the distances are measured from, as a face
  word of the part once placed: a rail along `across` has the ends `left`
  and `right`, a standing side `bottom` and `top`. A face that isn't an end
  of the part is `error[type-mismatch]`, naming its two ends.

**Why a distance from an end is not a coordinate.** The language places
nothing by coordinates (principle 3). A splice's place is a relationship
between the splice and an end of its own part, of the same sort as
`flush(base.left, inset: 20mm)` or a mark on a cut: it measures within the
part, from a face the part names, and stays right wherever the part is
placed and whatever its size. A mirror image is spliced at the same
distance from its mirrored end. A distance that depends on other parts is
written with the usual measures and sizes (`at: between(leg_l, leg_m) +
leg_l.width + leg_m.width / 2` for a rail flush with `leg_l`'s outer face
puts the splice over the middle of `leg_m`), so it follows them.

What Stave does with it:

- The part is placed, joined and fitted with hardware as one part. Then
  it is cut into pieces at the splices, numbered from 1 at the `from` end:
  `rail.piece[1]`, `rail.piece[2]`, ... Each piece is a part of its own in
  the solved design (the cut list, the checks, the 3D view, focus mode and
  the pulled-apart view), with the part's stock, grain and note; the
  first keeps the part's place in the source. Each splice is a joint of
  its own, `rail.splice[1]`, ... (splice `n` joins piece `n` to piece
  `n + 1`); its made parts are `rail.splice[1].pin`.
- The splice's `adds:` lengthens its pieces past the splice (a lap's
  overlap), as a joint lengthens any part. **Each piece must fit its
  stock's `max_length`** once its splices and the part's own joints have
  added to it: `rail.piece[1]: its cut length works out as 2645mm, which
  is more than its stock's max_length of 2400mm` (`error[type-mismatch]`
  at `lengths:`), with the help to move the splices, add one, or buy longer
  stock.
- **A splice needs its wood whole.** The cuts other joints and hardware
  make in the part are each made on the piece they fall on. A cut that
  reaches into a splice, where both pieces' wood belongs to the splice, is
  `error[wall]` at `lengths:` (a backstop, see [Cuts stay in their
  part](#cuts-stay-in-their-part)): `shelf_dados: its dado in side_l
  crosses the splice between side_l.piece[1] and side_l.piece[2]`, with a
  note saying where both run, measured from the `from` end, and the help
  `a splice needs its wood whole: move the splice with at: in side_l's
  lengths: so it clears the dado, or move shelf_dados`. A cut beside the
  splice is the splice kind's judgement, written with `wall()` (std's
  `lap_splice` keeps half the board's thickness between its laps and any
  other cut); a hinge, a pull or a dowel near a splice is measured the
  same way, by the same rules as anywhere else.
- A joint pair or piece of hardware that named the part is joined to the
  piece it cuts (or, if it cuts none, the piece it touches most); an
  assembly or scene that lists the part lists every piece.
- The splices' rules are checked for every splice; a failing one is
  reported at `lengths:`, naming the pieces.

A part without `lengths:` that is longer than its stock is
`error[type-mismatch]`, with the help suggesting splices that would give
pieces within `max_length`, measured from the start of its length:
`stock 'pine_19' comes no longer than 2400mm: splice it end to end with
lengths: spliced(by: <an end joint>, at: [2250mm, 4500mm, 6750mm], from:
bottom), choosing the end joint and leaving each piece room for what its
splices add, or buy longer stock`. A part shorter than its stock may
still be spliced (to use up short boards). Only `solid` board parts are
spliced: a turned part, a sheet (`plywood`, `mdf`) or round stock with
`lengths:` is `error[type-mismatch]`. A part a joint makes is never
spliced. Not yet supported, each `warning[not-supported]` at `lengths:`:
a splice kind's members and `uses:` fasteners (not placed or listed), and
a spliced part wider than its stock's `max_width` (its pieces are drawn
as single boards).

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
region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)
region drawer_bay[a, b, c] = split(bay, up, shares: [140, 180, 220])
```

| Function | Result |
|---|---|
| `box(across:, up:, deep:)` | a box of space made from three lengths, with no parts needed: the project's root (see [Placement rules](#placement-rules-checked), rule 1). All three are required |
| `opening(left:, right:, bottom:, top:, front:, back:)` | the box bounded by six anchors, one per side. All six are required |
| `split(region, axis, at: part)` | two regions, on either side of the part along `axis`, each ending at the part's face |
| `split(region, axis)` | as many equal regions as the declaration has elements (so the region being declared must be an array), with nothing between them |
| `split(region, axis, shares: [n, ...])` | as `split(region, axis)`, but the pieces are sized in proportion to the shares, in label order |
| `cover(region, mount, thickness: d)` | the space a door or drawer front fills: the region's front face, grown or shrunk at each edge as the mount says, `d` deep |

`split` labels its pieces in the direction of its axis: for `up` the first
label is the lowest piece, for `across` the leftmost, for `deep` the front one.

**`box`** makes the root region from sizes alone: the overall width, height
and depth of the piece, before any part exists. Each size is a `Length`
greater than `0mm`, usually a param (`overall_width`); a size of `0mm` or
less is `error[out-of-range]` (`'up' must be more than 0mm, found 0mm`).
The box is where everything else is measured from, so it is placed by
nothing and is never in a component; rule 1 of
[Placement rules](#placement-rules-checked) gives every error. Parts and
components place against its faces (`flush(envelope.bottom)` stands a
part on the floor inside it) and its mid-planes, and regions are made from it
(`split(envelope, up, ...)`, `opening(...)` with its faces) like any
region. Which `box` a call is follows from its argument names:
`across:`, `up:` and `deep:` make a region; the hardware envelope and a
member's shape are listed under [Built-in functions](#built-in-functions).

**`shares:`** gives relative sizes: `shares: [140, 180, 220]` splits the
region's size along the axis in the ratio 140 : 180 : 220, so here the
middle piece is 180 / 540 of it. A share is a unitless value (a `Count` or
a `Ratio`, usually a plain number) greater than zero. The rules, each with
its error:

- there are as many shares as the declared array has elements:
  `error[type-mismatch]` (`'shares' needs 3 numbers, one for each piece of drawer_bay, found 2`);
- every share is more than zero: `error[out-of-range]`
  (`'shares' must each be more than 0, found 0`);
- a share has no unit: `error[unit-mismatch]`
  (`'shares' needs a number, found a length`, with the help "shares are
  relative sizes: write 140, not 140mm");
- `shares:` and `at:` are not written together: `error[type-mismatch]`
  (`split takes 'at:' or 'shares:', not both`);
- the array's count is fixed: a counted array whose count is a param with a
  `within` list can't take `shares:` (`error[type-mismatch]`,
  `'shares' needs a fixed number of pieces, found a count that can change`).

`split(region, axis)` without `shares:` is equal division, as before. The
editor's grip between two neighbouring pieces edits their two shares (see
[`docs/editing.md`](editing.md)).

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

**A kind chosen through a param must take what the declaration hands
it.** A `JointKind` (or other kind type) param says only what class of
kind it takes, not which roles and settings; the declaration that uses it
decides that (`joint housing : joinery(board, into: sides, depth: depth)`
hands its kind a part as `piece`, parts as `into` and a `depth`). When
the kind that reaches such a declaration through a param (a component's
or the project's) has no setting the declaration gives it, no role it
names, or takes one part where it is handed several (or something else
the role doesn't take), or the declaration hands nothing to one of its
roles, the kind can never be used there, whatever the sizes. That is one
`error[type-mismatch]` at the place the kind was chosen: the assembly's
argument, or the param's default when it was not set (followed through a
component that only passes the param on to another). The message names
the kind, the param and its component; the first note says why, naming
who hands what; the help names the kinds of the same class in scope
there that take what is handed, by the same test; the declaration that
uses the kind is a secondary place:

```
error[type-mismatch]: 'glued_edge' can't be the joinery of Shelf
  --> shelf_test.stave:27:16
   |
27 |   joinery:     glued_edge
   |                ^^^^^^^^^^
  ::: std/casework/shelf.stave:26:47
   |
26 |   joint housing : joinery(board, into: sides, depth: depth)
   |                                               ----- Shelf uses joinery here
   = note: Shelf gives its joint a depth, and glued_edge has no depth
   = help: choose a kind that takes what Shelf hands its joint: cope_and_stick, dado, dowel, groove, rabbet, socket, splined_edge, tongue_and_groove
```

The reason has one of four shapes: `<Shelf> gives its joint a <depth>,
and <kind> has no <depth>`; `<Case> hands its joint a part as <into>,
and <kind> has no <into>: it takes its parts only as <boards>`; `<Case>
hands its joint two parts as <piece> (<case.side[left] and
case.side[right]>), and <kind> takes one part there`; `<Housed> hands its
joint nothing as <brace>, and <kind> needs one part there`. For a param of
the project itself the message is `'<kind>' can't be the project's
<joinery>` and the reason names the declaration (`the project gives
housing a depth, ...`); for hardware it says `its hardware`. When no kind
in scope takes what is handed, the help says so: `no joint kind in scope
takes what <Shelf> hands its joint; use another component, or write a kind
that does`. Nothing else is checked, cut or made for that declaration, and
a second declaration whose kind fails the same way at the same choice (the
Case's `base_joints` and `top_joints`, the elements of an array) is not a
second error: it is a note on the first, `the same for top_joints`, and
its line another secondary place. A kind written in the declaration itself
(`joint j : glued_edge(a, into: b, depth: 9mm)`) keeps the messages of
[Joints and hardware](#joints-and-hardware): there the field was written
for that kind, and `the kind 'glued_edge' has no field 'depth'` is what to
say.

### Roles and pairs

- A role is `Part` (exactly one part) or `Parts` (one part or a list). A
  hardware kind's roles are `Part` or `Region` (a slide's opening), its
  first a `Part` (the piece it is mounted on); a piece of hardware is one
  connection between exactly those, and every part it carries members on
  must meet one of them (see [Hardware placement](#hardware-placement)).
- A kind with a `Parts` role has one role or two. With two, the joint
  applies to **pairs**: one part from the first role and one from the second
  that **meet**. With one, it is **one connection across all of the parts**
  listed for it (a batten that crosses every board of a panel). A kind with
  three or more roles has only `Part` roles; it is one connection between
  exactly those parts.
- Two parts **meet** when their drawn boxes touch over an area (not just along
  an edge) or overlap. They *butt* when they only touch, and *cross* when they
  overlap.
- **Every listed part must meet at least one counterpart**, otherwise it is
  `error[no-contact]`, naming the part. In a joint with one `Parts` role or
  with three or more roles, every part must meet at least one of the others.
- Everything in the kind is evaluated **per pair**: `into.thickness` is the
  thickness of this pair's `into` part, param defaults and `require`s are
  worked out for each pair, and cuts are made for each pair. A setting given
  in the call is the same for all pairs.
- In a kind with **one `Parts` role**, the role's parts together count as
  one where one is needed: `boards.width` (and `.length`, `.thickness`) is
  the size of the box around all of them, along the first part's axes, and
  in a made part's `orient` and `place` the role names that box (a batten
  as long as the whole panel is wide). A cut on the role is made on each of
  its parts that the other part of the selector meets.
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

  cut socket: end(into, facing: piece) {
    dovetail_socket(neck: neck, depth: depth, angle: angle)
  }
  cut tail: end(piece, facing: into) {
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
takes (default 1): `uses: wood_screw(length: 30mm, count: 4)`. These
fasteners appear in the hardware list. They are counted, never placed, take
no part in collision checks, and are the only thing not modelled, for joints
and hardware alike (see [One collision model](#one-collision-model)). `uses:`
is for what is genuinely not placed (the screws that come with a hinge, a
handful of glue blocks); a fastener that goes somewhere particular (a dowel's
pins, the screws of a screwed joint) is a [joint member](#joint-members),
which the hardware list counts too.

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
| `against(selector)`, `against(selector, gap: d)`, `flush(selector)`, `flush(selector, inset: d)` | as for parts, against the face the selector names. A negative `gap` sinks the made part into the face by that much (a batten in its housing) |

Each axis must be fixed exactly once, as everywhere else. The place a
selector names is a plane and a footprint. For `crossing`, the plane is
where the two parts' drawn boxes touch when they butt, and the middle of
their overlap along the face's axis when they cross. For `end` and `side`,
the plane is that face of the first part's drawn box. Along the face,
`center` centres on the overlap of the two parts' cut boxes.

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

There is no way to switch this check off. Every cut operation is made and
checked, angled ones included (see "Angled cuts" below). The only cuts the
geometry can't make yet are cuts other than `drill` and `spigot` on round
stock or a turned part: those are `warning[not-supported]`, and the part is
drawn and checked without them.

**Joints with nothing cut.** A kind need not cut anything: with no
`cut`, `insert:` or `adds:`, it only says the parts are held together.
std's `glued_face` is one: two parts glued face to face, as a cleat glued
inside an apron, a lining, or a leg doubled up from two boards
(`joint cleat_glue : glued_face(cleat, into: apron)`). Its parts must
meet like any joint's, and the joint holds them (see [Held](#held)); its
`note:` goes on the work plan. Two boards glued edge to edge to make a
wide part are not a joint between parts but the part's
[boards](#boards), joined by `glued_edge`.

#### Joint members

A joint kind may place pieces where its parts touch: a dowel's pins, the
screws of a screwed joint. Each is a `member` with a body, laid out on a
contact patch instead of carried by a part, and drawn, cut for and checked
like a hardware member (see [One collision model](#one-collision-model)):

```
  /// The pins: wooden dowels glued half into each part, in matching holes.
  member pin : dowel_pin(diameter: diameter, length: 2 * depth) {
    on:     contact(piece, into)
    count:  count
    spaced: evenly
    shape:  cylinder(diameter: diameter, depth: -depth..depth)
  }
  cut end_hole: crossing(piece, pin) { drill(diameter: diameter, depth: depth) }
  cut hole: crossing(into, pin) { drill(diameter: diameter, depth: depth) }
```

| Entry | Type | Required | Meaning |
|---|---|---|---|
| `: fastener` | a fastener kind value | no | what it is: the hardware list counts it once for every copy placed |
| `on:` | `Patch` | yes | where it goes: `contact(a, b)`, the patch where two of the joint's parts touch |
| `count:` | `Count` | no (default `1`) | how many copies, for each pair |
| `spaced:` | `Spacing` | no (default `evenly`) | how the copies are spread along the patch's length |
| `shape [name]:` | a shape | yes | what it is made of (see [Shapes](#shapes)) |

A joint's member always has a body; one without is `error[missing-field]`
(`member 'pin' is missing 'shape'`). It has no `with:` or `mount { }`
(`error[unknown-field]`), and a missing `on:` is `error[missing-field]`.
Only a joint's member names what it is after `:`; a hardware member that
does is `error[unknown-field]` (`member 'screw' of a hardware kind can't
name a fastener`); a hardware kind lists its fasteners with `uses:`.

**The member's frame.** Each copy has its own frame, with the words a
hardware kind's frame uses: `along` runs along the patch's length (its
longer side), `across` along its width, and `depth` square to the patch,
from `a` into `b` of `contact(a, b)`: negative depths lie in `a`, positive
ones in `b`. The origin is on the **contact plane** (where the two parts
touch; for parts that cross, the middle of their overlap), in the middle of
the patch's width. So `cylinder(diameter: 8mm, depth: -15mm..15mm)` is a pin
reaching 15mm into each part from the joint line. `along` and `across` run
toward the higher world coordinate, which matters only for a shape that is
not symmetric along them.

**Spacing.** `spaced:` takes the [spacing values](#placement-terms) parts
use, measured along the patch's length from its lower end, with each copy
as long as its shapes are along `along`: `evenly` (equal gaps before,
between and after), `ends` and `ends(inset: d)`, `pitch(d)` and `gap(d)`.
With `pitch` or `gap`, copies that run past the far end of the patch are
`error[overlap]` at the joint's call: `pegs: its pegs run 10mm past the end
of where its parts touch`, with the help to use fewer or space them closer.
A copy is centred across the patch's width; a shape's `at:` moves it.
Positions are exact and rounded once, like every solved length.

**Cuts.** `cut name: crossing(role, member) { ... }` is made once for every shape
of every copy, on the face of the role's part the shape touches or enters,
measured on the shape (see [Faces to cut](#faces-to-cut)): a `drill` is
centred on the shape's axis. Because both parts are cut from the same
placed shape, the holes a pin makes in two parts share their centres by
construction, even where the patch is narrower than one part's end. A
member that meets nothing of the role's part is `error[no-contact]` at the
joint's call (`pegs: its peg doesn't reach base`).

**Checks.** A joint's members are bodies in the overlap check at rest, as
every hardware member is. A pin in its own hole touches it; a pin that
reaches past the bottom of its hole, into a part no hole is drilled in for
it, or into another joint's pin or tenon is `error[overlap]` at the joint's
call, naming the member as `<joint>'s <member>` (with the pair's parts when
the joint has several pairs):

```
error[overlap]: side_dowels's pin and foot overlap
  --> project.stave:47:21
   = note: they share 8 × 8 × 8mm (across × up × deep) at the top end of foot
   = help: the pin of 'side_dowels' goes 8mm into foot, where no hole is drilled for it; move foot or the joint so the pin misses it, or make the pin shorter
```

The other helps say `the pin of '<joint>' goes 5mm past the bottom of its
hole in <part>; make the hole as deep as the pin reaches, or the pin 5mm
shorter`, and `the pin of '<joint>' and the pin of '<other>' run 8mm into
each other inside <part>; move one of the joints, or use fewer or shorter
ones`. The members of one pair are not checked against each other, as a
kind's own members never are: a kind keeps its copies apart with a rule on
the patch (`require seat_long : contact(piece, into).length >= count *
(diameter + wall) + wall`).

**Screws.** A member is checked by its shapes, so a shape is the room the
piece takes once it is in. A screw's thread cuts its own way into the wood
around its pilot hole and takes no room of its own: a screw is written at
the size of its core, the pilot hole, and the pilot hole holds it exactly
while the clearance hole in the piece is wider. `std:joinery`'s `screw`
writes `shape: cylinder(diameter: pilot, depth: -piece.thickness..length -
piece.thickness)`.

A joint member moves with the pair's first part. Members aren't placed
between the boards of a part made of boards yet: such a kind is
`warning[not-supported]` there (see [Boards](#boards)).

**Drawing.** A viewer draws every copy of a joint's member where it is
placed, in the normal view, pulled apart and in a section, as the
fastener it is: in the wood its fastener kind's `species:` names (a
`dowel_pin` is beech), and as steel when the kind names no species or
the member names no fastener. It is a thing of its own in the view
(clicking a pin selects the joint's pins on that pair, named `<joint>'s
<member>`, and says which joint placed them and where that joint is
written), and selecting it draws the pair's parts see-through, so pins
inside the wood show. Pulled apart, and in focus mode as the pair's
`insert:` steps play, it moves with the part that carries it (the
pair's first part), so a dowel's pins ride on the piece and go into
their holes in the other part as it comes home. The parts tree lists
each joint's member once, grouped over its pairs, with how many copies
the joint places in all (`dowel_pin × 8`), as the hardware list
counts them.

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
| `end(p, facing: q)` | the end of `p` nearest `q`; without `facing:`, the end at the end of `p`'s length (`opposite: true` takes the start) | `width` and `thickness` across the end; depth runs along `p`'s length |
| `side(p, normal: width \| thickness, from: q)` | the reference face (`normal: thickness`) or reference edge (`normal: width`) of `p`; `opposite: true` takes the face opposite to it | `along` from the end nearest `q` (without `from:`, from the start of `p`'s length); `across` from the reference edge (or reference face); `flip: true` measures `across` from the opposite edge |
| `crossing(p, q)` | the face of `p` that `q` touches or enters. The **footprint** is the outline of `q`'s cut-size box on that face | within the footprint, in `q`'s own axes |

In a hardware or joint kind, the second argument of `crossing` may be a
member: `crossing(piece, cup)` is the face of `piece` the member's shapes
touch or enter (when a shape passes right through, the one nearer the mount
face, or for a joint member the contact plane), and there is one
**footprint per shape**: the region the shape's box and the part's cut box
share, in the part's own frame. The operations are measured on the shape's
whole box, not on the footprint: a `drill` is centred on the shape, so it
stays where the member is even where the shape runs past the part's edge. A hardware cut is made once for
each piece of hardware, a joint member's once for each copy (see [Joint
members](#joint-members)). Its operations are the same as a joint's:

```
cut cup_bore: crossing(piece, cup) { drill(diameter: 35mm, depth: 12.8mm) }
cut fixed_leaf_mortise: crossing(on, fixed_leaf) { notch(depth: 1.5mm) }
```

When `p` and `q` cross right through each other, `crossing(p, q)` is `p`'s
reference face if it is parallel to both lengths, otherwise `p`'s reference
edge. `normal: width | thickness` names the face instead (the face across
`p`'s width or thickness), for a cut that goes in from another face: a hole
bored down through a stick that another stick passes through vertically is
`crossing(first, lock, normal: thickness)`. `opposite: true` takes the face
opposite to the chosen one.

#### Named cuts

A cut in a joint or hardware kind is named by writing a name and `:` after
`cut`: `cut groove: crossing(into, piece) { ... }`. The name is what
messages call the cut, whatever its operations: `its groove in side_l`,
`the tenon on rail`, `its socket in beam_a`. In a message the name's
underscores are read as spaces (`mating_groove` is "mating groove"). A
cut name is not an item: it shares no namespace with the kind's params,
roles or made parts. Each cut of a kind has its own name, because the name
is how messages and patches tell the kind's cuts apart: two cuts of one
kind with the same name are `error[duplicate-name]`, naming both places,
with the help `give each cut of a kind its own name, as in groove and
mating_groove`. Two different kinds may use the same cut names.

Names come only from the kind; Stave never guesses one. A message about an
unnamed cut falls back to its operation's word: `mortise` for `pocket`,
`hole` for `drill`, `notch`, `slot`, and `cut` for every other operation.
So every `cut` in a kind should be named, and an unnamed one is
`warning[unnamed-cut]` (layer 2), at the cut's selector:

```
warning[unnamed-cut]: this cut has no name, so messages will call it "a slot"
  --> project.stave:52:7
   |
52 |   cut crossing(into, piece) {
   |       ^^^^^^^^^^^^^^^^^^^^^
   = help: name it after what it is, as in cut trench: crossing(...) { ... }
```

The word in quotes is the fallback above; when a cut's operations have
different words (a `drill` and a `slot` in one cut) it is `cut`. The help
repeats the cut's own selector word (`end`, `side` or `crossing`).

- It is reported once for each unnamed `cut` in a kind, at the kind's
  definition, however many times the kind is used (or in however many
  modules that import it).
- A kind nobody uses is still warned: every kind is resolved where it is
  declared, used or not, so `stave check std` checks every kind in the
  library.
- The `cut { }` block of a recipe lists cutting steps, not a kind's cuts,
  and never warns.

#### The kind's frame

A hardware kind's members, shapes and pins are drawn in the kind's own
frame, with the hardware closed, as a catalogue drawing is. Its three
directions are the words a cut already uses on a face:

| Word | Runs |
|---|---|
| `along` | along the length of the piece the hardware is mounted on |
| `across` | across the mount face, away from the piece's edge the hardware is measured from (a hinge: the edge nearer `on`) toward the middle of the piece |
| `depth` | from the mount face into the piece; negative stands off it |

The frame's origin is the point where the kind's `mount { }` and the
design's `place { }` put the hardware on its mount face (see
[Hardware placement](#hardware-placement)): a cup hinge's is the centre of
its cup, on the face the cup is bored into. These are **measurements within the hardware**, like a profile's
stations and a cut's marks: catalogue data, never a position in a design.
They are written with `point(along:, across:, depth:)` (each a `Length`,
default `0mm`) and with the ranges a shape takes, and only inside a
hardware kind's `member` and `mechanism`: `point(...)` anywhere else is
`error[syntax]`, as is a range `lo..hi` anywhere but a shape's argument.

#### Cut operations

`through` as a depth means all the way through the part.

| Operation | On | Removes |
|---|---|---|
| `notch(depth:)` | crossing | the whole footprint, to `depth` (dados, rabbets, grooves, halvings) |
| `slot(width:, depth:, angle:, from_centre:)` | crossing | a slot `width` wide through the footprint's centre and out past its edges, to the faces of the part. `angle` (default `0deg`) turns it in the face, measured from the entering part's length axis toward its width axis (or from its width axis toward its thickness axis when its length runs into the face). With `from_centre: true` it starts at the footprint's centre and runs out one side only, in the direction of `angle`. An angle that turns it into the face it is cut from is `error[type-mismatch]` |
| `pocket(width:, thickness:, depth:)` | crossing, end, side | a rectangular mortise, centred; `width` and `thickness` are in the entering part's axes |
| `tenon(length:, width:, thickness:)` | end, crossing | everything around a centred rectangular tenon, `length` long from the face. On `end`, `width` and `thickness` are across the part's width and thickness. On `crossing` it is a **tongue** along an edge, for parts that butt edge to edge (the face is across the part's width): `width` runs along the part's length, `thickness` across its thickness, centred on the footprint. On any other face of a crossing it is `error[type-mismatch]` |
| `spigot(length:, diameter:)` | end | everything around a centred round tenon |
| `round(diameter:, flats:, flats_angle:)` | crossing | turns the part round, to `diameter`, over the length where the other part passes (the footprint's extent along this part's length), leaving square sections either side. With `flats:` it keeps two flat faces that distance apart, their normal at `flats_angle` (default `0deg`) from the part's width axis toward its thickness axis |
| `drill(diameter:, depth:)` | crossing, end, side | one round hole, centred on the footprint or end; on `crossing(role, member)`, centred on the member's shape. A row of holes is a [joint member](#joint-members) laid out `count:` times, with the drill cut once for every copy. `count:` was retired: `drill(..., count: 4)` is `error[unknown-field]` (`'drill' has no field 'count'`), with the help `a drill makes one hole; for a row of holes, give the joint kind a member for what goes in them, with count: and spaced:, and drill crossing(role, member): one hole for every copy, lined up in both parts` |
| `dovetail_socket(neck:, depth:, angle:, slope:)` | end, crossing | a dovetail socket: `neck` wide at the face, `depth` deep, widening by its spread each side to its bottom, cut straight through the part (see "Angled cuts"). On `crossing` it is a **housing** for a sliding dovetail, running along the entering part's length |
| `dovetail_tail(neck:, depth:, angle:, slope:)` | end, crossing | everything around a dovetail tail: over the last `depth` of the part, the wood either side of a tail `neck` wide at its shoulder and wider by its spread each side at the face, cut straight through the part. On `crossing` it is the **tongue** of a sliding dovetail, running along the part's own length |
| `outline(marks: [...], depth:, from_depth:)` | side | the shape marked out with `mark(along:, across:)` points, in order round the shape, cut from `from_depth` (default `0mm`, the face itself) to `depth` below the face. With `from_depth` greater than zero the cut is hidden inside the part: a recess for a hidden stub tenon |

**Angled cuts.** `dovetail_socket`, `dovetail_tail`, `outline` and a `slot`
turned by an angle that isn't a multiple of 90deg are each a flat shape marked
on a face and cut straight down from it: a polygon in the plane of the face,
between two depths. They are cut, drawn and checked like every other cut.

- **`outline`**: the polygon is the marks, joined in the order written, the
  last back to the first (either way round). `along` is measured from the end
  the selector measures from, `across` from the reference edge (or face). The
  shape may have corners that point inward (an L, a stepped scarf), but its
  marks must not cross themselves: at least three marks, no mark twice, not
  all on one line, and no two sides that cross or touch except neighbours at
  the mark they share. Otherwise it is `error[type-mismatch]`, naming a side
  that crosses: `<joint>: the outline crosses itself between mark 3 and
  mark 4`.
- **`dovetail_socket`**: on the end of the part, centred across it. Its sides
  lie across the entering part's width (along the axis of this part that the
  entering part's width lies along; this part's own width when that one runs
  into the end), and it is cut straight through the part's third axis. It is
  `neck` wide at the end and `neck + 2 × spread` wide at its bottom, `depth`
  in: wider inside than at its mouth, so the tail goes in only along the
  axis it is cut through.
- **`dovetail_tail`**: on the end of the part, centred across its own width
  and cut straight through its third axis. The tail is `neck` wide at its
  shoulder, `depth` from the end, and widens by the spread each side to the
  end; what is cut away is the wood either side of it. A tail and a socket
  with the same `neck`, `depth` and slope fill each other exactly. Where the
  tail is wider than the part, the part's own edge is its side.
- **A sliding dovetail** is the same two shapes on `crossing`, for a part
  that enters the face of another along its whole length (a batten housed
  across the underside of a panel). `dovetail_socket` on `crossing(p, q)` is
  a housing in `p`'s face: its sides lie across `q`'s width, centred on the
  footprint, `neck` wide at the face and `neck + 2 × spread` at its floor,
  `depth` in, and it runs along `q`'s length straight through `p` and out
  past its edges. `dovetail_tail` on `crossing(p, q)` is the tongue along
  `p`'s own length: its sides lie across `p`'s width, centred on the
  footprint, `neck` wide at its shoulder `depth` below the face and wider by
  the spread each side at the face; the wood either side of it is cut away
  over that `depth`, the whole length of `p`. A housing and a tongue with the
  same `neck`, `depth` and slope, cut on the faces where the two parts meet,
  fill each other exactly, and the tongue can only slide in along its length.
- **A dovetail's slope** is given one of two ways, never both
  (`error[type-mismatch]`) and never neither (`error[missing-field]`):
  `angle: 10deg`, from straight, for a spread of `depth × tan(angle)` each
  side; or `slope: 1 in 8`, the way dovetail markers are sold, for a spread
  of `depth / 8`. A slope is a `Ratio` more than 0 (`1 in 8` is `1/8`);
  `0 in 8` is `error[type-mismatch]`.
- **A `slot` at an angle**: the rectangle `width` wide through the
  footprint's centre (from it, with `from_centre: true`), turned in the face
  by `angle`, out past the part's edges.

The marks are exact, and so is a spread given as a slope. Where a shape needs
a number that is not a whole number of nanometres (a dovetail's
`depth × tan(angle)`, a `depth / n` that falls between two nanometres, the
corners of a slot turned by an angle), it is worked out once from the exact
values and rounded to the nanometre, like a hole's position. A dovetail's
`angle` must be less than 90deg (`error[type-mismatch]`).

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

A `kind` with a hardware base declares a product or a family of products:
the parts it joins, where it sits, its members and their shapes, how they
move and what it cuts, and optionally a 3D model for the viewport. Nothing
about a slide, a hinge or a pull is built into the checker: the standard
kinds in `std:hardware` are written in `.stave` like any other.

```
/// A full-extension slide; a family, chosen by length.
kind side_mount_slide : slide {
  roles {
    piece: Part                      // the drawer side
    in:    Region                    // the opening it runs in
  }
  param length = 500mm within [250mm, 300mm, 350mm, 400mm, 450mm, 500mm, 550mm, 600mm]
  clearance: 12.7mm
  travel:    length

  // On the side of the drawer box nearer the cabinet, its front end level
  // with the side's, centred up and down.
  mount {
    face:   face(piece, nearer: in)
    across: center(piece)
    along:  from(end(piece, toward: front))
  }

  member drawer_member {
    with:  piece
    shape: box(along: 0mm..length, across: 45mm, depth: -6.35mm..0mm)
  }
  member cabinet_member {
    with:  in
    shape: box(along: 0mm..length, across: 45mm, depth: -12.7mm..-6.35mm)
  }

  // Pulled out toward the front: along runs back from the front end.
  mechanism {
    rail drawer_member, cabinet_member along along
    drive: travel(drawer_member, from: 0mm, to: -length)
  }
}

/// One exact product: a knob on one screw from behind.
kind brass_knob_26 : pull {
  roles {
    piece: Part
  }
  screw_diameter: 4mm
  mount {
    face:   front
    across: open
    along:  open
  }
  member knob {
    with:  piece
    shape: cylinder(diameter: 26mm, depth: -30mm..0mm)
  }
  member screw {
    with:  piece
    shape: cylinder(diameter: 4mm, depth: 0mm..piece.thickness)
  }
  cut screw_hole: crossing(piece, screw) { drill(diameter: 4mm, depth: through) }
  model: model("./models/brass_knob_26.glb", align: node("mount"))
}
```

A butt hinge sinks a leaf into each part and cuts a mortise for each (its
rule `leaf_fits` keeps each leaf on the wood: no wider than
`contact(piece, on).width`, the patch where the two parts close, which on
a box side's edge is the side's thickness); a cup hinge bores its cup; a
pull drills a hole for each screw. Each is a `cut` of the kind (see
[Faces to cut](#faces-to-cut)). `fastener` kinds are
only ever used through a `uses:` or as what a joint member is (`member pin :
dowel_pin(...)`), and `tool` kinds are listed in a
project's `tools:` and a recipe's `tools:`.

**Envelopes.** Outside a member, `box(length:, width:, thickness:)` and
`cylinder(diameter:, length:)` are envelopes, the only values a fastener's
optional `envelope` entry takes. Nothing reads an envelope: no check and no
drawing. A fastener placed as a [joint member](#joint-members) is drawn
and checked by the member's shape, and one in a `uses:` is only counted.
The entry is still accepted, so an older library checks, but the standard
fasteners don't write it.

#### Adjustment

Real hardware has screws that set it: a cup hinge's arm slides on its
plate to bring the door in or out. A kind says so with `adjust`:

```
  adjust arm on plate: across within 2mm
```

`adjust a on b: axis within d` says that member `a` can be set up to `d`
either way on member `b`, along `axis` (`along`, `across` or `depth`) of
the [kind's frame](#the-kinds-frame). Both are members with a body
(`error[unknown-name]` otherwise), and on that axis exactly one of them
has its own `mount` (`error[missing-field]` when neither has, as nothing
would ever move it; `error[type-mismatch]` when both have). `d` is a
`Length` and may use the kind's params; an axis is adjusted at most once
for a member (`error[duplicate-name]`).

The rest pose is solved within the adjustments: the design places the
parts, each member's mount places it, and the **setting** of `a` on `b` is
how far `a` then sits from its nominal place on `b`: the offset of `a`
less the offset of `b` on that axis (see [Hardware
placement](#hardware-placement); a member without a mount on the axis has
offset `0mm`). Everything is drawn and checked at that setting, which is
part of the solved design. A setting more than `d` either way, or an offset
on an axis no `adjust` covers, is `error[adjust]` at the hardware's kind
call:

```
error[adjust]: door.hinges[bottom]: the arm would sit 8.5mm off its place on the plate; it adjusts 2mm
  --> project.stave:62:34
   = note: the arm's setting across is 8.5mm; adjust arm on plate: across within 2mm
   = note: the same for door.hinges[top]
   = help: use plate_height: 9mm, from the kind's list
```

The other pieces of hardware of the same call (the elements of an array)
that fail the same way are a note on the first error, `the same for
<hardware>`; so are those that fail the same `require`.

The help is the general search over the kind's `within` lists (see
[Checks and `require`](#checks-and-require)): each value of each list, one
param at a time with the others as given, with the hardware placed again;
it prints the values that bring every setting within its adjustment, and
never a value the kind would reject.

**Catalogue rules are `require`s of the kind.** A hardware kind's
`require` is checked for each piece of hardware where it is placed. In it,
`mount` names the mount face as an anchor, so `between(mount, in)` is the
gap between the face the slide is mounted on and the same side of its
opening:

```
  require gap : between(mount, in) == 12.7mm "the slide needs exactly this much beside the drawer"
```

A failed rule is `error[require]` at the hardware's kind call, with the
help search; when the condition is `a == b`, a note gives both sides (`the
rule measures 10mm and needs 12.7mm`). Hardware whose `require` or
`adjust` failed is left out of the overlap check and the motion checks:
it is not where it can be, and one mistake is one error.

#### One collision model

Every solid in the design is a **body**: every part with its cuts made,
every shape of every hardware member, wood or steel, and every copy of
every joint member (a dowel's pins, see [Joint members](#joint-members)). Each body is posed by
the motions: a member moves with the part or region it is `with:` (or with
the member it rides on), and a member the mechanism places (a link) moves
as the mechanism moves it. One overlap check covers every body at rest and
at every scene's poses, and one sweep covers every motion (see
[Scenes](#scenes)). Two bodies may touch (within the 0.05mm contact
tolerance); they may share room only where a cut has made it: a bore, a
mortise, a pilot hole. A piece of hardware makes its own room with its
`cut`s, as a joint does: a cup sits in its door because the kind bores the
hole, and a kind without that cut overlaps its own door (`error[overlap]`).
There is no way to switch the check off and no pair is let off; two rules
hold for every kind alike:

- **A kind's own members are not checked against each other.** Stave
  checks a design, not a product: how a hinge's links fold inside its arm
  is the maker's drawing, not the design's. Members of two different
  pieces of hardware, or of two joint pairs, are checked against each other
  like anything else; the copies of one pair's members are one piece.
- **Fasteners named in `uses:` are counted, not placed, and not modelled**
  (see `uses:`). A screw that is a member (a knob's screw, a screwed
  joint's screws) is a shape like any other, and the kind drills its hole,
  so it fits and touches.

Only declared shapes are drawn, and everything drawn is checked; every
member has at least one shape (see [Shapes](#shapes)).
Diagnostics name a member as `<hardware>'s <member>`: `error[overlap]` at
rest and at a scene's poses, `error[motion]` in a sweep
(`door_l.hinges[upper]'s inner_link hits door_l.stile when door_l is
opened past 98.2deg`).

**Sweeps see cuts.** In the plane a hinge turns in, a part's wood is its
boxes less its cuts wherever that takes away no more than the cut does: a
round hole whose axis lies in the plane (a cup's bore) is taken out as its
narrowest chord over the width, along the hinge's axis, that the other body
shares with it. Box cuts are already out of the boxes. Other holes and
angled cuts are left as wood there: a contact may be reported that a finer
model would clear, never the reverse. A slide's travel and a turn about one
fixed line are swept exactly; a linkage, and every member its mechanism
moves, by conservative advancement to the contact tolerance.

**Models are for looks; the text is checked against them.** The `.stave`
numbers are the truth: a model is drawn, never measured, and no check
reads it. What Stave does read is enough to tell when the two disagree.

- **Format:** glTF 2.0 binary (`.glb`), one file, at most 20 MB.
- **Sources:** the same forms as imports. A relative path is relative to the
  file that declares the kind. A model in a standard module or at a remote
  source is not read in this version.
- **Frame:** the model is drawn in the [kind's frame](#the-kinds-frame),
  closed, in glTF's convention: units are metres, +X is `along`, +Y stands
  off the mount face (it is `-depth`), +Z is `across`. Modelled in Blender
  (Z up), the hardware sits on the ground plane with its mount face on it,
  `along` Blender's +X and `across` Blender's -Y; Blender's glTF exporter
  with "+Y Up" turns that into the convention above.
- `align: node("mount")` (optional) puts the node named `mount` on the
  kind's origin; without it the model's own origin is the kind's origin.
- `moving:` names the node that moves with the hardware's motion.
- A missing model is drawn as its members, with a label, and is not a
  diagnostic.

**Checked against the text** (`warning[model-mismatch]`, once for each
piece of hardware whose kind has a model it can read, with its params as
that piece of hardware sets them). Each node is found by name:

| Node name | Is compared with |
|---|---|
| `<member> <shape>` (`arm knuckle`) | the bounds of that named shape of the member |
| `<member>` (`knob`) | the bounds of the member's unnamed shapes together |
| `<a> <b>` (`arm inner_link`) | the point of `pivot a, b at point(...)`, the two names in the order written |
| `<a> <b> from`, `<a> <b> to` | the two ends of `slot a, b from ... to ...` |

A node's point is its origin and its bounds are those of its own mesh,
after every transform above it; a shape's bounds are its smallest box in
the kind's frame (a bar's, round its corners). A point or a corner of
the bounds more than **0.01mm** from the text is a warning that gives both
values, and the help names the usual cause: glTF is metres with +Y up,
while Blender is Z up and is often set up in millimetres, and an object's
scale or rotation that was not applied (Ctrl+A in Blender) is exported as
a transform. When the model would match turned Z up, or 1000 times larger
or smaller, the help says that one. A node the text needs that the model
lacks is a warning listing every missing name (a shape's node also needs
a mesh). A file that is not a glTF 2.0 binary, or is over 20 MB, is a
warning too.

**Pinning a file.** `model("./knob.glb", sha256: "<64 hex digits>")` pins
the model to one file: when the file's SHA-256 differs, that is
`warning[model-mismatch]` giving the file's hash, so a changed model is
looked at before its pin is updated. A `sha256:` that is not 64
hexadecimal digits is `error[type-mismatch]`.

#### Shapes

A hinge, slide or pull kind is made of **members**, each written `member
name { ... }` with a doc comment a beginner can read. A viewer numbers the
members in the order written, labels them on the drawing and shows the doc
comment as the caption. A member may have **any name**; nothing in Stave
knows a member by its name. Its block says what carries it (`with:`) and
what it is made of: at least one `shape` (`error[missing-field]`
otherwise, and so is a member written without a block, `member 'cup' is
missing 'shape'`):

```
  /// The arm: it clips onto the plate and reaches out to the door.
  member arm {
    with:          on
    shape body:    box(along: 16mm, across: -2.9mm..15mm, depth: -55mm..0mm)
    shape knuckle: box(along: 16mm, across: 8mm..15mm, depth: 0mm..12.4mm)
  }
```

`with:` names a role (the member moves with that part: `piece`, `on`) or
another member (it rides on that member). A member with no `with:` is
placed by the mechanism, as a link is; in a kind whose mechanism doesn't
place it (or that has none) nothing moves it, which is
`error[missing-field]` at the member. Two members of one kind have
different names (`error[duplicate-name]`); two shapes of one member have
different names (`error[duplicate-name]`), and a member with more than one
names them all. A member's own `mount { }` places it on some axes of the
frame from something other than the kind's origin (see [Hardware
placement](#hardware-placement)). Fasteners, tools and joints have no
members: `member head` in one is `error[unknown-field]` (`a fastener kind
has no member 'head'`).

A member's shapes are what it is made of: convex solids, written in the
[kind's frame](#the-kinds-frame), with the hardware closed. Sizes and
points may use the kind's params (as every expression in a kind may; its
fields are read from outside it, `hinge_kind.cup_diameter`), so
`euro_hinge(plate_setback: 52mm)` is still a family. Each range argument
is a range `lo..hi` (`-2.9mm..15mm`) or one length, which is centred on the
origin (`along: 16mm` is `-8mm..8mm`). A shape's arguments are named, each
at most once.

| Shape | Meaning |
|---|---|
| `box(along:, across:, depth:)` | a box; each argument is a range or one length |
| `cylinder(diameter:, depth:, at:)` | a cylinder whose axis runs along `depth`, through `at` (a `point(...)`, default the origin); `depth` is a range or one length. `axis: along` or `axis: across` lays it down, and its range is then written under that word (`along:` or `across:`) instead of `depth:` |
| `prism(points: [...], along:)` | a convex outline of `point(across:, depth:)`s, in order around it, extruded along `along` (a range or one length) |
| `bar(from:, to:, thickness:, along:)` | a straight bar between two `point(...)`s, `thickness` wide in the across/depth plane and reaching half its thickness past each point, `along` long: a prism, written the way a link is made |

Outside a member, `box(length:, width:, thickness:)` and
`cylinder(diameter:, length:)` stay envelopes (see
[Built-in functions](#built-in-functions)).

#### Mechanisms

A hardware kind says how its members move in one `mechanism { }` block.
Every name in it is a member with a body declared in the kind
(`error[unknown-name]` otherwise); the two members of one entry are
different.

| Entry | Meaning |
|---|---|
| `pivot a, b at point(...)` | `a` and `b` turn on one pin there |
| `slot a, b from point(...) to point(...)` | a pin on `a` runs in a straight slot in `b` |
| `rail a, b along <word>` | `a` runs straight along `b` without turning, along `along`, `across` or `depth` |
| `couple a to b: <ratio>` | `a` travels that share of `b`'s travel (a full-extension slide's middle member) |
| `drive: turn(m, from:, to:)` or `drive: travel(m, from:, to:)` | the one input and its range: member `m` turns through angles, or travels through lengths |
| `stop: <value> "message"` | what stops the drive there; the message is shown when a pose or a design asks for more |
| `pin_diameter: <length>` | how thick every pin is (optional, default `0mm`) |

`drive`, `stop` and `pin_diameter` are optional, each at most once. A
drive's `to` beyond its `stop` is `error[motion]`, with the stop's
message. A drive may run either way from its `from`: a slide whose `along`
runs back from its front end pulls out toward the front with
`travel(drawer_member, from: 0mm, to: -length)`. A pose or a scene's
amount is measured from the closed pose toward `to` (a drawer pulled out
200mm), whichever way that runs.

**The plane.** A mechanism is planar. Its members move in the across/depth
plane and every pin is parallel to `along`; a mechanism with a `rail ...
along along` (a slide) moves in the along/depth plane instead, its pins
parallel to `across`. A rail along the pins' axis is `error[mechanism]`.

**One freedom.** Three for each member the mechanism moves (the driven
member and each member with no `with:`), less two for each `pivot` or
`rail` and one for each `slot` or `couple`: a mechanism with a `drive`
must come to exactly one, which the drive takes, and one without a drive
to none once its assembly's motion is given (`error[mechanism]` at the
kind otherwise, giving the count; counted on its own as soon as a piece of
hardware is placed, and reported once per kind). The driven member is
`with:` the part that moves (the door, the lid, the drawer side): the
assembly moves with it (`error[mechanism]` otherwise). A member with no
`with:` that no entry reaches from what is placed is `error[mechanism]`
("nothing places ..."). How the members are then placed is in [How a
mechanism is solved](#how-a-mechanism-is-solved).

```
  mechanism {
    pivot arm, inner_link at point(across: 13.5mm, depth: 10.9mm)
    pivot cup, inner_link at point(across: -8.8mm, depth: 7.33mm)
    pivot arm, outer_link at point(across: 13.28mm, depth: 3.56mm)
    pivot cup, outer_link at point(across: -9.54mm, depth: 1.39mm)
    drive: turn(cup, from: 0deg, to: opening_angle)
    stop:  opening_angle "the limiter in the arm stops the door here"
  }
```

That is a cup hinge's four-bar linkage. A butt hinge is `pivot leaf,
fixed_leaf at point(...)` with `drive: turn(leaf, ...)`; a slide is `rail
drawer_member, cabinet_member along along` with `drive:
travel(drawer_member, ...)`; a full-extension slide adds a middle member
on two rails and `couple middle_member to drawer_member: 1/2`. The jargon
stays in library files: a project still writes `hardware hinges[lower,
upper] : euro_hinge(stile[hinge], on: side_l)`.

**Several items on one assembly.** An assembly has one motion, from the
hardware inside it (not inside an assembly within it). Its items with a
`drive` must all move it alike: seen in the first one's plane (the first
in the design's order), the same members, held the same way, the same
entries at the same points (to the nanometre) and the same drive from the
same `from`. Two cup hinges of one kind and setting on one door do; a cup
hinge and a butt hinge on one door don't (`error[motion]` at the second,
"`door`'s hardware doesn't move alike", naming both). Every pin of every
item is parallel to one axis (`error[motion]` otherwise). Items without a
drive **follow** (a lid stay): their members `with:` a part that moves
ride on the first driver's driven member, those `with:` a part that
stays are held, and the rest are placed by the same steps. The assembly
opens as far as the nearest end among its drivers' ranges and its
followers' stops, and the nearest `stop` gives its message. An assembly
whose hardware has no drive doesn't move.

#### How a mechanism is solved

A hardware kind's `mechanism { }` (pivots, slots, rails, couples and one
drive, in the kind's frame with the hardware closed) is solved the same way
for every kind, in closed form: no iteration, no guessing. An assembly's
mechanism (its drivers' and its followers', see [Mechanisms](#mechanisms))
is solved once; a piece of hardware no assembly moves is solved on its
own, so a mechanism that can't move is reported wherever it is used.

**The words.** A member is **held** (`with:` a role whose part stays put),
**free** (the mechanism places it; the driven member is free even when it
is `with:` the moving part) or **carried** (`with:` another member, one
rigid piece with it). The declared pose is the drive at its `from`; at a
value `v` the driven member has turned, or travelled, by `v - from`. A
positive turn carries `across` toward `depth`. A `slot a, b from P to Q`
has `a`'s pin at `P` when closed and lets it run to `Q`. A member's
**travel** is how far it has moved along its rail from its closed place,
measured against the held members; `couple a to b: r` makes `a`'s travel
`r` times `b`'s (a full-extension slide: the middle member travels half
the drawer member's travel).

**One freedom.** Three for each free member, less two for each `pivot` or
`rail`, less one for each `slot` or `couple`, must come to exactly one,
which the drive takes (`error[mechanism]` otherwise, giving the count). An
entry between two pieces of one rigid member, or between two held members,
is `error[mechanism]`.

**The steps.** Starting from the held members and the drive, the solver
repeats, until every member is placed, the first of these that applies:

1. **Couple**: a couple with one side's travel known gives the other's.
2. **Hang**: a member whose angle is known (the driven member of a turn),
   pinned to a placed member, is placed (a butt hinge).
3. **Rail**: a member whose travel is known, on a rail along a placed
   member that does not turn, is placed (a slide).
4. **Hung on two**: a member whose angle is known, held by two of: a link
   pinned to a placed member, a slot in or over a placed member, a member
   on a rail along a placed member. Its offset lies on a circle and a
   circle, or a circle and a line (a cup hinge's cup, hung on its two
   links from the arm).
5. **Two links** (a dyad): a point where two unplaced members meet, or the
   pin of a slot in a placed member on an unplaced link, lies on two paths,
   each a circle (a link pinned to a placed member) or a line (a rail along
   a placed member, or the slot): circle and circle, or circle and line (a
   slider-crank's rod and slider).
6. **Slot in a link**: a link pinned to a placed member, its slot over a
   pin on a placed member (a lid stay).

When no step applies and members are left, that is `error[mechanism]` at
the kind: "this mechanism can't be solved one step at a time from its
drive", naming the members left over. An entry still unused when every member is
placed holds the mechanism still: `error[mechanism]`. Two straight paths
for one point (a pin on a rail running in a slot) are not solved in
version 1.

**The branch.** Two circles, or a circle and a line, cross in two points.
The one the closed pose is on is chosen exactly, in integers, from the
declared points (which side of the line between the two centres, or which
way along the line); a closed pose exactly on a dead point is
`error[motion]`. The mechanism keeps that branch by continuity over the
whole range.

**Dead points.** The drive is run through its whole range, every 0.05deg
or 0.05mm, in floating point at the geometry boundary. At every sample
each two-answer step must stay at least 0.1mm from a dead point: two
circles' centres 0.1mm inside the sum of their radii and outside their
difference, a line 0.1mm inside its circle. A step that comes closer, or
whose paths no longer meet, is `error[motion]` at the hardware, naming the
step's members and the first drive value where it happens ("`rod` and
`slider` lock at 41.6deg"). Between samples the margin is not guessed but
proven: from the declared lengths and how far each point is from what
turns it, the checker bounds how fast each step's margin can change, and
shows from the two samples either side that it stays at least 0.1mm over
the whole interval, so a dead point narrower than one sample step is not
missed. Where it can't show that, it halves the interval and tries again,
down to 0.00005deg or 50nm; an interval still not shown clear is
`error[motion]` at the hardware, naming the step's members and the sample
nearest the dead point ("`rod` and `slider` may come within 0.1mm of
locking near 44.968359deg"). A step that would jump to its other branch
between two samples is `error[motion]` ("flip over at"), and so is a pin
that runs past an end of its slot ("`lid`'s pin reaches the end of its slot
in `stay` at <value>").

**Fast paths.** Two shapes of motion are swept exactly, chosen by the
mechanism's shape and never by its kind: a driven member pinned to a held
one turns the assembly about that pin's line (a butt hinge), and a driven
member on a rail along a held one moves it in one direction (a slide).
Every other motion, and every member the mechanism places, is posed by the
solver.

**Version 1 limits.** Planar, one freedom, the steps above. A travelling
drive and a couple act on rails that do not turn. An assembly inside a
moving assembly is not swept: its motion is posed on top of its parent's
in scenes, but only the outer one is swept.

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
| `dovetail_socket` | `walls`, `floor`, `waste`, `outline` | `neck`, `depth`, `angle`, `slope` |
| `dovetail_tail` | `cheeks`, `shoulders`, `waste`, `outline` | `neck`, `depth`, `angle`, `slope` |
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
`tenon`, `spigot` and `dovetail_tail` remove the end (or, for a tongue, the
edge) they are cut on, a
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
| `between(a, b)` | `Length` | the clear distance between two parts, or between two parallel anchors, or from a face to the same side of a region (`between(side.left, opening)` is from the side's left face to the opening's left side). Two parts must be apart along exactly one axis |
| `extent(items, axis)` | `Length` | overall drawn size of a list of parts along a world axis. With `of: part`, `axis` is one of that part's own axes (`length`, `width`, `thickness`), meaning the world axis it runs along once placed. In a joint kind this measures the other part along this one's direction: `extent(into, thickness, of: piece)` is how much of `into` a screw driven through `piece`'s thickness can bite into (its thickness for a face, its width for an edge). With `of: contact(a, b)`, `axis` is one of the patch's own axes: `length` and `width` along it, `thickness` square to it, so `extent(into, thickness, of: contact(piece, into))` is how much of `into` lies behind the place where `piece` meets it |
| `mid(item, axis)` | `Anchor` | the mid-plane of a part or region |
| `opening(...)`, `split(...)`, `cover(...)` | `Region` | see [Regions](#regions); `split` takes `at:` or `shares:` |
| `overlay(...)`, `inset(reveal:)` | `Mount` | see [Regions](#regions) |
| `turned(profile)` | `Shape` | turned on a lathe to a profile |
| `joined(by:)` | `Boards` | how the boards of a part are joined: `by:` is a joint kind value or a list of them; see [Boards](#boards) |
| `spliced(by:, at:, from:)` | `Splices` | where a part is spliced end to end into pieces: `by:` a joint kind value, `at:` a length or a list of them, measured from the end `from:` names; see [Splices](#splices) |
| `box(length:, width:, thickness:)` | `Envelope` | a fastener's `envelope`, which nothing reads (see [Hardware](#hardware)). Written `box(across:, up:, deep:)`, with those three argument names, it is instead the `Region` that is the project's root; see [Regions](#regions) |
| `cylinder(diameter:, length:)` | `Envelope` | a fastener's `envelope`, which nothing reads (see [Hardware](#hardware)) |
| `model(source, align:, sha256:)` | `Model` | a `.glb` model; optional `align: node("name")` and `sha256: "<hex>"` (see [Hardware](#hardware)) |
| `ends(inset:)`, `pitch(d)`, `gap(d)` | `Spacing` | used by `spaced`, with the bare words `evenly` and `ends` |
| `contact(a, b)` | `Patch` | the patch where two parts touch, read as `.length` (its longer side) and `.width` (its shorter side); see below |
| `edge_to_edge(a, b)` | `Bool` | `true` when two parts lie side by side in one plane and meet along an edge of each; see below |
| `wall(cut)` | `Length` | the least wood between one of the kind's named cuts and any face or other cut of the same part, read only in a kind's `require`; see [Walls](#walls) |

**`contact(a, b)`** is where two parts touch: a flat patch, measured on
the parts' drawn boxes, before joints lengthen them (a tenon reaching into
its mortise doesn't count as touching). Where the parts butt, the patch is
the overlap of the two faces that meet; where they cross (a half lap), it
is their overlap seen square to `a`'s faces. A turned part that meets the
other with an end touches only over its profile's diameter at that end.
`a` and `b` are one part each; a role of type `Parts` is one part in each
pair. Two parts that don't touch over an area are `error[no-contact]`
(`side_l and top don't touch`) at the call, with the help to place one
against the other. A `Patch` is read only for its sizes, `.length` and
`.width`, both `Length`s, as `of:` in `extent` (above), and as a joint
member's `on:` (see [Joint members](#joint-members)); the sizes are exact,
never rounded. A kind's rules use it to size a joint to the wood it
sits on:

```
require seat_wide : contact(piece, into).width >= diameter + 2 * wall "the parts overlap too little for these dowels"
require seat_long : contact(piece, into).length >= count * (diameter + wall) + wall "too many dowels for this joint"
```

**`edge_to_edge(a, b)`** is `true` when two parts lie side by side in one
plane and meet along an edge of each, as the boards of a glued-up top do.
Of a part's six surfaces, the two square to its thickness are its
**faces**, the two square to its width its **edges** and the two square to
its length its **ends** (for a part with `grain: width`, whose length runs
across the grain, the two square to its length are its edges and the two
square to its width its ends: edges always run along the grain). Two parts
are edge to edge when all three hold:

1. their thicknesses run along the same world axis once placed;
2. they touch: their drawn boxes meet over an area without sharing wood
   (parts that cross, as in a half lap, are not edge to edge);
3. where they touch is an edge of `a` and an edge of `b`.

Boards of different thickness still count; a kind that needs the same
thickness says so in a rule of its own. An edge against an end (a
breadboard end) is not edge to edge, nor is an end against a face (a
case's side standing on its base), nor are two edges meeting at right
angles. It is worked out on the parts' drawn boxes and orientations,
exactly, as `contact` is, before joints lengthen the parts (a tongue in
its groove doesn't make two boards cross). Parts that don't touch are not
edge to edge: the answer is `false`, not an error of its own. (A joint's
pairs are only parts that meet, and a listed part that meets none is
`error[no-contact]`; see [Roles and pairs](#roles-and-pairs).)

`a` and `b` are one part each, as for `contact` (anything else is
`error[type-mismatch]`); a role of type `Parts` is one part in each pair
the joint joins. It may be written wherever `contact` may: in a
project's or a component's `require` naming two parts, and in a joint or
hardware kind's `require`, reading its roles. A bare
call is a whole condition, since a condition is any `Bool`; it combines
with `&&`, `||` and `!` like any other. The standard edge joints
(`glued_edge`, `tongue_and_groove`, `splined_edge`, `chigiri`) use it to
say how their boards must meet:

```
require side_by_side : edge_to_edge(piece, into) "a butterfly key joins two boards lying side by side, edge to edge; these meet another way"
```

The boards of a part (see [Boards](#boards)) are always edge to edge, so
the rule holds at every seam of a part made of boards. A kind's rule that
asks how its parts meet is checked before its other rules; see
[Checks](#checks-and-require) for what a failure of it does.

Inside a hardware kind's `member` and `mechanism` only, written in the
[kind's frame](#the-kinds-frame) (anywhere else, `point(...)` is
`error[syntax]`):

| Function | Meaning |
|---|---|
| `point(along:, across:, depth:)` | a point in the kind's frame; each a `Length`, default `0mm` |
| `bar(from:, to:, thickness:, along:)` | a member's shape: a straight bar between two points (see [Shapes](#shapes)) |
| `prism(points: [...], along:)` | a member's shape: a convex outline extruded along `along` |
| `box(along:, across:, depth:)`, `cylinder(diameter:, depth:, at:)` | a member's shape; each argument a range `lo..hi` or one length |

Words that look like functions but are fixed by position, not built-in
functions: placement terms, face selectors, a hardware `mount`'s terms and
anchors (`from`, `center`, `edge`, `end`, `face`), cut operations,
`mark(...)` inside `outline`, assembly steps, `extend`, recipe steps,
`before`/`after`, and `mirror`.

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
- **Motions belong to assemblies.** The hardware that has a drive,
  declared inside a component, gives each instance one motion: the whole
  assembly moves, as one piece, relative to everything outside it, and the
  members its mechanism places (a link, a stay's arm) move as the
  mechanism moves them. Every item with a drive must move the assembly
  alike, and items without one follow (see [Mechanisms](#mechanisms),
  several items on one assembly).
- `pose` names the assembly and sets `open` (as far as its mechanism
  reaches: a hinge's `opening_angle`, a slide's travel, or sooner where a
  `stop` says), `closed`, an angle (something that turns) or a length
  (something that slides), measured from closed. A pose past where the
  mechanism reaches is `error[motion]` at the pose, with the `stop`'s
  message when a stop ends the range ("`door` can't open 170deg: the
  arm's limiter stops the door here"). A mirrored assembly has the mirror
  image of its source's motion.
- Every scene is collision checked, and every motion is checked through its
  whole travel, not only at its ends (`error[motion]`): every body that
  moves (a door, the hardware on it, a member its mechanism places) is
  swept from closed to open, against
  every body that moves differently, and the message names the first angle
  or distance at which they hit (see [One collision
  model](#one-collision-model)). Each motion is checked from the rest
  position and from every scene's poses; the first contact is found to
  within the 0.05mm contact tolerance, never stepped over. A door on cup
  hinges follows its linkage (it turns and moves at once); a door on butt
  hinges turns about its pin (see [How a mechanism is
  solved](#how-a-mechanism-is-solved), fast paths).
- `explode: true` draws the scene with every part moved out from its rest
  position along the direction its joint is assembled (the reverse of the
  joint's last `insert` step), so the joinery shows. Assemblies move out
  first, each as a whole (a drawer forward along its slides, clear of its
  depth; a door or lid off its hinges, toward the side it swings to; any
  other along the joint that attaches it), then the parts inside each
  assembly, and the parts in none, move apart about where it went. A part
  that would
  leave two of its pairs in opposite directions (a rail tenoned into two
  stiles) is held: the parts it joins move away from it instead, and,
  when nothing else moves it, it goes with whatever those parts all do
  besides, except along its own joints (a rung follows the legs it joins
  as they drop off the seat, so they pull off its ends); a held part
  whose parts share no move stays. A
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

**When a kind's rule fails, the help searches the kind's lists.** A `require`
of a kind that fails, and whose condition compares for equality (it is
`a == b`, or conditions joined by `&&` with at least one `a == b` among
them), gets a help line found by search: the checker tries every value of
each of the kind's `within` lists, one param at a time with the others as
given, and prints the values that make the rule hold, in the order the params
and their lists are written. It never prints a value the kind would reject:

```
error[require]: 'thirds' fails: the slot is a third of the post's thickness
  --> project.stave:44:14
   = note: for rail and post in corner
   = note: the rule measures 18mm and needs 24mm
   = help: use slot: 8mm, from the kind's list
```

When no value of any list makes the rule hold, the help says so and names the
lists that were tried:
`help: no value in the kind's lists makes this hold (slot within [6mm, 8mm,
10mm, 12mm]); change the parts' sizes`. Only lists are searched: a param
`within` a range (`within 2mm..9mm`) is left as it is. A rule that only
compares for order (`<`, `>=`), or a kind with no `within` list, keeps the
general help. The same search runs for a hardware kind's `require` and for
its adjustments (`error[adjust]`, see [Adjustment](#adjustment)), where each
trial places the hardware again.

**A failed kind rule says what it measured.** When the condition compares
two values (or joins comparisons with `&&`, of which the first that fails
is taken), a note gives both sides: the left is what the rule measures, the
right what it needs, `at least`, `more than`, `at most` or `less than` it
for `>=`, `>`, `<=` and `<`, and exactly it for `==`
(`the rule measures 5.5mm and needs at least 16mm`). Other pairs of the same
joint that break the same rule are a note on the first error, `the same for
side_r and top in top_dowels`, as for hardware. When the search offers
nothing, the help is, in order: for a comparison that reads
`contact(a, b).width` or `.length` where one part covers only some of the
other's end, edge or face along that side, how much and what to move; or
else the general help, naming the kind's params the comparison reads
(`change the joint's count, diameter or wall, or the parts' sizes, so the
rule holds`), or `change the joint's settings or the parts' sizes so the
rule holds` when it reads none. The reported cabinet whose top covers only
part of each side's end:

```
error[require]: 'seat_wide' fails: the parts overlap too little for these dowels
  --> carcass.stave:102:22
   = note: for side_l and top in top_dowels
   = note: the rule measures 5.5mm and needs at least 16mm
   = note: the same for side_r and top in top_dowels
   = help: top covers only 5.5mm of side_l's 19mm end; move or resize top so it covers the whole end
```

**A rule that asks how the parts meet comes first.** A joint kind's
`require` whose condition reads `edge_to_edge(a, b)` is checked for each
pair before the kind's other rules. When it fails, the joint is not one
for that pair: the kind's other rules are not checked there, and nothing
is cut, made, added or placed for that pair, so the rule's
`error[require]` is the only error the pair gives (not also a tongue, a
key or a spline that can't be cut where it would go). Its note says how
the parts do meet, and its help is to choose another kind:

```
error[require]: 'side_by_side' fails: a butterfly key joins two boards lying side by side, edge to edge; these meet another way
  --> case.stave:49:23
   = note: for case.side[left] and case.base in case.base_joints
   = note: case.side[left]'s end meets case.base's face at right angles
   = note: the same for case.side[right] and case.base in case.base_joints
   = help: choose another joint kind, one made for parts that meet the way these do, or place the parts as the rule says
```

Requires are what an AI patch or a smart edit is judged against: a change that
breaks one is rejected, with the message.

Checking runs in layers; a layer runs only if the ones before it found no
errors in the same item.

1. **Syntax**: this grammar.
2. **Names and types**: names resolve, units match, fields and roles exist.
3. **Placement and sizes**: every axis set once, `fit` resolvable, no cycles.
4. **Fit and geometry**: parts meet where joints say they do and overlap
   nowhere else (using cut sizes and cuts); hardware sits within its
   adjustments and keeps its kind's rules.
5. **Motion**: every scene, and every motion through its whole travel, is
   collision free.
6. **Requires and ranges** (z3): every `require` holds, and layers 3 and 4 hold,
   for every value inside each `within` range.
7. **Buildability**: an order of operations exists. See
   [Order of operations](#order-of-operations).

### Walls

`wall(cut)` is the least wood between one of the kind's own cuts and
everything else of the part it is cut in: the part's faces, and every other
cut of that part, whichever joint or hardware makes it. It is how a kind
says how much wood a cut must leave round it, in the kind's own words:

```
kind socket : joint {
  ...
  cut socket: crossing(into, piece) { drill(diameter: piece.diameter, depth: depth) }

  /// Wood left round the hole: to every face of the board and to its other
  /// cuts, such as a dado or another socket.
  param wall = 6mm
  require socket_wall : wall(socket) >= wall "the socket comes too close to a face of the board or another cut"
}
```

- **What it names.** The argument is the name of one of the kind's cuts
  (see [Named cuts](#named-cuts)), written alone: `wall(socket)`. A name
  that isn't one of the kind's cuts is `error[unknown-name]` (`unknown
  name 'sokcet'`), with the kind's cut names in the help. `wall(...)` is
  read only in a joint or hardware kind's `require` (anywhere else it is
  `error[syntax]`: `wall(...) is read only in a joint or hardware kind's
  require`), and the rule is one comparison with `wall(cut)` alone on one
  side (`wall(hole) >= 3mm`, `3mm <= wall(hole)`); anything else is
  `error[syntax]`: `wall(...) is compared on its own in a rule`.
- **What it measures.** For each pair of a joint (or each piece of
  hardware), every copy of the cut on every part it is made on: the least
  of them. A cut always opens onto the face it is cut from, and a `through`
  cut onto the opposite one too: those are never walls. A closed cut
  (`drill`, `pocket`) is measured to every other face, so one that breaks
  out of a face has a wall of `0mm`; any other cut opens wherever it
  reaches a face (a groove runs out of the ends of its board), so only the
  faces it stays clear of count. Another cut counts unless one of the two
  lies wholly inside the other (a stub tenon's mortise in the panel groove
  it sits in takes no wood of its own). Copies of the same cut count, so a
  dowel's holes are walls to each other.
- **Round parts.** On round stock and turned parts the faces across the
  length are the lathe profile, at its narrowest along the hole: a hole
  into the side of a leg is measured from its bottom to the round surface
  (its mouth breaks the surface by design), a hole into the end from its
  side.
- **Exact where the shapes allow it.** Between boxes, from a shape to a
  flat face, and between parallel holes in line, the wall is a difference
  of exact lengths. On a slant (a hole past the corner of a pocket) it is a
  square root, and between two holes square to each other (two rungs into
  a leg from adjacent faces) it is found by projecting from one hole to the
  other until the distance stops shrinking; both are rounded down to the
  nanometre.
- **When it is measured.** Once every cut of the design is made, after
  the kind's other rules: a rule that reads `wall()` is checked in layer 4,
  with the fit checks, and reported like any failed rule of its kind, at
  the joint's or hardware's call, with what it measured and where:

```
error[require]: 'socket_wall' fails: the socket comes too close to a face of the board or another cut
  --> carcass.stave:103:22
   = note: for rod and side_l in rod_sockets
   = note: the rule measures 0mm and needs at least 6mm
   = note: the same for rod and side_r in rod_sockets
   = help: the socket in side_l leaves 0mm of wood at the dado that shelf_dados cuts in side_l; move the parts so the socket has more wood round it, or change the joint's settings
```

  The help says `runs 2mm past <place>` when the cut goes past the face or
  into the other cut. A rule that fails where something already overlaps
  (a pin running into another joint's tenon) is left to the overlap, which
  names what collides.
- **Not measured yet.** Dovetails, outlines and slots turned at an angle,
  as the cut measured or as another cut, and the cuts between the boards
  of a part made of boards: a rule about such a cut is left out, with
  `warning[not-supported]` (`measuring wall(tail) is not supported yet`).
  The range layer doesn't measure walls yet: they are checked at the
  current values.

### Cuts stay in their part

Five backstops hold for every kind, whatever its rules, because no kind
should be able to get them wrong. All but the splice backstop are
reported at the joint's or hardware's call, in layer 4; that one at the
part's `lengths:`:

- **Cuts only remove wood** (`error[cut-size]`). A `tenon` wider or
  thicker than the part across its end (for a tongue, along its edge), or
  a `spigot` wider than the part's end (its diameter at that end, for a
  round or turned part): `rail_stubs: its stub on rail is 25mm thick, but
  rail is only 19mm thick`, with the help `a stub is the wood left when the
  wood round it is cut away, so it can't be thicker than the part; make it
  less than 19mm thick`.
- **Stopped cuts leave wood behind** (`error[cut-size]`). A depth given as
  a length (`notch`, `slot`, `pocket`, `drill`, `outline`) must be less
  than the part there (for a hole into the side of a round part, to the
  far side of the round); only `through` goes through: `bottom_slots: its
  slot in front is 12mm deep, but front is only 12mm thick there`, with
  the help `a cut with a depth stops inside the part and leaves wood behind
  it; make the slot less than 12mm deep, or write depth: through if it
  should go right through`.
- **Closed cuts keep a wall** (`error[wall]`). Every `drill` and `pocket`
  leaves wood (a wall of more than `0mm`, see [Walls](#walls)) between it
  and every face it isn't aimed at, and every closed cut of another joint
  pair or piece of hardware: `side_pegs: its peg hole in base breaks out
  of the left end of base` (help: `the peg hole runs 5.5mm past the left
  end of base; move the parts so the peg hole sits inside base with wood
  all round it, or make the peg hole smaller`), or `post_a_bore: its bore
  in base runs into the bore of post_b_bore` (help: `the bore runs 2mm
  into the bore in base; move one of the joints so there is wood between
  the two cuts, or make one of them smaller`). A pin or screw that runs
  into another joint's empty hole is caught here, or by its kind's
  `wall()` rule: its own hole runs into the other. How near a hole may come
  to a groove or dado is a kind's judgement, not a backstop: write it with
  `wall()`.
- **A splice needs its wood whole** (`error[wall]`). On a spliced part
  (see [Splices](#splices)) every cut of another joint or piece of
  hardware is made on one piece, and none may reach into a splice, where
  both pieces' wood belongs to the splice: `shelf_dados: its dado in
  side_l crosses the splice between side_l.piece[1] and side_l.piece[2]`.
  How near a cut may come to a splice is the splice kind's `wall()` rule.
- **A part stays one piece** (`error[wall]`). Once every cut is made, a
  board's wood must be one connected piece: pieces that touch only along
  an edge are two. Each of two housings cut from opposite faces may keep
  to its own depth and still meet the other across the board, and a cut
  that goes `through` right across a part parts it; the cut list would
  still list one part. Reported at the cut that, made after the cuts
  before it, first parts the wood: `shelves: its housing for shelf_r
  meets the housing for shelf_l and cuts divider in two` (help: `together
  the two cuts go right through divider and leave no wood between them;
  make one of them shallower, or move one of the joints so the cuts don't
  line up`), or for one cut `shelves: its housing for shelf goes right
  across side and cuts it in two`. A cut of another joint is named `the
  <housing> of <joint>`. A part made of boards is tested as one board
  with its own cuts; round and turned parts are not tested yet.

One mistake, one error. A kind's own rule says it first: when one fails
the design isn't checked further, so std's `tenon_thin` speaks for a tenon
too thick and `groove`'s `not_too_deep` for a groove too deep, in the
joint's words, before any backstop. A pair or piece of hardware whose
`wall()` rule failed gets no backstop for the same cuts; a cut too big or
too deep gets no wall error, and its part is left out of the overlap
check, which would only report the same mistake as parts that no longer
fit. Two closed cuts that run into each other are one error, for the
first. Nothing here is reported for a part or member that already
overlaps something, and a part whose cuts already failed a `wall()` rule
or a size gets no error for being in pieces.

### Held

Every part must be held: joined to the part placed with `ground`, through
joints and hardware, or be meant to lift out. The parts a joint pair binds
(and the parts it makes) hold each other, as do a part made of boards and
the parts its board joints make; a piece of hardware holds the part it is
mounted on, the parts its roles name, the parts that carry its members and
the parts its members touch. A part that nothing joins to the ground part
this way is `warning[loose-part]` (layer 4), once for each group of parts
joined only to each other, at the group's first part. It is a question
about the whole project, asked only once the design has no errors: a part
with an error may be the one that would join the rest.

When the root is a box (see [Placement rules](#placement-rules-checked)),
there is no ground part: a group is held when one of its parts stands on
the floor, its bottom face on the box's bottom face. A group that doesn't
is `warning[loose-part]` with the message
`<part> is loose: no joint or hardware joins it to a part that stands on the floor`.

```
warning[loose-part]: slat[0] is loose: no joint or hardware joins it to top or to anything joined to top
  --> table.stave:120:6
   = help: join slat[0] to stretcher_front, which it touches, with a joint or hardware, or write loose: true in the part if it is meant to lift out
```

A group of several parts is named as `a, b, c, d and 11 more are joined
only to each other: ...`. The help names the held part the group touches
most, or, when it touches none, says to place it against one.

**Meant to lift out.** A part that is meant to rest in place or lift out
(a loose shelf, a tray, a removable lid, the samples on a bench) says so
with `loose: true`, a field of `part` (see [Declarations and their fields](#declarations-and-their-fields)).
A group with a loose part is not warned: the whole group lifts out
together, so one part of a tray is enough. `loose:` is a `Bool`, so a
component can take it as a param.

### Diagnostics

A diagnostic is printed compiler-style: `error[code]: message`, then
`--> file:line:column`, then notes and a concrete `help:` line. Codes and the
shape of their messages are fixed, because tests and AI tools match on them.

| Layer | Code | Message |
|---|---|---|
| 1 | `syntax` | `expected <what>, found <token>`; several expectations are joined with "or" |
| 1, 2 | `missing-value` | `'<name>' has no value` (layer 2 for a `locked` or `solved` param with no default) |
| 1 | `reserved-word` | `'<word>' is a reserved word and can't be used as a name` |
| 1 | `unknown-unit` | `unknown unit '<unit>' (expected one of mm, cm, m, in, ft, deg)` |
| 1 | `unclosed-bracket` | `this "<bracket>" is never closed` |
| 1 | `missing-brace` | `<the body of part 'side_l'> is missing its closing "}"` |
| 2 | `unknown-name` | `unknown name '<name>'` |
| 2 | `duplicate-name` | `'<name>' is declared twice` |
| 2 | `ambiguous-name` | `'<name>' could be <this> or <that> here` |
| 2 | `unit-mismatch` | `can't <add> <a length> and <an angle>`, or `'<field>' needs <a length>, found <a count>` |
| 2, 4 | `type-mismatch` | `'<field>' needs <a part>, found <a region>` (for a param kind `'<locked>' needs a length, an angle, a ratio or a count, found <a stock>`; for shares `'shares' needs <3> numbers, one for each piece of <drawer_bay>, found <2>` or `'shares' needs a fixed number of pieces, found a count that can change`), `split takes 'at:' or 'shares:', not both`, for a cut's shape `<joint>: the outline crosses itself between mark <n> and mark <n + 1>`, or for a kind chosen through a param that doesn't take what its declaration hands it `'<kind>' can't be the <joinery> of <Shelf>` (`'<kind>' can't be the project's <joinery>`), its reason the first note (see [Kind values](#kind-values)) |
| 2 | `unknown-field` | `<a part> has no field '<name>'` |
| 2 | `missing-field` | `<what> is missing '<name>'` (`solved param 'overall_width' is missing 'within'`); a warning for a stock with no `max_length`: `<stock> doesn't say how long its <boards> are, so Stave can't check part lengths` |
| 2 | `out-of-range` | `'<param>' must be within <domain>, found <value>`, or for a size that must be positive `'<up>' must be more than <0mm>, found <value>` (`'shares' must each be more than 0, found <value>`) |
| 2 | `not-in-library` | `a library can't contain a <part> at the top level` |
| 2 | `import-cycle` | `modules import each other: <a> -> <b> -> <a>` |
| 2 | `unknown-module` | `can't find module '<source>'` |
| 2, 4 | `not-supported` | `<what> is not supported yet` (an error for remote imports and for a format version other than 1, a warning for a cut on a round or turned part other than a hole or a round tenon) |
| 2 | `unnamed-cut` | `this cut has no name, so messages will call it "a <slot>"` (a warning, once for each unnamed `cut` in a kind; see [Named cuts](#named-cuts)) |
| 3 | `ground` | `nothing is the root: no part is placed with 'ground' and no region is made with 'box'`, `'<a>' and '<b>' are both placed with 'ground'`, `'<a>' and '<b>' are both made with 'box'`, `'<part>' is placed with 'ground', but '<region>' is made with 'box'; only one can be the root`, `'<a>' is placed with 'ground' inside a component`, `'<a>' is made with 'box' inside a component`, or `box(across:, up:, deep:) can only be the whole value of a region`; see [Placement rules](#placement-rules-checked) |
| 3 | `axis-not-set` | `<item>: position along '<axis>' is not set` |
| 3 | `axis-set-twice` | `<item>: position along '<axis>' is set twice` |
| 3 | `placement-cycle` | `<a> and <b> are placed against each other` |
| 3 | `fit-undecided` | `<part>: nothing decides its <length>` |
| 3 | `mirror` | `<item> has no counterpart on the other side of the mirror` |
| 3 | `mechanism` | `<kind>'s mechanism has <n> freedoms; it needs exactly one`; `<kind>'s mechanism: this mechanism can't be solved one step at a time from its drive; left over: ...` |
| 4 | `no-contact` | `<joint or hardware>: <part> meets none of the parts it should join`, or for `contact(a, b)` `<a> and <b> don't touch` |
| 4 | `overlap` | `<a> and <b> overlap` |
| 4 | `adjust` | `<hardware>: the <member> would sit <offset> off its place on the <member>; it adjusts <d>`, or for an axis no `adjust` covers `<hardware>: the <member> would sit <offset> off its place <across>; it doesn't adjust that way` |
| 4 | `wall` | `<joint>: its <hole> in <part> breaks out of the <left end> of <part>`, `<joint>: its <bore> in <part> runs into the <bore> of <other>` (`meets` when they only touch), `<joint>: its <dado> in <part> crosses the splice between <part>.piece[1] and <part>.piece[2]`, or `<joint>: its <housing> for <other> meets the <housing> for <other> and cuts <part> in two` (`goes right across <part> and cuts it in two` for one cut); see [Cuts stay in their part](#cuts-stay-in-their-part) |
| 4 | `cut-size` | `<joint>: its <tenon> on <part> is <25mm> <thick>, but <part> is only <19mm> <thick>`, `... is <80mm> across, but <part> is only <70mm> across at that end`, or `<joint>: its <groove> in <part> is <12mm> deep, but <part> is only <12mm> <thick> there` |
| 5 | `motion` | `<a> hits <b> when <assembly> is <open>`; `<assembly>'s hardware doesn't move alike`; `<hardware>: <members> lock at <value>`; `<hardware>: <members> may come within 0.1mm of locking near <value>` |
| 6 | `require` | `'<name>' fails: <message>` |
| 6 | `require-range` | `'<name>' does not hold for every allowed value: <message>` |
| 6 | `range` | `<what> stops working when <param> <is below> <value>` |
| 7 | `order` | `<part>: there is no valid order for its cuts`, or for a joint's insert steps `<joint>: <step> is blocked by <part>` |

Warnings and notes: `shadows-prelude`, `no-recipe`, `not-supported`,
`model-mismatch`, `unnamed-cut`, `loose-part`.

A stock with no `max_length` is the one `missing-field` that is a warning
(see [`stock`](#stock-name--)): the design can still be checked and built.

A model mismatch (layer 4) is reported at the kind's
`model(...)` and says one of `<kind>'s model puts '<node>' at <point>, but
the kind says <point>`, `<kind>'s model has '<node>' over <bounds>, but
the kind says <bounds>`, `<kind>'s model has no node <names>`, `<kind>'s
model is not the pinned file: its SHA-256 is <hex>` or `can't read
<kind>'s model: <reason>` (see [Hardware](#hardware)).

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
diagnostic, and the line comes off when it does. A design that needs
language the parser doesn't read yet
can't check clean, so it waits in `tests/broken-next/`, in the same
format with its `// pending:` line, and moves to `tests/broken/` with the
change that builds its language. `docs/test-coverage.md` maps
every rule of this document and every code above to the tests that pin it
down.

### Fixes

A diagnostic's help may carry **fixes**: [patches](#patches) that make the
problem go away, which a tool can apply as they are. "Use 18mm" is a patch
that sets the door's overlay; "tighten the range" is a patch that replaces
the param. Each fix is printed on its own line after the help, as
`= fix: ` and then one patch written on one line (the operations separated
by commas, as any block allows):

```
error[require]: 'thirds' fails: the slot is a third of the post's thickness
  --> project.stave:44:14
   = note: for rail and post in corner
   = note: the rule measures 18mm and needs 24mm
   = help: use slot: 8mm, from the kind's list
   = fix: patch "Use slot: 8mm" { set corner.slot = 8mm }

error[require-range]: 'hanger_room' does not hold for every allowed value: room for coat hangers
  --> cabinet.stave:27:1
   = it fails when cabinet_depth = 450mm
   = help: tighten the range: param cabinet_depth = 560mm within 546mm..650mm
   = fix: patch "Tighten the range" { replace param cabinet_depth = 560mm within 546mm..650mm }
```

- The text after `= fix: ` is exactly a `patch` of the [patch grammar](#syntactic-grammar),
  so a test or a tool parses it with the patch parser and matches on it as
  it matches on codes and messages. Its string is the label a tool shows
  on the fix's button: a short instruction with a capital letter and no
  full stop ("Use 18mm", "Remove 'locked'").
- Fixes come in the order the help names them, the one the help
  recommends first. Every alternative the help offers as a value (each
  value of a kind's list that makes a rule hold) is its own fix.
- A fix is offered only when the checker has seen that it removes this
  diagnostic: the list search of [Checks and `require`](#checks-and-require)
  tries each value, a range is tightened to the values that held. It is
  not promised to make the whole design check: applying one is checked
  again like any patch, and the editor refuses it, as any edit, if it
  leaves an error (see [`docs/editing.md`](editing.md)).
- **A diagnostic that can't offer a patch still explains itself in
  words.** The help line is always there; fixes are added to it, never in
  place of it. A help that asks for a judgement ("move or resize top so
  it covers the whole end") has no fix.
- Fixes are patches, so they edit what parsed: from layer 2 on. A syntax
  error's help stays a text edit, offered as a quick fix (see
  [Tooling and editors](#tooling-and-editors)).
- A design in the broken corpus may pin its first fix with a line
  `// fix: patch "..." { ... }` right after its `// expect:` line (or after
  its `// pending:` line); the corpus test then requires the diagnostic's
  first fix to be exactly that patch.

The checks that offer fixes so far:

| Diagnostic | Fix |
|---|---|
| `out-of-range` for a param's `within` | sets the value to the nearest allowed one, where it is written: the param's default, the assembly's argument, or the setting in the joint's or hardware's kind call (`patch "Use 300mm" { set runner.length = 300mm }`) |
| `motion`, a hinged front that turns into something as it starts to open | sets the overlay on the edge between them to what clears, as an exact length: in the assembly's `Mount` argument, or in the one `cover(..., overlay(...))` region of its component (`patch "Use left: 17mm" { set door.mount = overlay(left: 17mm, right: 17.8mm) }`); for two mirrored fronts that meet, `meeting:` |
| `motion`, a hinged front that hits something | opens it no further than just short of where it hits, a tenth of a degree, when the param its hardware's kind turns to allows that value (`patch "Open no further than 106.8deg" { set Door.hinges.opening_angle = 106.8deg }`) |

A fix edits only the project's own module: a value written in a library
gets no fix. A `solved param` with no `within` gets none either: there is
no right range to guess.

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
| Fast | 1 syntax, 2 types and names, 3 placement and sizes, and the overlaps of the parts' boxes at rest | on every change, within one keystroke; in the editor, on every move of the pointer |
| Slow | 4 fit, 5 motion, 6 requires and ranges (z3), 7 buildability | in the background after typing pauses; cancelled and restarted by the next change |

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
| Quick fixes | each diagnostic's [fixes](#fixes), as workspace edits made by applying their patches; for a syntax error, its `help:` line as a text edit when it is mechanical ("add `as hw`", "write `name[*]`", "add `)`") |
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
  role, station, named shape, `stop` or `couple`; a declaration's `:` has a
  space on each side (`part side_l : oak_19`);
- consecutive lines that each start with a field (`stop:` included), a
  named shape (`shape knuckle:`), a named argument or a
  profile station, in the same block or argument list, line their values up
  one space after the longest key; a blank line or another kind of entry
  ends the run;
- consecutive one-line items with the same keyword in the same scope (a
  file, a component or a kind body), with no blank, comment or doc comment
  line between them, pad their names so that the token after the name lines
  up one space after the longest: the `=` or `:` of params, the `:` of
  joints, kinds and requires, the `{` of stocks, the `=` of regions. A name
  includes its array labels (`bay[drawers, doors]`). Nothing after that
  token is aligned (not `within`), except trailing comments;
- in such a run of params, a param's keyword is `param` whatever comes
  before it, and the word `param` is aligned across prefixes (`locked`,
  `solved`, `private` and `private locked` or `private solved`): each line
  is padded with spaces before its prefix, or before `param` when it has
  none, so that every `param` starts in the column of the longest prefix's,
  and names and `=` signs still line up:

  ```
  locked param reveal        = 1.5mm
         param overall_depth = 560mm within 550mm..650mm
  solved param overall_width = 900mm within 600mm..1200mm
  ```

- a comment at the end of a line keeps its column when there is room;
- at most one blank line in a row, none at the start or end of a block or
  of the file.

---

## Smart edits

The mouse tools in the app edit the design through relationships, and
every edit they make is an ordinary [patch](#patches). How the editor
presents them is [`docs/editing.md`](editing.md); this section is what a
drag may change.

A **resize** drag on a part, a region or a grip asks: which values, if
changed, give this new size? Every size is an expression that leads back
to params and numbers: dragging the top drawer 50mm wider traces its box
width through `opening.across` to `cabinet_width`. The trace follows
`let`s, defaults, region functions and the params an assembly passes to its
component, and collects the **candidates**: the params the size depends
on. It stops at

- a `locked param`: the chain stops there, and it is never a candidate;
- a stock's fields (its thickness) and a kind's sizes (a slide's
  clearance, a hinge's cup): what you buy is not changed by a drag;
- a `let`: it is followed to its value, but a number written in it is not a
  knob and is never a candidate.

What happens next depends on how the size is written and on the kinds of
its candidates (see [Param kinds](#param-name--type--default-within-domain-with-locked-or-solved-before-it)):

1. **A plain number is edited in place.** If the size is written as a
   number (`width: 400mm`, a share in `shares: [140, 180, 220]`), the drag
   changes that number and nothing else follows.
2. **Plain params are the user's to pick.** A plain param is changed only
   when it is the single candidate, or when the user picks it from the
   list of candidates. Its new value is worked out with exact arithmetic,
   without z3: with every other value fixed, the size is `a × p + b` in the
   param `p`, with `a` and `b` exact fractions found by evaluating the size;
   the drag sets `p = (size − b) / a`. When the size is not of that form
   in `p` (`p` times `p`, a division by `p`, a `fit` that `p` decides) or
   `a` is zero, `p` is not offered.
3. **Solved params are picked automatically**, by z3. Its only unknowns are
   the solved params the size depends on; every other value is fixed. The
   target is an equality (the size equals the dragged size), and every
   `require` and every `within` is a constraint. Among the answers it
   minimises, in this order:
   1. how many params change;
   2. how far they move, each measured as a fraction of its own `within`
      domain (from its lowest to its highest value), summed;
   3. how far they are from the thing dragged: a param of the component
      the dragged part is in counts 0, of the component around that 1, and
      so on out to the project's params, summed over the params that
      change; so a component's own solved param goes before the project's;
   4. the order the params are written in (by file, then position), so
      that no two answers tie.

   The ordering is total, so the answer is the same on every run. When the
   size depends on plain params as well, they are listed beside the solved
   answer for the user to pick instead. When there is no answer, the drag
   is refused and says which `require` or `within` stopped it.

Each answer is applied to a scratch copy and checked like any patch; the
editor commits it only if it checks. The result is an ordinary patch
(`set cabinet_width = 950mm`, `set drawer_bay.shares = [140, 200, 200]`).

**Move** drags edit the measured offset of one placement term (its `gap:`
or `inset:`), and **snap** replaces a term with `flush` or `against` on the
face the part was dropped on.

What counts as linear. With every other value held, the trace takes the
choices the solved design made (the side a part stands on, which way a
`spaced` row runs, which piece of a split a region is) as they are. A
`fit` is linear when exactly one part it is joined into can end it on
each side it is measured to; when several can, the nearest decides it and
it is not. `between` and `extent` measure whichever end is nearer or
farther, so they are not linear either. Linearity is decided for each
param on its own: `width * factor + extra` is not linear as a whole, but
with the others held it is `a × p + b` in each of the three, so each is
offered; only a param multiplied by itself, divided by, or deciding a `fit`
or a measure is not. The trace and rule 2's inversion
are built (`stave_solve::trace`, `stave_model::trace::invert`); the
editor does not use them yet.

Rule 3 is built headless (`stave_app::solve_edit`): a solve from a
target to a patch, snapped to the project's `resolution:` and judged by
the scratch check, and a drag that works the values out between solves by
exact arithmetic. The solve's constraints are the `require`s whose two
sides are linear in the solved params; a `require` that is not, a kind's
`require` and the geometric checks are applied to its answer by the
scratch check, which refuses it if one fails. A solved param of one
element of an array, or of an assembly inside a component, is held: no
single `set` reaches it alone. The editor's grips do not use rule 3 yet;
until they do, a `solved param` is edited there as a plain one.

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
| `Ratio` | `0.5`, `2 / 3`, `1 in 8` |
| `Count` | `4` |
| `Bool` | `true`, `false`, comparisons, `edge_to_edge(a, b)` |
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
| `Boards` | `joined(by: ...)` |
| `Splices` | `spliced(by: ..., at: ..., from: ...)` |
| `Envelope` | `box(...)`, `cylinder(...)` |
| `Model` | `model(...)` |
| `Spacing` | `evenly`, `ends`, `ends(inset: d)`, `pitch(d)`, `gap(d)` |
| `Patch` | `contact(a, b)`: where two parts touch (not a patch file, see [Patches](#patches)) |
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
`contact(a, b)` is a smallest and a largest of placed faces, which z3 can
check as a split into cases; until this layer is built, a rule that reads
it is checked at the current values like every other `require`.

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

What the mouse writes (see [`docs/editing.md`](editing.md)) is made of
these same operations:

```
patch "Add a drawer" {
  add region drawer_bay[bottom, top] = split(bay, up, shares: [1, 1])
  add assembly drawer[bottom, top] : casework.Drawer {
    opening: drawer_bay[same]
    slide:   side_mount_slide(length: 450mm)
  }
}
patch "Hold this" {
  add require slat_to_apron_front_min : between(slat[0], apron_front) >= 12mm "room for a finger"
}
patch "Name this size" {
  add param shelf_gap = 300mm
  set shelf.place.up = against(divider.top, gap: shelf_gap)
}
patch "Move the apron back" {
  set apron_front.setback = 5mm
}
```

What a `set` or `unset` path can address:

| Path | Addresses |
|---|---|
| `cabinet_width` | the default of a `param`, or the value of a `let` |
| `shelf.width`, `oak_19.thickness`, `all_open.explode` | a field of an item |
| `shelf_dados.depth`, `shelf_dados.into` | a setting or role in a joint's or hardware's kind call |
| `drawer.pull_kind`, `apron_front.setback` | an argument of an assembly (a component param), added when it isn't written yet |
| `drawer_bay.shares`, `envelope.across` | a named argument of the call that makes a region (`split`, `box`, `opening`, `cover`) |
| `side_l.place.up`, `.place.across`, `.place.deep` | the placement term that fixes that axis (each axis has exactly one). A term that fixes two axes (`corners`) is replaced as a whole |
| `side_l.place.orient` | the `orient` term |
| `casework.Drawer.box_height` | the same, inside a component of a local module |

The elements of an array share one declaration, so a path through one
element (`drawer[top].slide`) is refused: the patch is not applied, and the
reason says to set the value on the array (`drawer.slide`), which changes it
for every element. `drawer[*].slide` means the same as `drawer.slide`.

A patch may edit modules that are local folders. A module from a remote source
is read-only (`error[not-supported]`). After a patch is applied, the whole
design is checked again; a patch that introduces an error is rejected with
that error.
