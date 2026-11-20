//! Drives the language server over in-memory channels, as an editor
//! would: open an example, edit it (with non-ASCII text before the
//! edits, so UTF-16 columns matter), and receive diagnostics from both
//! tiers, completions, signature help, semantic tokens, a definition in
//! the standard library, hover, code lenses and a rename. Nothing is
//! written to disk: the edits live in the server's buffers.

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidOpenTextDocument, Exit, Initialized, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    CodeLensRequest, Completion, GotoDefinition, HoverRequest, Initialize, Rename, Request as _,
    SemanticTokensFullRequest, Shutdown, SignatureHelpRequest,
};
use lsp_types::{
    CompletionResponse, DocumentChanges, GotoDefinitionResponse, InitializeResult, NumberOrString,
    OneOf, Position, PublishDiagnosticsParams, Range, SemanticTokensResult, SignatureHelp,
    TextDocumentContentChangeEvent, Uri, WorkspaceEdit,
};
use serde_json::json;
use stave_ide::Document;
use stave_lsp::{Config, document_uri};

/// How long to wait for any one answer: the slow tier runs the whole
/// pipeline in a debug build.
const PATIENCE: Duration = Duration::from_secs(120);

struct Client {
    connection: Connection,
    server: Option<JoinHandle<()>>,
    next_id: i32,
}

impl Client {
    fn start() -> Client {
        let (server, client) = Connection::memory();
        let config = Config {
            slow_delay: Duration::from_millis(50),
        };
        let handle = std::thread::spawn(move || {
            stave_lsp::run(&server, config).unwrap();
        });
        Client {
            connection: client,
            server: Some(handle),
            next_id: 1,
        }
    }

    fn notify(&self, method: &str, params: serde_json::Value) {
        self.connection
            .sender
            .send(Message::Notification(Notification::new(
                method.to_owned(),
                params,
            )))
            .unwrap();
    }

    /// Sends a request and waits for its response, setting aside nothing:
    /// notifications that arrive meanwhile are dropped.
    fn request(&mut self, method: &str, params: serde_json::Value) -> Response {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        self.connection
            .sender
            .send(Message::Request(Request::new(
                id.clone(),
                method.to_owned(),
                params,
            )))
            .unwrap();
        let message = self.wait(|message| match message {
            Message::Response(response) => response.id == id,
            Message::Request(_) | Message::Notification(_) => false,
        });
        let Message::Response(response) = message else {
            unreachable!()
        };
        response
    }

    fn ok(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let response = self.request(method, params);
        match response.response_result {
            Ok(value) => value,
            Err(error) => panic!("{method} failed: {}", error.message),
        }
    }

    fn wait(&self, wanted: impl Fn(&Message) -> bool) -> Message {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self
                .connection
                .receiver
                .recv_timeout(left)
                .expect("the server answered in time");
            if wanted(&message) {
                return message;
            }
        }
    }

    /// Waits for diagnostics of `uri` that `wanted` accepts.
    fn diagnostics(
        &self,
        uri: &Uri,
        wanted: impl Fn(&[lsp_types::Diagnostic]) -> bool,
    ) -> Vec<lsp_types::Diagnostic> {
        let message = self.wait(|message| {
            published(message)
                .is_some_and(|params| params.uri == *uri && wanted(&params.diagnostics))
        });
        published(&message).unwrap().diagnostics
    }

    fn stop(mut self) {
        let response = self.request(Shutdown::METHOD, serde_json::Value::Null);
        assert!(response.response_result.is_ok());
        self.notify(Exit::METHOD, serde_json::Value::Null);
        self.server.take().unwrap().join().unwrap();
    }
}

fn published(message: &Message) -> Option<PublishDiagnosticsParams> {
    let Message::Notification(notification) = message else {
        return None;
    };
    (notification.method == PublishDiagnostics::METHOD)
        .then(|| serde_json::from_value(notification.params.clone()).unwrap())
}

fn codes(diagnostics: &[lsp_types::Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .filter_map(|d| match &d.code {
            Some(NumberOrString::String(code)) => Some(code.clone()),
            Some(NumberOrString::Number(_)) | None => None,
        })
        .collect()
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .canonicalize()
        .unwrap()
}

/// The protocol position of a byte offset, in UTF-16 units.
fn position(text: &str, offset: usize) -> Position {
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let column = text[line_start..offset].encode_utf16().count();
    Position::new(line as u32, column as u32)
}

fn position_of(text: &str, needle: &str) -> Position {
    position(text, text.find(needle).unwrap())
}

/// One incremental change: `range` of `text` becomes `new_text`.
fn change(
    client: &Client,
    uri: &Uri,
    version: i32,
    text: &mut String,
    range: std::ops::Range<usize>,
    new_text: &str,
) {
    let start = position(text, range.start);
    let end = position(text, range.end);
    text.replace_range(range, new_text);
    client.notify(
        DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [ TextDocumentContentChangeEvent {
                range: Some(Range::new(start, end)),
                range_length: None,
                text: new_text.to_owned(),
            } ],
        }),
    );
}

