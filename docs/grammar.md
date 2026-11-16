# The `.stave` language

Version 1, draft 2.

A `.stave` file describes a woodworking project as **intent**: parameters,
stock, parts, joints, hardware and how parts relate to each other. Everything
derived (positions, meshes, cut list, build manual) is computed and never stored.

Design rules:

1. **Every length carries a unit.** `19mm`, `3/4in`, `1-1/2in`. A bare number is
   a count or a ratio, never a length.
2. **No coordinates, ever.** There is no point, vector or tuple literal. Parts
   are positioned only by relationships to named anchors (faces, mid-planes,
   joint housings), optionally offset by real measurements. See [Placement](#placement).
3. **Everything has a name and is referenced by it**: parts, joints, hardware,
   regions, checks. Nothing is identified by its position in a list.
   Names from another module are always qualified: `casework.Drawer`.
4. **One fact, once.** A measurement is written in one place; everything that
   depends on it refers to it. See [Sizes](#sizes).
5. **One construct per job.** Declarations use `keyword name : kind { fields }`;
   everything inside an expression is a call with named arguments. Lists are
   always `[ ]`.
6. **A newline ends a field**, so an error is reported on the line that has it.
   See [Newlines](#newlines).
7. **A folder is a module.** Every `.stave` file in a folder shares its names;
   other folders are imported. See [Modules](#modules).

---

## Lexical grammar

```ebnf
(* Spaces, tabs and comments separate tokens. Newlines are tokens; see below. *)
line_comment  = "//" , ? any char except LF ? ;          (* not "///" *)
block_comment = "/*" , ? any chars ? , "*/" ;            (* does not nest *)
doc_comment   = "///" , ? any char except LF ? ;         (* attaches to next item *)
NL            = LF ;                                     (* see "Newlines" *)

IDENT         = ident_start , { ident_char } ;           (* minus KEYWORDS *)
ident_start   = "A".."Z" | "a".."z" | "_" ;
ident_char    = ident_start | DIGIT ;

STRING        = '"' , { str_char | escape } , '"' ;
escape        = "\\" , ( '"' | "\\" | "n" | "t" | "u{" , HEX , { HEX } , "}" ) ;

INT           = DIGIT , { DIGIT | "_" } ;
DECIMAL       = INT , "." , DIGIT , { DIGIT } ;

(* A quantity is ONE token: no whitespace inside it. *)
QUANTITY      = magnitude , unit ;                       (* must not be followed by ident_char *)
magnitude     = INT , "-" , INT , "/" , INT              (* mixed fraction: 1-1/2in *)
              | INT , "/" , INT                          (* fraction: 3/4in *)
              | DECIMAL                                  (* 18.5mm *)
              | INT ;                                    (* 19mm *)
unit          = "mm" | "cm" | "m" | "in" | "ft" | "deg" ;

NUMBER        = DECIMAL | INT ;                          (* unitless: counts, ratios *)
```

**Keywords** (reserved, cannot be identifiers):

```
stave project library import private kind recipe param let region stock part
joint hardware assembly profile component scene require patch add set remove
place pose at within locked fit true false
```

Face and direction names (`top`, `left`, `front`, `up`, ...) are **not**
keywords, so a part may be called `top` and an array may use them as labels.

`as` and `in` are **contextual**: they are special only in an import line
(`import "./hardware" as hw`) and after a patch's `add` (`add part x ... in
"carcass.stave"`). Everywhere else they are ordinary names, so named arguments
such as `in: opening` keep working. Likewise `params`, `roles`, `cut`, `steps`
and `makes` are special only at the start of an entry in a `kind` body, and
`mark`, `cut` and `check` only at the start of an entry in a `recipe` body.

### Quantities

The lexer tries the longest `QUANTITY` first and falls back if no unit follows:

| Source | Tokens | Meaning |
|---|---|---|
| `3/4in` | `QUANTITY(3/4 in)` | three-quarters of an inch |
| `3/4` | `INT(3)` `/` `INT(4)` | the ratio 0.75 |
| `1-1/2in` | `QUANTITY(1 1/2 in)` | one and a half inches |
| `1 - 1/2in` | `INT(1)` `-` `QUANTITY(1/2 in)` | type error: a count minus a length |
| `2in-1/2in` | `QUANTITY(2in)` `-` `QUANTITY(1/2in)` | 1-1/2in |
| `3inch` | error | `unknown unit 'inch' (expected one of mm, cm, m, in, ft, deg)` |

### Newlines

A newline is a token only where it can end something. The lexer drops a newline:

- inside `( )` and `[ ]`, at any depth;
- after a token that can't end an expression: a binary operator, `,`, `:`, `=`,
  `(`, `[`, `{`, `..`, or a keyword that expects more;
- before `}` and before another newline.

So a long expression continues onto the next line when the line ends with an
operator or an open bracket, and a field always ends at the end of its line:

```
length: cabinet_height
        - base.thickness          // error: line starts with an operator; end the
                                  // previous line with "-" instead
length: cabinet_height -
        base.thickness            // fine
```

A missing value is reported on its own line: `length:` followed by a newline is
`error: 'length' has no value`, not a confusing error on the next field.

---

## Syntactic grammar

```ebnf
(* ---------- Files ---------- *)
file          = header , { NL } , { import , NL , { NL } } , { item , { NL } } ;
patch_file    = header , { NL } , { patch , { NL } } ;
header        = "stave" , INT , NL ;
import        = "import" , STRING , [ "as" , IDENT ] ;  (* applies to this file only *)

(* ---------- Items ---------- *)
item          = { doc_comment } , [ "private" ] , item_kind ;
item_kind     = project | library | kind | recipe | param | let | region | stock
              | part | joint | hardware | assembly | profile | component | scene
              | require ;

project       = "project" , STRING , body ;
library       = "library" , STRING , [ body ] ;
kind          = "kind" , IDENT , ":" , IDENT , kind_body ;
                (* kind side_mount_500 : slide { }   or   kind ari_tsugi : joint { } *)
kind_body     = "{" , { kind_entry , sep } , "}" ;
kind_entry    = field                                    (* insert: ..., adds: ..., envelope: ... *)
              | "params" , "{" , { param_decl , sep } , "}"
              | "roles" , "{" , { role_decl , sep } , "}"
              | "cut" , expr , "{" , { expr , sep } , "}"   (* cut end(part, facing: into) { tenon(...) } *)
              | "steps" , "{" , { STRING , sep } , "}"
              | "makes" , part                           (* a part the joint creates: a key, a wedge *)
              | require ;
role_decl     = IDENT , ":" , ( IDENT | "[" , IDENT , { "," , IDENT } , "]" ) ;

recipe        = "recipe" , IDENT , ":" , IDENT , recipe_body ;
                (* recipe notch_by_hand : notch { }   — how to make one cut operation *)
recipe_body   = "{" , { recipe_entry , sep } , "}" ;
recipe_entry  = field                                    (* tools: [...], needs: [...], prefer: ... *)
              | "mark" , "{" , { expr , sep } , "}"      (* marking-out steps *)
              | "cut" , "{" , { expr , sep } , "}"       (* cutting steps *)
              | "check" , STRING ;
                (* part: Part   into: Parts   into: [Part, Part] *)

param         = "param" , IDENT , "=" , expr , [ "within" , domain ] , [ "locked" ] ;
domain        = expr , ".." , expr                       (* continuous: 600mm..1200mm *)
              | list ;                                   (* one of: [350mm, 400mm, 450mm] *)
let           = "let" , IDENT , "=" , expr ;

region        = "region" , IDENT , [ array_spec ] , "=" , expr ;
stock         = "stock" , IDENT , body ;

part          = "part" , IDENT , [ array_spec ] ,
                ( ":" , path , body                      (* cut from stock *)
                | "=" , expr ) ;                         (* derived: mirror(...) *)

joint         = "joint" , IDENT , [ array_spec ] , ":" , call , [ body ] ;
hardware      = "hardware" , IDENT , [ array_spec ] , ":" , call , [ body ] ;
                (* the call names the kind and its parts by role:
                   joint back_rabbets : rabbet(back, into: [side_l, side_r]) *)

assembly      = "assembly" , IDENT , [ array_spec ] ,
                ( ":" , path , body                      (* instance of a component: casework.Drawer *)
                | "=" , expr ) ;                         (* derived: mirror(...) *)

profile       = "profile" , IDENT , "{" , { profile_entry , sep } , "}" ;
profile_entry = "at" , expr , ":" , expr                 (* station: value *)
              | field ;

component     = "component" , IDENT ,
                "(" , [ param_decl , { "," , param_decl } , [ "," ] ] , ")" ,
                "{" , { NL } , { item , { NL } } , "}" ;
param_decl    = IDENT , ":" , IDENT , [ "=" , expr ] ;   (* type is a plain name *)

scene         = "scene" , IDENT , [ STRING ] , "{" , { scene_entry , sep } , "}" ;
scene_entry   = "pose" , path , "=" , expr               (* pose door_l = open *)
              | field ;                                  (* show: drawer[top] *)

require       = "require" , IDENT , ":" , expr , STRING ;

array_spec    = "[" , ( IDENT , "," , IDENT , { "," , IDENT } , [ "," ]
                                                         (* two or more names: labels *)
                    | expr ) , "]" ;                     (* otherwise: a count *)

(* ---------- Bodies ---------- *)
body          = "{" , { body_entry , sep } , "}" ;
body_entry    = field | place ;
field         = IDENT , ":" , expr ;                     (* exactly one expression *)
place         = "place" , "{" , { expr , sep } , "}" ;   (* one placement term per line *)
sep           = NL | "," | ";" ;                         (* repeated separators are fine *)

path          = IDENT , { "." , IDENT | "[" , index , "]" } ;
index         = "*" | expr ;

(* ---------- Expressions (lowest to highest precedence) ---------- *)
expr          = or_expr ;
or_expr       = and_expr , { "||" , and_expr } ;
and_expr      = cmp_expr , { "&&" , cmp_expr } ;
cmp_expr      = add_expr , [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) , add_expr ] ;
add_expr      = mul_expr , { ( "+" | "-" ) , mul_expr } ;
mul_expr      = unary , { ( "*" | "/" ) , unary } ;
unary         = ( "-" | "!" ) , unary | postfix ;
postfix       = primary , { "." , IDENT | "[" , index , "]" | "(" , [ args ] , ")" } ;
call          = path , "(" , [ args ] , ")" ;
args          = arg , { "," , arg } , [ "," ] ;
arg           = [ IDENT , ":" ] , expr ;                 (* positional or named *)
list          = "[" , [ expr , { "," , expr } , [ "," ] ] , "]" ;

primary       = QUANTITY | NUMBER | STRING | "true" | "false" | "fit"
              | IDENT | list | "(" , expr , ")" ;

(* ---------- Patches (AI and suggestion output) ---------- *)
patch         = "patch" , STRING , "{" , { patch_op , sep } , "}" ;
patch_op      = "add" , item , [ "in" , STRING ]          (* target file within the module *)
              | "set" , path , "=" , expr
              | "remove" , item_tag , IDENT ;
item_tag      = "kind" | "recipe" | "part" | "stock" | "param" | "let" | "region" | "joint"
              | "hardware" | "assembly" | "profile" | "component" | "scene" | "require" ;
```

There are no object literals. `Name { ... }` appears only after a declaration
header (`part x : stock { }`, `assembly door_l : FramePanelDoor { }`). Inside an
expression, everything is a call: `euro_hinge(opening_angle: 110deg)`.

---

## Names and arrays

- Item names are unique within their module (every file in the folder), or
  within a component body. A duplicate is an error that names both files.
- `part shelf[3]` declares a counted array: `shelf[0]`, `shelf[1]`, `shelf[2]`.
- `assembly drawer[bottom, top] : Drawer { ... }` declares a labelled array:
  `drawer[bottom]`, `drawer[top]`. Labels are in declaration order, so
  `drawer[0]` is still `drawer[bottom]`. `[*]` means every element.
- Two or more names in a declaration are labels; anything else is a count. A
  single label isn't allowed (just name the part).
- In an index, a bare name that matches a label of that array is the label. A
  `param` or `let` with the same name as a label is an error.
- Brackets keep labels apart from anchors: `drawer[top]` is the top drawer,
  `drawer[top].top` is that drawer's top face.
- **Elementwise `[*]`.** Inside an array declaration, `other[*]` means the
  element of `other` with the same label (or the same index, for counted
  arrays) as the element being declared. The two arrays must have the same
  labels, in the same order.

  ```
  region drawer_bay[bottom, top] = split(bay[drawers], up)
  assembly drawer[bottom, top] : Drawer {
    opening: drawer_bay[*]        // drawer[bottom] gets drawer_bay[bottom], and so on
  }
  ```

- Array counts are constants, or params with a `within` list, so the checker
  can try every count.

---

## Modules

**A folder is a module.** Every `.stave` file directly inside a folder belongs to
that folder's module and shares its names. File names and order don't matter:
splitting or merging files never changes meaning. Subfolders are separate
modules.

Each module has exactly one file with a `project` item (a project module) or a
`library` item (a library module).

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
  hardware/                // library module, used as hardware.*
    library.stave          //   library "Shop hardware"
    handles.stave
    models/
      brass_knob_26.glb
```

### Imports

```
import "std:casework"
import "./hardware"
import "github:some-maker/stave-catalog/slides@v2.1.0" as slides

assembly drawer[bottom, top] : casework.Drawer {
  opening: drawer_bay[*]
  slide:   slides.side_mount_500
}
```

- You import a **folder**, never single names. Everything in it is used through
  the module's name: `casework.Drawer`, `hardware.brass_knob_26`. There is no
  wildcard or unqualified import, so a name's origin is visible at every use.
- The module name is the folder's last segment, or the name after `as`.
- An import applies to the file it is written in, so each file shows what it
  depends on.
- A module name can't match an item name in the same module, so `side_l.top`
  (a face) and `casework.Drawer` (an item of a module) never collide.
- **The prelude.** A small, fixed set of standard names is available in every
  file without an import or a prefix: the joints of `std:joinery` (`dado`,
  `rabbet`, `groove`, `dowel`, `mortise_tenon`, `round_tenon`, `half_lap`,
  `socket`, `screw`, `cope_and_stick`, ...), the base hardware kinds, and the
  built-in functions. Beginners write `dado(...)`, not `joinery.dado(...)`.
  Anything from outside `std` is always qualified (`japanese.ari_tsugi`), so a
  joint from someone else is visible as such. The prelude changes only with the
  format version in the `stave` header, so a local item can't take a prelude
  name: that's an error.
- Imports between modules can't form a cycle. A cycle is an error that lists it.

### Sources

| Source | Example | Notes |
|---|---|---|
| relative path | `"./hardware"`, `"../shared/joinery"` | relative to the importing file |
| standard library | `"std:casework"` | ships with Stave, versioned with it |
| GitHub | `"github:owner/repo/path@v2.1.0"` | a tag or a commit; `@main` is a warning and `stave build` refuses it |
| any URL | `"https://example.com/stave/stock#sha256=9f2c..."` | must carry a content hash; may point at a folder archive or a single file (a module of one file) |

- **`stave.lock`**, one per project, records every remote module's resolved
  commit and content hash, including modules that remote modules import. The
  same project builds the same way on every machine.
- **Fetching happens only on request:** `stave fetch`, or a confirm button in
  the app. Checking never touches the network; it reads the local cache.
  `stave vendor` copies remote modules into the project for offline use.
- **Imports can't run code.** `.stave` is declarative, so a module is data.
- **An AI patch can't add a remote import or model without confirmation**,
  because it fetches from the internet and changes what the project depends on.

### What a library may contain

Libraries provide reusable declarations: `component`, `stock`, `profile`,
`kind`, `recipe`, `let` and `require`. They can't contain `part`, `assembly`, `region`,
`joint`, `hardware` or `scene` at the top level, because those need placing in a
project. (Inside a component body they're fine.)

Items are public by default. Mark helpers `private` to keep them out of other
modules and out of autocomplete:

```
private let groove_offset = 8mm
```

### Placement stays inside a module

A part can only be placed against parts of its own module, or of a component it
instantiates. Projects grow by components, not by cross-folder references: a
kitchen is a project that places `cabinets.Base600` and `cabinets.Wall400`, each
a self-contained component in a `cabinets/` module.

---

## Placement

`place { }` is the **only** way to position a part, an assembly element or a
piece of hardware. It holds one placement term per line. There is no syntax
for coordinates.

### Directions, axes and faces

| Axis | Directions | Meaning |
|---|---|---|
| `up` | `up`, `down` | floor to ceiling |
| `across` | `left`, `right` | left to right, seen from the front |
| `deep` | `front`, `back` | front to back |

Each part has six faces named by the world direction they face once placed:
`top`, `bottom`, `left`, `right`, `front`, `back`.

### Anchors

| Anchor | Example | Meaning |
|---|---|---|
| face | `base.top` | a face of a part, or of a region |
| mid-plane | `mid(base, across)` | the plane halfway between `base.left` and `base.right` |

A joint's housing is not an anchor. It is used only by `inside(...)`.

### Placement terms

| Term | Fixes | Meaning |
|---|---|---|
| `ground` | everything | the root that every other part is placed from. Exactly one per project. The floor is wherever the lowest part ends up. |
| `against(anchor)` | 1 position | this part's opposite face touches the anchor |
| `against(anchor, gap: d)` | 1 position | parallel to the anchor, `d` away on the anchor face's outward side |
| `on(anchor)` | 1 position | `against` for a `.top` anchor only; reads better for "sits on". Any other anchor is an error. |
| `flush(anchor)` | 1 position | this part's same-named face lines up with the anchor |
| `flush(anchor, inset: d)` | 1 position | lines up, then moved inward by `d` |
| `center(part_or_region, axis)` | 1 position | centered on its mid-plane along that axis |
| `center(between: a, b)` | 1 position | centered between two parallel anchors |
| `spaced(spacing, from: a, to: b)` | 1 position per element | arrays: `evenly`, `ends`, `pitch(d)`, `gap(d)`; `ends` takes `inset: d` |
| `corners(part, inset: d)` | 2 positions | array of exactly 4, in the order front_left, front_right, back_left, back_right |
| `inside(joint)` | position and rotation | seated in a joint's housing. Adding `orient` is an error, because the housing already decides it. |
| `orient(length: dir, width: dir)` | rotation | `length: down` means the part's length runs downward from its start (station 0 of a profile is at the top). Thickness takes the remaining axis. Default: `length: right, width: back`, lying flat. |

### Placement rules (checked)

1. Exactly one part is placed with `ground`.
2. Every anchor belongs to something placed earlier in dependency order. Cycles
   are errors.
3. Each axis is fixed by exactly one term. A missing axis is reported with what
   to add: `side_l: position along 'deep' is not set; add flush(...), center(...) or against(...)`.
4. **Joints never position parts**, with two exceptions: a part placed
   `inside(joint)`, and parts a joint creates itself (a key, a wedge), which the
   joint places. Every other joint only checks that the parts it connects
   actually meet.
5. Because of rules 2 to 4, positions are worked out directly in dependency
   order, without a solver, and every error points at one term.

Rake and splay (angled legs) are out of scope for version 1. They will be added
as `orient` options, so they don't change these rules.

### Mirroring

`part side_r = mirror(side_l, about: mid(base, across))` mirrors a part or
assembly through a plane. Mirroring also swaps `left` and `right` in the
mirrored item's arguments, so `overlay(left: 18mm, right: 9mm)` on one door
becomes `overlay(left: 9mm, right: 18mm)` on its mirror image.

---

## Sizes

**The rule: write the size you see; joints add their own engagement.** For a
shelf in dados, write the distance between the sides; the dado depth is added
to the cut length automatically. The cut list always shows the final size.

`fit` means a size is decided by something else, and is written instead of
repeating that fact:

| Where `fit` is used | What decides the size |
|---|---|
| a part placed `inside(joint)` | the housing (a back panel in its rabbets) |
| a part with joints at both ends along that axis | the gap between the two parts it joins (a shelf between two sides) |
| a part placed against faces at both ends | the gap between those faces |
| a turned part's length | its profile |

If none of these applies, or more than one gives a different answer, `fit` is
an error that says which.

A part's thickness comes from its stock. A part may set a thinner
`thickness:` (a key planed down from thicker stock); the cut list notes the
planing.

---

## Regions

A region is a named box of space, used for openings and bays. Regions are
placed like parts and have the same faces and mid-planes, but are never cut.

```
region front_opening = opening(left: side_l.right, right: side_r.left,
                               bottom: base.top, top: top.bottom,
                               face: side_l.front)
region bay[drawers, doors] = split(front_opening, up, at: divider)
region drawer_bay[bottom, top] = split(bay[drawers], up)
```

`split` labels its pieces in the direction of its axis, so for `up` the first
label is the lowest piece.

---

## Joints and hardware

Every joint and piece of hardware has a name, written first. The kind is a
call whose arguments name the parts by role:

```
joint shelf_dados  : dado([divider, shelf], into: [side_l, side_r]) { depth: 9mm }
joint back_rabbets : rabbet(back, into: [side_l, side_r]) { depth: 9mm }
hardware slides[left, right] : side_mount_slide(side[*], in: opening, length: 500mm)
```

- The first positional argument is the part that is inserted, fastened or
  mounted. `into:` (joints) or `on:` / `in:` (hardware) names what it goes
  into. The direction of insertion follows from these roles.
- Settings that describe the joint go in the body; settings that choose the
  kind go in the call.
- Hardware is mounted on the part named by its first argument. The mounting
  face fixes one axis; `place { }` fixes the rest. Hardware with a fixed
  position in its catalog entry (slides, hinge plates) needs no `place`.
- When a role holds several parts (`dado([divider, shelf], into: [side_l,
  side_r])`), the joint applies to every pair that actually meets.

### Defining joints

Joint kinds are not built into the checker. Each one is a `kind ... : joint`
written in `.stave`, usually in a library. The standard joints in `std:joinery`
are written this way too. A definition says everything the checks, the cut list
and the build manual need, so a new joint needs no new code.

```
/// Ari tsugi: a dovetailed end joint that drops in from above.
kind ari_tsugi : joint {
  roles {
    part: Part
    into: Part
  }
  params {
    neck:  Length = part.width / 3
    depth: Length = part.width / 2
    angle: Angle  = 10deg
  }
  require fits : depth <= into.length / 4 "the dovetail is too deep for this beam"

  insert: [slide(part, along: width)]
  adds:   length(part, depth)

  cut end(into, facing: part) {
    dovetail_socket(neck: neck, depth: depth, angle: angle)
  }
  cut end(part, facing: into) {
    dovetail_tail(neck: neck, depth: depth, angle: angle)
  }

  steps {
    "Set a bevel gauge to {angle} and mark the tail on {part}"
    "Saw and chisel the socket in {into}"
    "Drop {part} into {into} from above"
  }
}
```

| Entry | Says | Used by |
|---|---|---|
| `roles { }` | the parts the joint connects. Defaults to `part: Parts` and `into: Parts`; `Parts` is one part or a list, `[Part, Part]` is exactly two | type checking of every use |
| `params { }` | settings, with types and defaults that may refer to the roles | type checking, the inspector, the body of each use |
| `require` | rules for valid use | checks; errors point at the line that uses the joint |
| `insert:` | how the parts come together, as a list of steps | the assembly check, the build manual's order |
| `cut_first:` | which role is cut first; the other is then fitted to it. `cut_first: into` (the default) cuts the mortise or dado first and fits the tenon or shelf to it | the order of operations |
| `adds:` | how much the joint adds to each part's cut size | cut list, `fit` |
| `cut <face> { }` | material removed, from the cut operations below | geometry, collision checks, the 3D view, templates |
| `makes part` | a part the joint creates (a key, a wedge, a pin) | cut list, build manual |
| `steps { }` | build-manual text; `{name}` inserts a param or role, formatted in the project's units | build manual |

**Faces to cut.** A cut is measured on one face of one role's part, from that
face's own edges. These are measurements within a part, like a profile's
stations; world placement stays coordinate-free.

| Selector | The face | Measuring |
|---|---|---|
| `end(p, facing: q)` | the end of `p` nearest `q` | `width` and `thickness` across the end; depth runs along `p`'s length |
| `side(p, normal: width \| thickness, from: q)` | a long face of `p` | `along` from the end nearest `q`, `across` from the face's reference edge; `flip: true` measures `across` from the other edge |
| `crossing(p, q)` | where `p` and `q` overlap | in `p`'s own length, width and thickness |

**Cut operations.** Implemented in `stave-geom`; joint authors combine them.
`through` as a depth means all the way through.

| Operation | On | Removes |
|---|---|---|
| `notch(depth:)` | crossing | the overlap with the other part, to `depth` (dados, rabbets, halvings) |
| `slot(width:, depth:)` | crossing | a slot of `width`, centered in the overlap |
| `pocket(width:, thickness:, depth:)` | end, side | a rectangular mortise, centered unless `offset:` is given |
| `tenon(width:, thickness:, length:)` | end | everything around a centered tenon |
| `drill(diameter:, depth:)` | end, side | a round hole |
| `dovetail_socket(neck:, depth:, angle:)` | end | a dovetail socket |
| `dovetail_tail(neck:, depth:, angle:)` | end | everything around a dovetail tail |
| `outline(marks: [...], depth:)` | side | the shape marked out with `mark(along:, across:)` points, cut to `depth` |

**Assembly steps** (`insert:`). Each step moves one part along or about one of
its own axes (`length`, `width`, `thickness`). The checker sweeps the moving
part's shape through each step and finds which direction is free; if neither
is, it's an error naming the step.

| Step | Meaning |
|---|---|
| `slide(p, along: axis)` | move `p` along its axis until seated |
| `rotate(p, by: angle, about: axis)` | turn `p` in place, e.g. to lock a twist joint |
| `drive(p, along: axis)` | knock in a part the joint makes (a key or wedge) |

`adds: length(p, d)` adds `d` at each end of `p` that the joint connects. A list
adds to several parts: `adds: [length(part, d), length(into, d)]`.

---

## Hardware kinds and 3D models

A `kind` declares a piece of hardware: its catalog data for the checks, and
optionally a 3D model for the viewport.

```
library "Shop hardware"

kind side_mount_500 : slide {
  length:    500mm
  clearance: 12.7mm
  travel:    500mm
  envelope:  box(length: 500mm, height: 45mm, thickness: 12.7mm)
  model:     model("./models/side_mount_500.glb", align: bounds)
  moving:    "drawer_member"          // the node in the model that moves when the slide opens
  license:   "CC-BY-4.0"
}

kind brass_knob_26 : knob {
  envelope: cylinder(diameter: 26mm, length: 30mm)
  model:    model("github:some-maker/stave-catalog/models/knob_26.glb@v1.4.0", align: node("mount"))
}
```

- The base kind after `:` (`slide`, `hinge`, `knob`, `pull`, `fastener`, ...)
  comes from the catalog and decides which fields are required.
- **Models are for looks only.** Fit, collision and the cut list use the
  `envelope`, a box or cylinder with exact sizes. A project checks the same
  offline or before its models are fetched; a missing model is drawn as its
  envelope with a label.
- **Format:** glTF 2.0 binary (`.glb`), with mesh, materials and textures in one
  file. A model may not reference other files or URLs. Size limit 20 MB.
- **Sources:** the same forms as imports. A relative path is relative to the file
  that declares the kind, so a hardware module carries its models with it.
  Remote models are pinned in `stave.lock`.
- **Alignment without coordinates.** glTF's own convention is used: +Y is
  `up`, +Z is `front`, +X is `right`, units are metres.
  - `align: bounds` sets the model's bounding box onto the envelope. If they
    differ by more than 0.5mm, it's a warning (wrong units, or the wrong model).
  - `align: node("mount")` uses a named node in the model as the mounting point,
    for hardware whose shape doesn't fill its envelope.
- `moving:` names the node that moves with the hardware's motion, so a slide or
  hinge animates correctly. The motion itself comes from the catalog data
  (`travel`, hinge pivot).
- `license:` is recorded and credited in the build manual.

---

## Work cards and recipes

Every part gets a **work card**: the steps to turn stock into that part, drawn
on the part itself. Marks go on its faces, waste is hatched, cuts are shown
before and after, and every dimension is measured from the part's reference
face, reference edge or an end. The build manual and the app's shop mode are
made from work cards.

Work cards are generated from the cuts that joints declare. A **recipe** says
how to make one cut operation with a given set of tools.

```
/// A dado or rabbet, cut by hand.
recipe notch_by_hand : notch {
  tools: [marking_knife, square, marking_gauge, tenon_saw, chisel, router_plane]
  mark {
    knife(walls)                       // square a line across the face at each wall
    gauge(depth, on: open_edges)       // gauge the depth on every edge the notch runs out at
    hatch(waste)
  }
  cut {
    saw(walls, to: depth, keep: outside)
    chop(waste, bevel: down)
    pare(floor, with: router_plane, depth: depth)
  }
  check "the mating part pushes in by hand and leaves no gap at the shoulders"
}
```

- **Features.** Each cut operation names the parts of its geometry a recipe can
  refer to: `walls`, `floor`, `waste`, `depth`, `open_edges` for `notch` and
  `slot`; `cheeks`, `shoulders` and `length` for `tenon`; `centre`, `diameter`
  and `depth` for `drill`; and so on. `stave schema` lists them.
- **Marking steps:** `knife(...)`, `pencil(...)`, `gauge(distance, on:)`,
  `square_around(...)`, `centre(...)`, `hatch(...)`, `template(...)` (a 1:1
  printed template for shapes such as dovetails or a marked-out `outline`).
- **Cutting steps:** `saw(...)`, `chop(...)`, `pare(...)`, `plane(...)`,
  `rout(...)`, `drill(...)`, `bore(...)`, `turn(...)`. `keep: outside` puts the
  kerf on the waste side of the line.
- **Choosing a recipe.** The project lists the tools you have:

  ```
  project "Clothing cabinet" {
    tools: [marking_knife, square, marking_gauge, tenon_saw, chisel, router_plane, router, drill]
  }
  ```

  For each cut, Stave picks the first recipe whose `tools` you have, preferring
  the standard library's order unless a recipe sets `prefer:`. A joint can name
  one directly: `joint shelf_dados : dado(...) { depth: 9mm, method: notch_by_router }`.
  A cut with no usable recipe is a warning that lists the missing tools.
- **Needs.** A recipe can say what must still be true of the part when it runs,
  which constrains the order of operations:
  - `flat_face`: the face being cut is still flat (a router or drill press
    rides on it), so this step comes before cuts that remove that face;
  - `square_stock`: the part is still a square blank, so this comes before any
    shaping, such as boring before turning;
  - `before(op)` and `after(op)`: relative to another cut operation on the
    same part.

  ```
  recipe notch_by_router : notch {
    tools: [square, tape, router, straight_bit, clamps]
    needs: [flat_face]
    ...
  }
  ```

- **Reference face and edge.** Each part has a reference face and edge that all
  measurements start from, marked on the card. The default is the face most of
  the joinery is cut into, and the front-facing edge; a part can set them:
  `face_side: right`, `face_edge: front`.
- **Planned like a woodworker.** The work plan:
  - groups steps by **tool setting**, so a marking gauge set to 9mm marks every
    9mm line on every part before it's reset;
  - marks **matched parts together** when their cuts line up (both cabinet
    sides, clamped with bottom ends flush), so shelves come out level;
  - orders cuts so a part never loses the face or edge a later step measures
    from.
- Recipes live in libraries. `std:joinery` ships recipes for its joints, and a
  joint library such as the Japanese samples can ship its own (marking with a
  sumitsubo, for example).

---

## Order of operations

The order of every step, from milling the stock to fitting the hardware, is
worked out by Stave. It is never written in the file: it follows from the
joints, the recipes and the parts' shapes.

**1. Phases.** The build always runs in this order: mill the stock (flatten,
square and cut to size); mark (reference marks first); cut the joinery; dry fit
each sub-assembly; prepare surfaces that are hard to reach once glued; glue up,
sub-assemblies first; fit hardware; finish. Within the glue-up, assemblies are
ordered by their joints' `insert:` steps: a part goes in only once its path is
clear.

**2. Rules that must hold** (each one is an edge in a dependency graph):

- Marking a feature comes before cutting it.
- **Reference faces survive.** A cut never removes a face or edge that a later
  step measures from: everything measured from it is marked first.
- **The hole first, then fit the peg**, as each joint's `cut_first:` says:
  mortise before tenon, dado before trimming the shelf.
- Recipe `needs:` hold (`flat_face`, `square_stock`, `before`, `after`).
- Matched parts are marked in one step.

**3. Among valid orders, the least effort.** Orders are scored and the cheapest
wins:

1. fewest tool-setting changes (gauge, router depth, fence, saw height),
   counted across the whole project, so one setting marks every matching line
   on every part before it changes;
2. fewest tool changes;
3. least flipping and re-clamping of a part;
4. risky cuts early, before much work has gone into the part.

Ties are broken the same way every time, so a small design change doesn't
reshuffle the build manual.

The plan is project-wide. Shop mode can follow it **by setting** (gauge at 9mm:
`side_l`, `side_r`, `divider`) or **by part** (one work card at a time).

**4. Explained and checked.**

- Every step records why it is where it is: "before step 9, because step 9
  removes the front edge that step 7 measures from".
- **If no valid order exists, the design can't be built**, and that is an
  error on the line that causes it:

  ```
  error[order]: drawer[top].bottom can't be fitted
    --> casework/drawer.stave:58:3
     = the bottom slides into its groove from the back, but the back is glued in first
     = help: make the back 6mm shorter so the bottom slides in under it,
             or glue the bottom in before the back
  ```

- You can drag steps into a different order in shop mode. Stave refuses a move
  that breaks a rule and says which one. The chosen order is remembered by the
  app, not stored in the design.

---

## Built-in functions

| Function | Returns | Meaning |
|---|---|---|
| `between(a, b)` | length | clear distance between two parts or faces |
| `extent(items, axis)` | length | overall size of a list of parts along an axis |
| `mid(part_or_region, axis)` | anchor | mid-plane |
| `mirror(item, about: anchor)` | part or assembly | mirror image through a plane |
| `opening(left:, right:, bottom:, top:, face:)` | region | the space bounded by four faces, open at `face` |
| `split(region, axis, at:)` | regions | split by a part (`at: divider`) or into equal pieces (no `at`) |
| `cover(region, mount)` | region | the outline of a door or drawer front covering an opening |
| `overlay(left:, right:, top:, bottom:, meeting:)` | mount | how far a front extends past each edge of its opening; negative is a reveal. `meeting:` replaces the value on any edge that meets a neighbouring front (between two doors, between two drawers), so one mount works for every front in a row |
| `inset(reveal:)` | mount | a front set inside its opening with an even gap |
| `turn(profile)` | shape | turned on a lathe to a profile |
| `movement_gap(panel, along: axis)` | length | seasonal movement allowance for a solid panel, by species and width |
| `evenly`, `ends`, `pitch(d)`, `gap(d)` | spacing | used by `spaced` |
| `box(length:, height:, thickness:)` | envelope | a hardware envelope |
| `cylinder(diameter:, length:)` | envelope | a hardware envelope |
| `model(source, align:)` | model | a `.glb` model for a `kind`; `align: bounds` or `align: node("name")` |

---

## Scenes

A scene is a named view of the same design: motion states and visibility.
Scenes never change the design.

```
scene all_open "All open" {
  pose door_l = open
  pose door_r = open
  pose drawer[top] = open
}

scene drawer_bench "Top drawer on the bench" {
  show: drawer[top]
  explode: true
}
```

- Placement sets the **rest position** (doors closed, drawers shut). Hinges and
  slides define motions from there. `pose` sets `open`, `closed`, an angle or a
  travel length.
- Pose the moving thing, not its hinge: an assembly with exactly one motion is
  posed directly. Name the hardware only when there are several.
- Fields: `show:` and `hide:` (one item or a list), `explode:` (bool),
  `section:` (cut plane, by anchor).
- Every scene is collision checked, and every motion is checked through its full
  range, not just at the positions scenes use.

---

## Checks and `require`

`require` states something about the design that must stay true. It is checked
like every other rule and reported as an error on its own line:

```
require hanger_room : cabinet_depth - back.thickness >= 550mm "room for coat hangers"
```

Requires are what an AI patch or a smart edit is judged against: a change that
breaks one is rejected, with the message.

Checking runs in layers:

1. **Syntax**: grammar.
2. **Types**: names resolve, units match, fields and roles exist.
3. **Placement and sizes**: every axis set once, `fit` resolvable, no cycles.
   Worked out directly, in dependency order.
4. **Fit and geometry**: parts meet where joints say they do and overlap nowhere
   else; drawers fit openings with slide clearance; panels have a movement gap.
5. **Motion**: every scene and every motion's full range is collision free.
6. **Requires and ranges** (z3): every `require` holds, and layers 3 and 4 hold
   for every value inside each `within` range. Errors name the failing value:
   `drawer[top] stops fitting when cabinet_width < 640mm`.
7. **Buildability**: an order of operations exists that keeps every reference
   face, meets every recipe's needs and gets every part into place. See
   [Order of operations](#order-of-operations).

---

## Smart edits

The mouse tools in the app edit the design through relationships. A **resize**
drag on a part asks: which unlocked values, if changed, give this new size?

- Every size is an expression that leads back to params. Dragging the top
  drawer 50mm wider traces its box width through `opening.width` to
  `cabinet_width`, so `cabinet_width` grows by 50mm and everything that depends
  on it follows.
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
- The result is an ordinary patch (`set cabinet_width = 950mm`), shown while
  dragging and undone in one step.

**Move** drags edit the measured offset of one placement term (the `gap:` in
`against(base.top, gap: 500mm)`), and **snap** replaces a term with `flush` or
`against` on the face the part was dropped on.

---

## Types, units and arithmetic

- Lengths are stored as `i64` nanometers: exact for metric values down to 1nm
  and for inch fractions down to 1/64in. Angles are `i64` micro-degrees.
- **Expressions are evaluated with exact fractions**, so `1in / 3` and
  `spaced(evenly)` never lose precision. Values are rounded once, when the cut
  list is made, to the project's `resolution:` (default 0.5mm), and the cut list
  says where it rounded.
- A literal that can't be stored exactly (`1/3in`) is an error that shows the
  nearest storable value.
- Types: `Length`, `Angle`, `Ratio` (exact fraction), `Count` (whole numbers, a
  kind of `Ratio`), `Bool`, `Part`, `Stock`, `Region`, plus catalog kinds such
  as `JointKind` and `HardwareKind`.
- Arithmetic: `length ± length`, `length × ratio`, `length ÷ ratio`,
  `length ÷ length → ratio`, `ratio ± × ÷ ratio`. `count ÷ length`,
  `length + angle` and `count - length` are errors.
- **Bare words** such as `evenly`, `open`, `plywood` or `cope_and_stick` are
  resolved by the type the field or argument expects. Naming a part after a
  built-in value is a warning with a rename fix.

### What z3 can check over a range

z3 works with exact real arithmetic over the params. For a param with a
`within` range, the checker rejects expressions it can't verify over the whole
range, and says so: a param multiplied by a param, division by a param, and
`spaced(evenly)` over an array whose count is a param. Each assertion is tracked
by its source span, so a failure points at the line that caused it.

---

## Patches

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
  }
  add joint shoe_shelf_dados : dado(shoe_shelf, into: [side_l, side_r]) { depth: 9mm }
  set divider.width = cabinet_depth - back.thickness - 10mm
}
```

Every item has a name, so every patch operation addresses exactly one thing.
`add ... in "carcass.stave"` picks the file within the module; without it, the
new item goes into the file that holds the first item it refers to.
Placement terms are addressed by the axis they set, because each axis has
exactly one term: `set side_l.place.deep = flush(base.back, inset: 6mm)`
replaces only the term that sets `side_l`'s front-to-back position.
