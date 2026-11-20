// The Stave extension: starts `stave-lsp` over standard input and output
// and shows the standard library's read-only `stave-std:///` documents.
//
// Everything the editor shows (diagnostics, completion, semantic tokens,
// hover, rename, ...) comes from the server; the extension only starts it,
// restarts it on request and fetches standard library text. The contract
// between them is editors/README.md.

import * as vscode from "vscode";
import {
  type Executable,
  LanguageClient,
  type LanguageClientOptions,
  RequestType,
  type ServerOptions,
} from "vscode-languageclient/node";

/** The scheme of the standard library's documents, served by stave-lsp. */
const STANDARD_LIBRARY_SCHEME = "stave-std";

/** The parameters of `stave/standardLibraryText`. */
interface StandardLibraryTextParams {
  /** The document's URI: `stave-std:///joinery/dado.stave`. */
  uri: string;
}

/**
 * Asks stave-lsp for the text of one standard library file. The answer is
 * `null` when the URI names no file.
 */
const standardLibraryText = new RequestType<StandardLibraryTextParams, string | null, void>(
  "stave/standardLibraryText",
);

let client: LanguageClient | undefined;

/** The command that starts the server: `stave.server.path`, or `stave-lsp`. */
function serverCommand(): string {
  const configured = vscode.workspace.getConfiguration("stave").get<string>("server.path", "");
  const trimmed = configured.trim();
  return trimmed.length > 0 ? trimmed : "stave-lsp";
}

function createClient(): LanguageClient {
  // No arguments and no `transport`: stave-lsp always speaks the protocol
  // over standard input and output, and a `transport` would add `--stdio`.
  const executable: Executable = { command: serverCommand(), args: [] };
  const serverOptions: ServerOptions = { run: executable, debug: executable };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { language: "stave", scheme: "file" },
      { language: "stave", scheme: "untitled" },
      { language: "stave", scheme: STANDARD_LIBRARY_SCHEME },
    ],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.stave"),
    },
    outputChannelName: "Stave",
  };
  return new LanguageClient("stave", "Stave", serverOptions, clientOptions);
}

async function startClient(): Promise<void> {
  const created = createClient();
  client = created;
  try {
    await created.start();
  } catch (error) {
    client = undefined;
    const message = error instanceof Error ? error.message : String(error);
    const openSettings = "Open Settings";
    const choice = await vscode.window.showErrorMessage(
      `Stave: could not start '${serverCommand()}' (${message}). Install stave-lsp ` +
        "(cargo install --path crates/stave-lsp) or set stave.server.path.",
      openSettings,
    );
    if (choice === openSettings) {
      await vscode.commands.executeCommand("workbench.action.openSettings", "stave.server.path");
    }
  }
}

async function stopClient(): Promise<void> {
  const running = client;
  client = undefined;
  if (running !== undefined) {
    await running.stop();
  }
}

/** Shows `stave-std:///…` documents with the text the server sends. */
class StandardLibraryDocuments implements vscode.TextDocumentContentProvider {
  async provideTextDocumentContent(
    uri: vscode.Uri,
    token: vscode.CancellationToken,
  ): Promise<string> {
    if (client === undefined) {
      throw new Error("stave-lsp is not running");
    }
    const text = await client.sendRequest(
      standardLibraryText,
      { uri: uri.toString() },
      token,
    );
    if (text === null) {
      throw new Error(`no standard library file ${uri.toString()}`);
    }
    return text;
  }
}

/** Called by VS Code when a `.stave` file is opened. */
export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider(
      STANDARD_LIBRARY_SCHEME,
      new StandardLibraryDocuments(),
    ),
    vscode.commands.registerCommand("stave.restartServer", async () => {
      await stopClient();
      await startClient();
    }),
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (event.affectsConfiguration("stave.server.path")) {
        await stopClient();
        await startClient();
      }
    }),
  );
  await startClient();
}

/** Called by VS Code on shutdown. */
export async function deactivate(): Promise<void> {
  await stopClient();
}