#[test]
fn an_editor_session_over_the_protocol() {
    let folder = example("clothing_cabinet");
    let path = folder.join("doors.stave");
    let uri = document_uri(&Document::File(path.clone())).unwrap();
    let mut text = std::fs::read_to_string(&path).unwrap();

    let mut client = Client::start();
    let initialized = client.ok(
        Initialize::METHOD,
        json!({
            "capabilities": {
                "workspace": { "workspaceEdit": { "documentChanges": true } },
            },
            "rootUri": document_uri(&Document::File(folder.clone())).unwrap(),
        }),
    );
    let initialized: InitializeResult = serde_json::from_value(initialized).unwrap();
    assert_eq!(initialized.server_info.unwrap().name, "stave-lsp");
    let Some(lsp_types::SemanticTokensServerCapabilities::SemanticTokensOptions(tokens)) =
        initialized.capabilities.semantic_tokens_provider
    else {
        panic!("no semantic tokens");
    };
    let types: Vec<String> = tokens
        .legend
        .token_types
        .iter()
        .map(|t| t.as_str().to_owned())
        .collect();
    let modifiers: Vec<String> = tokens
        .legend
        .token_modifiers
        .iter()
        .map(|m| m.as_str().to_owned())
        .collect();
    client.notify(Initialized::METHOD, json!({}));

    client.notify(
        DidOpenTextDocument::METHOD,
        json!({
            "textDocument": { "uri": uri, "languageId": "stave", "version": 1, "text": text },
        }),
    );
    // The slow tier on the example as it is: no errors.
    let clean = client.diagnostics(&uri, |_| true);
    assert!(
        clean
            .iter()
            .all(|d| d.severity != Some(lsp_types::DiagnosticSeverity::ERROR)),
        "{clean:?}"
    );

    // Non-ASCII text in a comment before the name the edits change, so
    // a wrong column conversion would edit the wrong bytes.
    let line = text.find("  hinge_side:").unwrap();
    change(&client, &uri, 2, &mut text, line..line + 2, "  /* é🪚 */ ");

    // A syntax error is published at once (fast tier).
    let frame = text.find("  frame:").unwrap();
    change(&client, &uri, 3, &mut text, frame..frame, "  part : {\n");
    let broken = client.diagnostics(&uri, |d| codes(d).iter().any(|c| c == "syntax"));
    assert!(!broken.is_empty());
    change(
        &client,
        &uri,
        4,
        &mut text,
        frame..frame + "  part : {\n".len(),
        "",
    );

    // A name that means nothing comes from the slow tier, where the name is.
    let side = text.find("side_l\n").unwrap();
    change(
        &client,
        &uri,
        5,
        &mut text,
        side..side + "side_l".len(),
        "side_x",
    );
    let unknown = client.diagnostics(&uri, |d| codes(d).iter().any(|c| c == "unknown-name"));
    let unknown = unknown
        .iter()
        .find(|d| d.code == Some(NumberOrString::String("unknown-name".to_owned())))
        .unwrap();
    assert_eq!(unknown.range.start, position(&text, side));
    assert!(unknown.message.contains("side_x"), "{}", unknown.message);
    change(
        &client,
        &uri,
        6,
        &mut text,
        side..side + "side_x".len(),
        "side_l",
    );
    let fixed = client.diagnostics(&uri, |d| !codes(d).iter().any(|c| c == "unknown-name"));
    assert!(!codes(&fixed).iter().any(|c| c == "syntax"));

    // Completion after `hw.` offers the hardware module's items.
    let after_module = position(&text, text.find("hw.brass").unwrap() + "hw.".len());
    let completions = client.ok(
        Completion::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": after_module }),
    );
    let completions: CompletionResponse = serde_json::from_value(completions).unwrap();
    let CompletionResponse::Array(items) = completions else {
        panic!("expected a list");
    };
    assert!(
        items.iter().any(|item| item.label == "brass_knob_26"),
        "{:?}",
        items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );

    // Signature help inside `overlay(`.
    let in_overlay = position(&text, text.find("overlay(").unwrap() + "overlay(".len());
    let help = client.ok(
        SignatureHelpRequest::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": in_overlay }),
    );
    let help: Option<SignatureHelp> = serde_json::from_value(help).unwrap();
    assert!(help.unwrap().signatures[0].label.starts_with("overlay("));

    // Semantic tokens: `mm` is a unit, `side_l` a reference to a part.
    let tokens = client.ok(
        SemanticTokensFullRequest::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    let tokens: SemanticTokensResult = serde_json::from_value(tokens).unwrap();
    let SemanticTokensResult::Tokens(tokens) = tokens else {
        panic!("expected tokens");
    };
    let mut decoded = Vec::new();
    let (mut line, mut column) = (0u32, 0u32);
    for token in &tokens.data {
        if token.delta_line > 0 {
            line += token.delta_line;
            column = token.delta_start;
        } else {
            column += token.delta_start;
        }
        let line_text = text.lines().nth(line as usize).unwrap();
        let units: Vec<u16> = line_text.encode_utf16().collect();
        let word =
            String::from_utf16(&units[column as usize..(column + token.length) as usize]).unwrap();
        let named: Vec<&str> = (0..modifiers.len())
            .filter(|bit| token.token_modifiers_bitset & (1 << bit) != 0)
            .map(|bit| modifiers[bit].as_str())
            .collect();
        decoded.push((word, types[token.token_type as usize].clone(), named));
    }
    assert!(
        decoded
            .iter()
            .any(|(word, kind, _)| word == "mm" && kind == "unit")
    );
    assert!(
        decoded
            .iter()
            .any(|(word, kind, _)| word == "18" && kind == "number")
    );
    assert!(decoded.iter().any(|(word, kind, named)| word == "side_l"
        && kind == "part"
        && named.contains(&"reference")));
    assert!(
        decoded
            .iter()
            .any(|(word, kind, _)| word == "/* é🪚 */" && kind == "comment")
    );

    // Definition of a standard component lands in a read-only document.
    let on_component = position_of(&text, "FramePanelDoor");
    let definition = client.ok(
        GotoDefinition::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": on_component }),
    );
    let definition: GotoDefinitionResponse = serde_json::from_value(definition).unwrap();
    let GotoDefinitionResponse::Scalar(location) = definition else {
        panic!("expected one location");
    };
    assert_eq!(
        location.uri.as_str(),
        "stave-std:///casework/frame_panel_door.stave"
    );
    let standard = client.ok(
        stave_lsp::STANDARD_DOCUMENT_REQUEST,
        json!({ "uri": location.uri }),
    );
    assert!(
        standard["text"]
            .as_str()
            .unwrap()
            .contains("component FramePanelDoor")
    );
    let standard_tokens = client.ok(
        SemanticTokensFullRequest::METHOD,
        json!({ "textDocument": { "uri": location.uri } }),
    );
    assert!(!standard_tokens["data"].as_array().unwrap().is_empty());

    // Hover and code lenses.
    let hover = client.ok(
        HoverRequest::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": position(&text, side) }),
    );
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("side_l")
    );
    let lenses = client.ok(
        CodeLensRequest::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    let titles: Vec<&str> = lenses
        .as_array()
        .unwrap()
        .iter()
        .map(|lens| lens["command"]["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"show in 3D"), "{titles:?}");

    // Renaming a standard component is refused; renaming door_l edits
    // its declaration and the mirror that names it.
    let refused = client.request(
        Rename::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": on_component, "newName": "Door" }),
    );
    assert!(refused.response_result.is_err());
    let on_door = position_of(&text, "door_l :");
    let rename = client.ok(
        Rename::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": on_door, "newName": "door_left" }),
    );
    let rename: WorkspaceEdit = serde_json::from_value(rename).unwrap();
    let Some(DocumentChanges::Edits(files)) = rename.document_changes else {
        panic!("expected document changes");
    };
    let doors = files
        .iter()
        .find(|file| file.text_document.uri == uri)
        .unwrap();
    assert_eq!(doors.text_document.version, Some(6));
    let mut renamed = text.clone();
    let mut edits: Vec<_> = doors
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            OneOf::Right(annotated) => annotated.text_edit.clone(),
        })
        .collect();
    edits
        .sort_by_key(|edit| std::cmp::Reverse((edit.range.start.line, edit.range.start.character)));
    for edit in &edits {
        let offset = |p: Position| {
            let line_start: usize = renamed
                .split_inclusive('\n')
                .take(p.line as usize)
                .map(str::len)
                .sum();
            let line = &renamed[line_start..];
            let mut units = 0;
            let mut bytes = 0;
            for c in line.chars() {
                if units >= p.character as usize {
                    break;
                }
                units += c.len_utf16();
                bytes += c.len_utf8();
            }
            line_start + bytes
        };
        let (start, end) = (offset(edit.range.start), offset(edit.range.end));
        renamed.replace_range(start..end, &edit.new_text);
    }
    assert!(renamed.contains("assembly door_left : casework.FramePanelDoor"));
    assert!(renamed.contains("mirror(door_left, about"));
    assert!(!renamed.contains("door_l "));

    client.stop();
}
