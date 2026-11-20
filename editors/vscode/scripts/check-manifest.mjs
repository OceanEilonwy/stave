// Checks package.json against the semantic token legend in
// editors/semantic-tokens.json: every type and modifier that is not one of
// the protocol's standard ones is contributed, every custom type has a
// TextMate fallback scope, and nothing is contributed that the legend does
// not have. Exits non-zero on a mismatch.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const extension = join(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(readFileSync(join(extension, "package.json"), "utf8"));
const legend = JSON.parse(readFileSync(join(extension, "..", "semantic-tokens.json"), "utf8"));

// The protocol's predefined types and modifiers (LSP 3.17), which VS Code
// knows without a contribution.
const standardTypes = new Set([
  "namespace", "type", "class", "enum", "interface", "struct", "typeParameter", "parameter",
  "variable", "property", "enumMember", "event", "function", "method", "macro", "keyword",
  "modifier", "comment", "string", "number", "regexp", "operator", "decorator",
]);
const standardModifiers = new Set([
  "declaration", "definition", "readonly", "static", "deprecated", "abstract", "async",
  "modification", "documentation", "defaultLibrary",
]);

const failures = [];
const contributes = manifest.contributes;
const contributedTypes = contributes.semanticTokenTypes.map((entry) => entry.id);
const contributedModifiers = contributes.semanticTokenModifiers.map((entry) => entry.id);
const scopes = contributes.semanticTokenScopes.find((entry) => entry.language === "stave")?.scopes ?? {};

const customTypes = legend.tokenTypes.filter((type) => !standardTypes.has(type));
const customModifiers = legend.tokenModifiers.filter((modifier) => !standardModifiers.has(modifier));

for (const type of customTypes) {
  if (!contributedTypes.includes(type)) failures.push(`type '${type}' is not contributed`);
  if (!(type in scopes)) failures.push(`type '${type}' has no semanticTokenScopes entry`);
}
for (const type of contributedTypes) {
  if (!customTypes.includes(type)) failures.push(`type '${type}' is contributed but not in the legend`);
}
for (const modifier of customModifiers) {
  if (!contributedModifiers.includes(modifier)) failures.push(`modifier '${modifier}' is not contributed`);
}
for (const modifier of contributedModifiers) {
  if (!customModifiers.includes(modifier)) {
    failures.push(`modifier '${modifier}' is contributed but not in the legend`);
  }
}
for (const selector of Object.keys(scopes)) {
  const [type, ...modifiers] = selector.split(".");
  if (!legend.tokenTypes.includes(type)) failures.push(`scope selector '${selector}': unknown type`);
  for (const modifier of modifiers) {
    if (!legend.tokenModifiers.includes(modifier)) {
      failures.push(`scope selector '${selector}': unknown modifier '${modifier}'`);
    }
  }
}
const rules = contributes.configurationDefaults["editor.semanticTokenColorCustomizations"].rules;
for (const selector of Object.keys(rules)) {
  const [tokenSelector, language] = selector.split(":");
  if (language !== "stave") failures.push(`style rule '${selector}' is not limited to stave`);
  const [type, ...modifiers] = tokenSelector.split(".");
  if (type !== "*" && !legend.tokenTypes.includes(type)) failures.push(`style rule '${selector}': unknown type`);
  for (const modifier of modifiers) {
    if (!legend.tokenModifiers.includes(modifier)) failures.push(`style rule '${selector}': unknown modifier`);
  }
}

if (failures.length > 0) {
  for (const failure of failures) console.error(`FAIL ${failure}`);
  process.exit(1);
}
console.log(
  `ok: ${legend.tokenTypes.length} types (${customTypes.length} custom), ` +
    `${legend.tokenModifiers.length} modifiers (${customModifiers.length} custom)`,
);
