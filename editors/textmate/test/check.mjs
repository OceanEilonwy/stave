// Checks the Stave TextMate grammar against the real language.
//
// 1. Every regular expression in the grammar compiles with Oniguruma, the
//    engine VS Code, GitHub and most TextMate hosts use.
// 2. The grammar's word lists match the Rust sources they are derived from:
//    the keywords and contextual words (crates/stave-lang/src/lib.rs), the
//    units (crates/stave-lang/src/ast.rs) and the closed word lists
//    (crates/stave-schema/src/words.rs, species.rs and
//    crates/stave-lang/src/placement.rs).
// 3. Every .stave file in examples/ and std/ is tokenised with the grammar
//    and compared, token by token, with a port of the lexer's first pass
//    (crates/stave-lang/src/lexer.rs): comments, doc comments, strings,
//    numbers, a quantity's number and unit, and keywords must get the scope
//    the language server's semantic tokens would map them to, and no scope
//    may be `invalid.*` (the corpus is valid code).
//
// Run with `npm test` in editors/textmate. Exits non-zero on any failure.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import oniguruma from "vscode-oniguruma";
import textmate from "vscode-textmate";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const repository = join(here, "..", "..", "..");
const grammarPath = join(here, "..", "stave.tmLanguage.json");
const grammarText = readFileSync(grammarPath, "utf8");
const grammarJson = JSON.parse(grammarText);

const failures = [];
function fail(message) {
  failures.push(message);
}

// ---- 1. Oniguruma compiles every expression ----

const wasm = readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
await oniguruma.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));

let expressionCount = 0;
function compileAll(node, path) {
  if (Array.isArray(node)) {
    node.forEach((child, index) => compileAll(child, `${path}[${index}]`));
    return;
  }
  if (node === null || typeof node !== "object") {
    return;
  }
  for (const key of ["match", "begin", "end", "while"]) {
    if (typeof node[key] === "string") {
      expressionCount += 1;
      try {
        new oniguruma.OnigScanner([node[key]]).dispose();
      } catch (error) {
        fail(`${path}.${key} does not compile: ${error}`);
      }
    }
  }
  for (const [key, value] of Object.entries(node)) {
    compileAll(value, `${path}.${key}`);
  }
}
compileAll(grammarJson, "grammar");

// ---- 2. Word lists match the Rust sources ----

function readSource(path) {
  return readFileSync(join(repository, path), "utf8");
}

function quotedIn(text) {
  return [...text.matchAll(/"([^"]*)"/g)].map((found) => found[1]);
}

function rustStringArray(text, name) {
  const found = text.match(new RegExp(`pub const ${name}: &\\[&str\\] = &\\[([\\s\\S]*?)\\];`));
  if (!found) {
    fail(`could not find ${name} in the Rust sources`);
    return [];
  }
  return quotedIn(found[1]);
}

/** The words of `Enum::Variant => "word"` arms for one enum. */
function matchArms(text, enumName) {
  return [...text.matchAll(new RegExp(`\\b${enumName}::\\w+ => "([a-z_]+)"`, "g"))].map(
    (found) => found[1],
  );
}

/** The words of one `word_enum! { Name { Variant = "word", ... } }`. */
function wordEnum(text, enumName) {
  const start = text.search(new RegExp(`\\n\\s*${enumName} \\{`));
  if (start < 0) {
    fail(`could not find word_enum ${enumName}`);
    return [];
  }
  const rest = text.slice(start);
  const body = rest.slice(0, rest.indexOf("\n}"));
  return [...body.matchAll(/= "([a-z_]+)"/g)].map((found) => found[1]);
}

/** The alternation of words in a pattern such as `(?<!..)(a|b|c)(?!..)`. */
function alternation(pattern) {
  const found = pattern.match(/\((?:\?:)?([a-z_]+(?:\|[a-z_]+)+)\)/);
  return found ? found[1].split("|") : [];
}

function sameSet(label, actual, expected) {
  const actualSet = new Set(actual);
  const expectedSet = new Set(expected);
  const missing = [...expectedSet].filter((word) => !actualSet.has(word)).sort();
  const extra = [...actualSet].filter((word) => !expectedSet.has(word)).sort();
  if (missing.length > 0 || extra.length > 0) {
    fail(`${label}: missing [${missing.join(", ")}], not in the language [${extra.join(", ")}]`);
  }
}

