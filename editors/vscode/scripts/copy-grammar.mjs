// Copies the TextMate grammar from editors/textmate into the extension, so
// the packaged extension carries it. editors/textmate is the only copy kept
// in git.

import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const extension = join(dirname(fileURLToPath(import.meta.url)), "..");
mkdirSync(join(extension, "syntaxes"), { recursive: true });
copyFileSync(
  join(extension, "..", "textmate", "stave.tmLanguage.json"),
  join(extension, "syntaxes", "stave.tmLanguage.json"),
);