const lib = readSource("crates/stave-lang/src/lib.rs");
const keywords = rustStringArray(lib, "KEYWORDS");
const contextualWords = rustStringArray(lib, "CONTEXTUAL_WORDS");

const ast = readSource("crates/stave-lang/src/ast.rs");
const units = matchArms(ast, "Unit");

const placement = readSource("crates/stave-lang/src/placement.rs");
const words = readSource("crates/stave-schema/src/words.rs");
const species = readSource("crates/stave-schema/src/species.rs");
// Each variant of `enum Species` is documented with its word: `/// \`white_oak\``.
const speciesEnum = species.match(/pub enum Species \{([\s\S]*?)\n\}/)?.[1] ?? "";
const speciesWords = [...speciesEnum.matchAll(/\/\/\/ `([a-z_]+)`/g)].map((found) => found[1]);
for (const word of speciesWords) {
  if (!new RegExp(`facts\\(\\s*"${word}"`).test(species)) fail(`species '${word}' has no facts("${word}", ...) row`);
}
const outputUnits = wordEnum(words, "OutputUnit");
const through = words.match(/pub const THROUGH: &str = "([a-z_]+)";/)?.[1];
const spacing = quotedIn(words.match(/pub const SPACING_WORDS: \[&str; \d+\] = \[([^\]]*)\]/)?.[1] ?? "");
const enumWords = [
  ...matchArms(placement, "Axis"),
  ...matchArms(placement, "Direction"),
  ...matchArms(placement, "Face"),
  ...matchArms(placement, "PartAxis"),
  ...speciesWords,
  ...wordEnum(words, "StockForm"),
  ...wordEnum(words, "Measure"),
  ...wordEnum(words, "MotionWord"),
  ...wordEnum(words, "Grain"),
  ...(through ? [through] : []),
  ...spacing,
];
if (speciesWords.length < 10 || spacing.length === 0 || !through || units.length !== 6) {
  fail("the word lists could not be read from the Rust sources");
}

const repo = grammarJson.repository;
sameSet(
  "keywords",
  [
    ...repo.keywords.patterns.flatMap((pattern) => alternation(pattern.match)),
  ],
  keywords,
);
sameSet("units", alternation(repo.quantities.patterns[0].match), units);
sameSet("output units", alternation(repo["output-units"].match), outputUnits);
sameSet("enum words", alternation(repo["enum-words"].match), enumWords);
const contextualInGrammar = [
  ...repo["contextual-entries"].patterns.flatMap((pattern) => alternation(pattern.match)),
  "shape",
  "as",
];
for (const word of contextualInGrammar) {
  if (!contextualWords.includes(word)) {
    fail(`contextual word '${word}' is not in CONTEXTUAL_WORDS`);
  }
}
const declared = repo.declarations.patterns.flatMap((pattern) => {
  const listed = alternation(pattern.match);
  return listed.length > 0 ? listed : [pattern.match.match(/\(([a-z]+)\)/)[1]];
});
sameSet(
  "declaration keywords",
  declared,
  ["kind", "component", "recipe", "param", "let", "region", "stock", "part", "joint", "hardware", "assembly", "profile", "scene", "require"],
);

// ---- 3. The corpus, against a port of the lexer ----

const isIdentStart = (c) => /[A-Za-z_]/.test(c ?? "");
const isIdentChar = (c) => /[A-Za-z0-9_]/.test(c ?? "");
const isDigit = (c) => /[0-9]/.test(c ?? "");

/** The lexer's first pass, as far as the checks need: kinds and offsets. */
function lex(text) {
  const tokens = [];
  const push = (kind, start, end, extra = {}) => tokens.push({ kind, start, end, ...extra });
  let at = 0;
  const scanWhile = (from, keep) => {
    let position = from;
    while (position < text.length && keep(text[position])) position += 1;
    return position;
  };
  const scanInt = (from) => scanWhile(from, (c) => isDigit(c) || c === "_");
  while (at < text.length) {
    const start = at;
    const c = text[at];
    const next = text[at + 1];
    if (c === "\n" || c === " " || c === "\t" || c === "\r") {
      at += 1;
    } else if (c === "/" && next === "/") {
      const kind = text[at + 2] === "/" ? "DOC" : "LINE_COMMENT";
      let end = scanWhile(at, (ch) => ch !== "\n");
      if (end > start && text[end - 1] === "\r") end -= 1;
      push(kind, start, end);
      at = end;
    } else if (c === "/" && next === "*") {
      const close = text.indexOf("*/", at + 2);
      at = close < 0 ? text.length : close + 2;
      push("BLOCK_COMMENT", start, at);
    } else if (c === '"') {
      at += 1;
      let textStart = at;
      for (;;) {
        const ch = text[at];
        if (ch === undefined || ch === "\n") {
          if (at > textStart) push("STRING_TEXT", textStart, at);
          break;
        }
        if (ch === '"') {
          if (at > textStart) push("STRING_TEXT", textStart, at);
          at += 1;
          break;
        }
        if (ch === "\\") {
          at += 2;
          continue;
        }
        if (ch === "{") {
          const interpolation = text.slice(at).match(/^\{[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*\}/);
          if (interpolation) {
            if (at > textStart) push("STRING_TEXT", textStart, at);
            push("INTERPOLATION", at, at + interpolation[0].length);
            at += interpolation[0].length;
            textStart = at;
            continue;
          }
        }
        at += 1;
      }
    } else if (isDigit(c)) {
      const intEnd = scanInt(at);
      let magnitudeEnd = -1;
      let unitless = intEnd;
      if (text[intEnd] === "-" && isDigit(text[intEnd + 1])) {
        const numeratorEnd = scanInt(intEnd + 1);
        if (text[numeratorEnd] === "/" && isDigit(text[numeratorEnd + 1])) {
          const denominatorEnd = scanInt(numeratorEnd + 1);
          if (isIdentStart(text[denominatorEnd])) magnitudeEnd = denominatorEnd;
        }
      }
      if (magnitudeEnd < 0 && text[intEnd] === "/" && isDigit(text[intEnd + 1])) {
        const denominatorEnd = scanInt(intEnd + 1);
        if (isIdentStart(text[denominatorEnd])) magnitudeEnd = denominatorEnd;
      }
      if (magnitudeEnd < 0 && text[intEnd] === "." && isDigit(text[intEnd + 1])) {
        const fractionEnd = scanWhile(intEnd + 1, isDigit);
        if (isIdentStart(text[fractionEnd])) magnitudeEnd = fractionEnd;
        else unitless = fractionEnd;
      }
      if (magnitudeEnd < 0 && unitless === intEnd && isIdentStart(text[intEnd])) {
        magnitudeEnd = intEnd;
      }
      if (magnitudeEnd >= 0) {
        const end = scanWhile(magnitudeEnd, isIdentChar);
        push("QUANTITY", start, end, { magnitudeEnd });
        at = end;
      } else {
        push("NUMBER", start, unitless);
        at = unitless;
      }
    } else if (isIdentStart(c)) {
      const end = scanWhile(at, isIdentChar);
      const word = text.slice(start, end);
      push(keywords.includes(word) ? "KEYWORD" : "IDENT", start, end);
      at = end;
    } else {
      push("PUNCTUATION", start, start + (text.slice(at, at + 2).match(/^(\.\.|==|!=|<=|>=|&&|\|\|)$/) ? 2 : 1));
      at = tokens[tokens.length - 1].end;
    }
  }
  return tokens;
}

const onigLib = Promise.resolve({
  createOnigScanner: (patterns) => new oniguruma.OnigScanner(patterns),
  createOnigString: (text) => new oniguruma.OnigString(text),
});
const registry = new textmate.Registry({
  onigLib,
  loadGrammar: async (scopeName) =>
    scopeName === "source.stave" ? textmate.parseRawGrammar(grammarText, grammarPath) : null,
});
const grammar = await registry.loadGrammar("source.stave");

/** The scopes of every UTF-16 unit of the file, as joined strings. */
function scopesOf(text) {
  const scopes = new Array(text.length).fill("");
  let ruleStack = textmate.INITIAL;
  let lineStart = 0;
  for (const line of text.split("\n")) {
    const result = grammar.tokenizeLine(line, ruleStack);
    for (const token of result.tokens) {
      const joined = token.scopes.join(" ");
      for (let index = token.startIndex; index < Math.min(token.endIndex, line.length); index += 1) {
        scopes[lineStart + index] = joined;
      }
    }
    ruleStack = result.ruleStack;
    lineStart += line.length + 1;
  }
  return scopes;
}

function staveFiles(folder) {
  const out = [];
  for (const entry of readdirSync(folder).sort()) {
    const path = join(folder, entry);
    if (statSync(path).isDirectory()) out.push(...staveFiles(path));
    else if (entry.endsWith(".stave")) out.push(path);
  }
  return out;
}

const corpus = [...staveFiles(join(repository, "examples")), ...staveFiles(join(repository, "std"))];
const counts = { files: 0, tokens: 0, quantities: 0, keywords: 0 };

for (const file of corpus) {
  const text = readFileSync(file, "utf8");
  const name = relative(repository, file);
  const scopes = scopesOf(text);
  counts.files += 1;
  const lineOf = (offset) => text.slice(0, offset).split("\n").length;
  const expectRange = (start, end, wanted, what) => {
    for (let index = start; index < end; index += 1) {
      if (!wanted.test(scopes[index])) {
        fail(`${name}:${lineOf(index)}: ${what} '${text.slice(start, end)}' has scopes '${scopes[index]}'`);
        return;
      }
    }
  };
  scopes.forEach((scope, index) => {
    if (/\binvalid\./.test(scope)) {
      fail(`${name}:${lineOf(index)}: invalid scope '${scope}' on '${text[index]}'`);
    }
  });
  const tokens = lex(text);
  tokens.forEach((token, index) => {
    counts.tokens += 1;
    switch (token.kind) {
      case "DOC":
        expectRange(token.start, token.end, /comment\.line\.documentation\.stave/, "doc comment");
        break;
      case "LINE_COMMENT":
        expectRange(token.start, token.end, /comment\.line\.double-slash\.stave/, "comment");
        break;
      case "BLOCK_COMMENT":
        expectRange(token.start, token.end, /comment\.block\.stave/, "block comment");
        break;
      case "STRING_TEXT":
        expectRange(token.start, token.end, /string\.quoted\.double\.stave/, "string text");
        break;
      case "INTERPOLATION":
        expectRange(token.start, token.end, /meta\.interpolation\.stave/, "interpolation");
        break;
      case "NUMBER":
        expectRange(token.start, token.end, /constant\.numeric\./, "number");
        break;
      case "QUANTITY":
        counts.quantities += 1;
        expectRange(token.start, token.magnitudeEnd, /constant\.numeric\.stave/, "quantity's number");
        expectRange(token.magnitudeEnd, token.end, /keyword\.other\.unit\.stave/, "quantity's unit");
        break;
      case "KEYWORD": {
        counts.keywords += 1;
        const previous = tokens[index - 1];
        const afterDot = previous?.kind === "PUNCTUATION" && text[previous.start] === "." && previous.end === token.start;
        const beforeColon = /^\s*:/.test(text.slice(token.end, text.indexOf("\n", token.end) + 1 || undefined));
        if (afterDot) {
          expectRange(token.start, token.end, /variable\.other\.member\.stave/, "member");
        } else if (beforeColon) {
          expectRange(token.start, token.end, /variable\.other\.property\.stave/, "key");
        } else if (/(?<![A-Za-z0-9_])kind\s+[A-Za-z_]\w*\s*:\s*$/.test(text.slice(text.lastIndexOf("\n", token.start) + 1, token.start))) {
          // `kind name : joint`: the base kind, a type to the language server.
          expectRange(token.start, token.end, /entity\.other\.inherited-class\.stave/, "base kind");
        } else {
          expectRange(
            token.start,
            token.end,
            /storage\.type\.|storage\.modifier\.|keyword\.other\.|constant\.language\.boolean\./,
            "keyword",
          );
        }
        break;
      }
      default:
        break;
    }
  });
}

if (counts.files < 50) {
  fail(`expected the whole corpus, found only ${counts.files} files`);
}

console.log(
  `compiled ${expressionCount} expressions; checked ${counts.files} files, ` +
    `${counts.tokens} tokens (${counts.quantities} quantities, ${counts.keywords} keywords)`,
);
if (failures.length > 0) {
  for (const message of failures.slice(0, 50)) console.error(`FAIL ${message}`);
  console.error(`${failures.length} failure(s)`);
  process.exit(1);
}
console.log("ok");
