//! The server's state and main loop: open documents onto [`Buffers`],
//! one [`Analysis`] per project folder rebuilt on every change (through
//! the project's [`AnalysisHost`], which redoes only what the change
//! touched), the fast
//! diagnostics published at once and the slow ones from a background
//! check after typing pauses.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, never, select};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response, ResponseError};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument, Exit,
    LogMessage, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{
    CodeLensRequest, Completion, GotoDefinition, HoverRequest, PrepareRenameRequest, References,
    Rename, Request as _, SemanticTokensFullRequest, SignatureHelpRequest,
};
use lsp_types::{
    CodeLens, CodeLensOptions, CodeLensParams, CompletionOptions, CompletionParams,
    CompletionResponse, DocumentChanges, GotoDefinitionParams, GotoDefinitionResponse, HoverParams,
    HoverProviderCapability, InitializeParams, Location, LogMessageParams, MessageType, OneOf,
    OptionalVersionedTextDocumentIdentifier, PrepareRenameResponse, PublishDiagnosticsParams,
    ReferenceParams, RenameOptions, RenameParams, SaveOptions, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, SignatureHelp, SignatureHelpOptions,
    SignatureHelpParams, TextDocumentEdit, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Uri,
    WorkDoneProgressOptions, WorkspaceEdit,
};
use stave_ide::{Analysis, AnalysisHost, Buffers, Document, FilePosition, Overlay};
use stave_lang::ast::Span;
use stave_lang::{FileId, SourceMap};

use crate::convert::{self, Sources};
use crate::legend;
use crate::line_index::{self, LineIndex, PositionEncoding};
use crate::uri::{document_uri, uri_document};
use crate::{Config, STANDARD_DOCUMENT_REQUEST, ServerError};

/// How many folders above a file are searched for its project.
const PROJECT_SEARCH_DEPTH: usize = 4;

/// Runs the server on a connection until the client exits: the
/// initialize handshake, then the main loop.
pub(crate) fn run(connection: &Connection, config: Config) -> Result<(), ServerError> {
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params)
        .map_err(|error| ServerError::Initialize(error.to_string()))?;
    let encoding = PositionEncoding::negotiate(
        params
            .capabilities
            .general
            .as_ref()
            .and_then(|general| general.position_encodings.as_deref()),
    );
    let document_changes = params
        .capabilities
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.workspace_edit.as_ref())
        .and_then(|edit| edit.document_changes)
        .unwrap_or(false);
    let result = serde_json::json!({
        "capabilities": capabilities(encoding),
        "serverInfo": { "name": "stave-lsp", "version": env!("CARGO_PKG_VERSION") },
    });
    connection.initialize_finish(id, result)?;
    let (slow_sender, slow_receiver) = crossbeam_channel::unbounded();
    let mut server = Server {
        sender: connection.sender.clone(),
        config,
        encoding,
        document_changes,
        buffers: Buffers::new(),
        versions: BTreeMap::new(),
        documents: BTreeMap::new(),
        projects: BTreeMap::new(),
        generations: 0,
        checking: BTreeSet::new(),
        slow_sender,
        shut_down: false,
    };
    server.main_loop(connection, &slow_receiver)
}

/// What the server announces it can do.
fn capabilities(encoding: PositionEncoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(encoding.kind()),
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                will_save: None,
                will_save_wait_until: None,
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(false),
                })),
            },
        )),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                work_done_progress_options: WorkDoneProgressOptions::default(),
                legend: legend::legend(),
                range: None,
                full: Some(SemanticTokensFullOptions::Bool(true)),
            },
        )),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_owned(), "[".to_owned(), ":".to_owned()]),
            ..CompletionOptions::default()
        }),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".to_owned(), ",".to_owned()]),
            retrigger_characters: None,
            work_done_progress_options: WorkDoneProgressOptions::default(),
        }),
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(false),
        }),
        ..ServerCapabilities::default()
    }
}

/// A slow check's diagnostics for one generation of a project.
struct SlowResult {
    folder: PathBuf,
    generation: u64,
    diagnostics: BTreeMap<Document, Vec<lsp_types::Diagnostic>>,
}

/// One project folder with at least one open document.
#[derive(Default)]
struct Project {
    /// The latest analysis; `None` when the folder could not be loaded.
    analysis: Option<Analysis>,
    /// What the analyses so far worked out, for the next to reuse.
    host: AnalysisHost,
    /// The text's generation, from the server-wide count of changes, so a
    /// slow check's result for older text is dropped, even one started
    /// before the project was closed and opened again.
    generation: u64,
    /// The slow tier's diagnostics for the current generation.
    slow: BTreeMap<Document, Vec<lsp_types::Diagnostic>>,
    /// When the next slow check starts, once typing has paused.
    due: Option<Instant>,
    /// What was last published for each file of the project.
    published: BTreeMap<Document, Vec<lsp_types::Diagnostic>>,
}

/// Whether the main loop goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Exit,
}

/// A request's failure, as the protocol reports it.
fn failure(code: ErrorCode, message: impl Into<String>) -> ResponseError {
    ResponseError {
        code: code as i32,
        message: message.into(),
        data: None,
    }
}

struct Server {
    sender: Sender<Message>,
    config: Config,
    encoding: PositionEncoding,
    /// The client applies `documentChanges` in a workspace edit.
    document_changes: bool,
    buffers: Buffers,
    /// Each open document's version, as the client numbers them.
    versions: BTreeMap<Document, i32>,
    /// The project folder of each open file.
    documents: BTreeMap<Document, PathBuf>,
    projects: BTreeMap<PathBuf, Project>,
    /// Counts changes across every project: each rebuild takes the next
    /// number as its project's generation.
    generations: u64,
    /// The project folders with a slow check running in the background.
    /// Kept apart from [`Project`] so a check outlives its project being
    /// closed, and a project opened again waits for it.
    checking: BTreeSet<PathBuf>,
    slow_sender: Sender<SlowResult>,
    /// The client sent `shutdown`, so `exit` is expected.
    shut_down: bool,
}

/// The texts of an analysis, for converting its diagnostics.
struct AnalysisSources<'a>(&'a Analysis);

impl Sources for AnalysisSources<'_> {
    fn source(&self, file: FileId) -> Option<(Document, &str)> {
        Some((self.0.document(file)?, self.0.text(file)?))
    }
}

/// The texts a check read, for converting its diagnostics.
struct CheckSources<'a>(&'a SourceMap);

impl Sources for CheckSources<'_> {
    fn source(&self, file: FileId) -> Option<(Document, &str)> {
        let source = self.0.get(file)?;
        Some((Document::from_source_path(source.path()), source.text()))
    }
}

/// A document, the analysis it is loaded in, and its text.
struct Target<'a> {
    analysis: &'a Analysis,
    file: FileId,
    document: Document,
    index: LineIndex<'a>,
}

impl Target<'_> {
    fn position(&self, position: lsp_types::Position, encoding: PositionEncoding) -> FilePosition {
        FilePosition {
            file: self.file,
            offset: self.index.span_offset(position, encoding).unwrap_or(0),
        }
    }
}

impl Server {
    fn main_loop(
        &mut self,
        connection: &Connection,
        slow: &Receiver<SlowResult>,
    ) -> Result<(), ServerError> {
        loop {
            let timer = match self.next_due() {
                Some(due) => crossbeam_channel::at(due),
                None => never(),
            };
            select! {
                recv(connection.receiver) -> message => {
                    let Ok(message) = message else {
                        return Ok(());
                    };
                    if self.handle(connection, message)? == Flow::Exit {
                        return Ok(());
                    }
                }
                recv(slow) -> result => {
                    if let Ok(result) = result {
                        self.slow_finished(result)?;
                    }
                }
                recv(timer) -> _ => self.start_due_checks(),
            }
        }
    }

    fn send(&self, message: impl Into<Message>) -> Result<(), ServerError> {
        self.sender
            .send(message.into())
            .map_err(|_| ServerError::Disconnected)
    }

    fn log(&self, message: String) -> Result<(), ServerError> {
        self.send(Notification::new(
            LogMessage::METHOD.to_owned(),
            LogMessageParams {
                typ: MessageType::WARNING,
                message,
            },
        ))
    }

    fn handle(&mut self, connection: &Connection, message: Message) -> Result<Flow, ServerError> {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    self.shut_down = true;
                    return Ok(Flow::Exit);
                }
                let response = self.respond(request);
                self.send(response)?;
            }
            Message::Notification(notification) => {
                if notification.method == Exit::METHOD {
                    // The protocol asks for an error exit code when the
                    // client exits without shutting the server down.
                    return if self.shut_down {
                        Ok(Flow::Exit)
                    } else {
                        Err(ServerError::ExitWithoutShutdown)
                    };
                }
                self.notified(notification)?;
            }
            Message::Response(_) => {}
        }
        Ok(Flow::Continue)
    }

    // ---- documents ----

    fn notified(&mut self, notification: Notification) -> Result<(), ServerError> {
        let method = notification.method.as_str();
        let params = notification.params;
        match method {
            DidOpenTextDocument::METHOD => {
                if let Ok(params) =
                    serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(params)
                {
                    self.opened(params.text_document)?;
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(params) =
                    serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(params)
                {
                    self.changed(params)?;
                }
            }
            DidSaveTextDocument::METHOD => {
                if let Ok(params) =
                    serde_json::from_value::<lsp_types::DidSaveTextDocumentParams>(params)
                    && let Some(document) = uri_document(&params.text_document.uri)
                {
                    // The client wrote the file: the buffer is clean again.
                    let _ = self.buffers.reload(&document);
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(params) =
                    serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(params)
                {
                    self.closed(&params.text_document.uri)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn opened(&mut self, item: lsp_types::TextDocumentItem) -> Result<(), ServerError> {
        let Some(document) = uri_document(&item.uri) else {
            return Ok(());
        };
        self.buffers.open_with_text(&document, &item.text);
        self.versions.insert(document.clone(), item.version);
        let Document::File(path) = &document else {
            return Ok(());
        };
        let folder = project_folder(path);
        self.documents.insert(document.clone(), folder.clone());
        self.projects.entry(folder.clone()).or_default();
        self.rebuild(&folder)
    }

    fn changed(
        &mut self,
        params: lsp_types::DidChangeTextDocumentParams,
    ) -> Result<(), ServerError> {
        let Some(document) = uri_document(&params.text_document.uri) else {
            return Ok(());
        };
        if document.is_read_only() {
            return self.log(format!(
                "{} is part of the standard library and can't be changed",
                document.source_path()
            ));
        }
        let Some(old) = self.buffers.text(&document) else {
            // The client changed a document it never opened: there is no
            // text to apply the changes to.
            return self.log(format!(
                "{} was changed without being opened first",
                document.source_path()
            ));
        };
        let text = line_index::apply_changes(old, &params.content_changes, self.encoding);
        self.buffers.open_with_text(&document, &text);
        self.versions
            .insert(document.clone(), params.text_document.version);
        let folder = match self.documents.get(&document) {
            Some(folder) => folder.clone(),
            None => {
                let Document::File(path) = &document else {
                    return Ok(());
                };
                let folder = project_folder(path);
                self.documents.insert(document.clone(), folder.clone());
                folder
            }
        };
        self.projects.entry(folder.clone()).or_default();
        self.rebuild(&folder)
    }

    fn closed(&mut self, uri: &Uri) -> Result<(), ServerError> {
        let Some(document) = uri_document(uri) else {
            return Ok(());
        };
        self.buffers.close(&document);
        self.versions.remove(&document);
        let Some(folder) = self.documents.remove(&document) else {
            return Ok(());
        };
        if self.documents.values().any(|open| *open == folder) {
            // The file reads from disk again.
            return self.rebuild(&folder);
        }
        // The last open file of the project: forget it, and clear what
        // was published for it.
        if let Some(project) = self.projects.remove(&folder) {
            for document in project.published.into_keys() {
                self.publish_one(&document, Vec::new())?;
            }
        }
        Ok(())
    }

    /// Loads a project again after a change, publishes its fast
    /// diagnostics, and puts off its slow check until typing pauses.
    fn rebuild(&mut self, folder: &Path) -> Result<(), ServerError> {
        let overlay = self.buffers.overlay();
        let delay = self.config.slow_delay;
        self.generations = self.generations.wrapping_add(1);
        let generation = self.generations;
        let problem = {
            let Some(project) = self.projects.get_mut(folder) else {
                return Ok(());
            };
            let loaded = project.host.analyse(folder, &overlay);
            project.generation = generation;
            project.slow.clear();
            project.due = Some(Instant::now() + delay);
            match loaded {
                Ok(analysis) => {
                    project.analysis = Some(analysis);
                    None
                }
                Err(error) => {
                    project.analysis = None;
                    Some(error.to_string())
                }
            }
        };
        if let Some(problem) = problem {
            self.log(format!("can't load '{}': {problem}", folder.display()))?;
        }
        self.publish(folder)
    }

    // ---- diagnostics ----

    /// Sends a project's diagnostics: the fast tier from its analysis and
    /// the slow tier's for the current text, each file once, only when
    /// they changed.
    fn publish(&mut self, folder: &Path) -> Result<(), ServerError> {
        let encoding = self.encoding;
        let Some(project) = self.projects.get_mut(folder) else {
            return Ok(());
        };
        let mut merged: BTreeMap<Document, Vec<lsp_types::Diagnostic>> = BTreeMap::new();
        if let Some(analysis) = &project.analysis {
            let sources = AnalysisSources(analysis);
            for (file, document) in analysis.project_files() {
                let found = analysis
                    .diagnostics(file)
                    .filter_map(|d| convert::diagnostic(d, &sources, encoding))
                    .map(|(_, shown)| shown);
                merged.entry(document).or_default().extend(found);
            }
        }
        for (document, found) in &project.slow {
            merged
                .entry(document.clone())
                .or_default()
                .extend(found.iter().cloned());
        }
        for found in merged.values_mut() {
            found.sort_by(|a, b| {
                let key = |d: &lsp_types::Diagnostic| {
                    (
                        d.range.start.line,
                        d.range.start.character,
                        d.range.end.line,
                        d.range.end.character,
                    )
                };
                key(a).cmp(&key(b)).then_with(|| a.message.cmp(&b.message))
            });
            found.dedup();
        }
        let gone: Vec<Document> = project
            .published
            .keys()
            .filter(|document| !merged.contains_key(*document))
            .cloned()
            .collect();
        let mut sends = Vec::new();
        for document in gone {
            project.published.remove(&document);
            sends.push((document, Vec::new()));
        }
        for (document, found) in merged {
            if project.published.get(&document) != Some(&found) {
                project.published.insert(document.clone(), found.clone());
                sends.push((document, found));
            }
        }
        for (document, found) in sends {
            self.publish_one(&document, found)?;
        }
        Ok(())
    }

    fn publish_one(
        &self,
        document: &Document,
        diagnostics: Vec<lsp_types::Diagnostic>,
    ) -> Result<(), ServerError> {
        let Some(uri) = document_uri(document) else {
            return Ok(());
        };
        let version = self.versions.get(document).copied();
        self.send(Notification::new(
            PublishDiagnostics::METHOD.to_owned(),
            PublishDiagnosticsParams {
                uri,
                diagnostics,
                version,
            },
        ))
    }

    /// When the next slow check is due, of the projects not checking now.
    fn next_due(&self) -> Option<Instant> {
        self.projects
            .iter()
            .filter(|(folder, _)| !self.checking.contains(*folder))
            .filter_map(|(_, project)| project.due)
            .min()
    }

    /// Starts the slow checks whose time has come, each in its own thread
    /// on a snapshot of the open buffers.
    fn start_due_checks(&mut self) {
        let now = Instant::now();
        let overlay = self.buffers.overlay();
        let encoding = self.encoding;
        for (folder, project) in &mut self.projects {
            if self.checking.contains(folder) || !project.due.is_some_and(|due| due <= now) {
                continue;
            }
            project.due = None;
            self.checking.insert(folder.clone());
            let folder = folder.clone();
            let overlay = overlay.clone();
            let generation = project.generation;
            let sender = self.slow_sender.clone();
            std::thread::spawn(move || {
                let diagnostics = slow_diagnostics(&folder, &overlay, encoding);
                // The server may have stopped; nothing waits for this then.
                let _ = sender.send(SlowResult {
                    folder,
                    generation,
                    diagnostics,
                });
            });
        }
    }

    /// Takes a slow check's result: kept and published if the text has
    /// not changed since it started, dropped otherwise (a newer check is
    /// already due).
    fn slow_finished(&mut self, result: SlowResult) -> Result<(), ServerError> {
        self.checking.remove(&result.folder);
        let Some(project) = self.projects.get_mut(&result.folder) else {
            return Ok(());
        };
        if result.generation != project.generation {
            return Ok(());
        }
        project.slow = result.diagnostics;
        self.publish(&result.folder)
    }

    // ---- requests ----

    fn respond(&mut self, request: Request) -> Response {
        let id = request.id.clone();
        let result = match request.method.as_str() {
            GotoDefinition::METHOD => self.call::<GotoDefinition>(request, Server::definition),
            References::METHOD => self.call::<References>(request, Server::references),
            HoverRequest::METHOD => self.call::<HoverRequest>(request, Server::hover),
            PrepareRenameRequest::METHOD => {
                self.call::<PrepareRenameRequest>(request, Server::prepare_rename)
            }
            Rename::METHOD => self.call::<Rename>(request, Server::rename),
            SemanticTokensFullRequest::METHOD => {
                self.call::<SemanticTokensFullRequest>(request, Server::semantic_tokens)
            }
            Completion::METHOD => self.call::<Completion>(request, Server::completion),
            SignatureHelpRequest::METHOD => {
                self.call::<SignatureHelpRequest>(request, Server::signature_help)
            }
            CodeLensRequest::METHOD => self.call::<CodeLensRequest>(request, Server::code_lens),
            STANDARD_DOCUMENT_REQUEST => standard_document(request.params),
            _ => Err(failure(
                ErrorCode::MethodNotFound,
                format!("stave-lsp doesn't answer '{}'", request.method),
            )),
        };
        Response {
            id,
            response_result: result,
        }
    }

    fn call<R: lsp_types::request::Request>(
        &mut self,
        request: Request,
        handler: fn(&mut Server, R::Params) -> Result<R::Result, ResponseError>,
    ) -> Result<serde_json::Value, ResponseError> {
        let params: R::Params = serde_json::from_value(request.params)
            .map_err(|error| failure(ErrorCode::InvalidParams, error.to_string()))?;
        let result = handler(self, params)?;
        serde_json::to_value(result)
            .map_err(|error| failure(ErrorCode::InternalError, error.to_string()))
    }

    /// The project folder a document's requests are answered from,
    /// loading it if no open file has: the folder of a file on disk, or
    /// any loaded project for a standard file (every project loads the
    /// standard library).
    fn prepare(&mut self, uri: &Uri) -> Option<PathBuf> {
        let document = uri_document(uri)?;
        match &document {
            Document::File(path) => {
                let folder = match self.documents.get(&document) {
                    Some(folder) => folder.clone(),
                    None => project_folder(path),
                };
                if !self.projects.contains_key(&folder) {
                    let mut host = AnalysisHost::new();
                    let analysis = host.analyse(&folder, &self.buffers.overlay()).ok();
                    self.projects.insert(
                        folder.clone(),
                        Project {
                            analysis,
                            host,
                            ..Project::default()
                        },
                    );
                }
                Some(folder)
            }
            Document::Std { .. } => self
                .projects
                .iter()
                .find(|(_, project)| project.analysis.is_some())
                .map(|(folder, _)| folder.clone()),
        }
    }

    fn target(&self, uri: &Uri, folder: &Path) -> Option<Target<'_>> {
        let document = uri_document(uri)?;
        let analysis = self.projects.get(folder)?.analysis.as_ref()?;
        let file = analysis.file(&document)?;
        let text = analysis.text(file)?;
        Some(Target {
            analysis,
            file,
            document,
            index: LineIndex::new(text),
        })
    }

    /// Prepares and finds the document of a request.
    fn locate(&mut self, uri: &Uri) -> Option<Target<'_>> {
        let folder = self.prepare(uri)?;
        self.target(uri, &folder)
    }

    fn definition(
        &mut self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>, ResponseError> {
        let encoding = self.encoding;
        let position = params.text_document_position_params;
        let Some(target) = self.locate(&position.text_document.uri) else {
            return Ok(None);
        };
        let at = target.position(position.position, encoding);
        let found = target.analysis.definition(at);
        Ok(found
            .and_then(|found| location(target.analysis, &found, encoding))
            .map(GotoDefinitionResponse::Scalar))
    }

    fn references(
        &mut self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>, ResponseError> {
        let encoding = self.encoding;
        let position = params.text_document_position;
        let Some(target) = self.locate(&position.text_document.uri) else {
            return Ok(None);
        };
        let at = target.position(position.position, encoding);
        let Some(uses) = target.analysis.uses_at(at) else {
            return Ok(None);
        };
        Ok(Some(
            uses.uses
                .iter()
                .filter(|found| {
                    params.context.include_declaration || found.kind != stave_ide::UseKind::Declared
                })
                .filter_map(|found| location(target.analysis, &found.location, encoding))
                .collect(),
        ))
    }

    fn hover(&mut self, params: HoverParams) -> Result<Option<lsp_types::Hover>, ResponseError> {
        let encoding = self.encoding;
        let position = params.text_document_position_params;
        let Some(target) = self.locate(&position.text_document.uri) else {
            return Ok(None);
        };
        let at = target.position(position.position, encoding);
        Ok(target
            .analysis
            .hover(at)
            .map(|found| convert::hover(&found, &target.index, encoding)))
    }

    fn prepare_rename(
        &mut self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>, ResponseError> {
        let encoding = self.encoding;
        let Some(target) = self.locate(&params.text_document.uri) else {
            return Ok(None);
        };
        let at = target.position(params.position, encoding);
        let Some(uses) = target.analysis.uses_at(at) else {
            return Ok(None);
        };
        if !target.analysis.is_in_project(uses.symbol) {
            return Err(failure(
                ErrorCode::RequestFailed,
                format!(
                    "'{}' is declared outside this project, so it can't be renamed here",
                    uses.name
                ),
            ));
        }
        let span = uses
            .uses
            .iter()
            .map(|found| &found.location)
            .find(|found| {
                found.file == target.file
                    && found.span.start() <= at.offset
                    && at.offset <= found.span.end()
            })
            .map(|found| found.span);
        Ok(span
            .and_then(|span| target.index.range(span, encoding))
            .map(|range| PrepareRenameResponse::RangeWithPlaceholder {
                range,
                placeholder: uses.name.clone(),
            }))
    }

    fn rename(&mut self, params: RenameParams) -> Result<Option<WorkspaceEdit>, ResponseError> {
        let encoding = self.encoding;
        let document_changes = self.document_changes;
        let position = params.text_document_position;
        let Some(folder) = self.prepare(&position.text_document.uri) else {
            return Ok(None);
        };
        let Some(target) = self.target(&position.text_document.uri, &folder) else {
            return Ok(None);
        };
        let at = target.position(position.position, encoding);
        let Some(symbol) = target.analysis.symbol_at(at) else {
            return Ok(None);
        };
        let renamed = target
            .analysis
            .rename(symbol, &params.new_name)
            .map_err(|error| failure(ErrorCode::RequestFailed, error.to_string()))?;
        let mut files: Vec<(Uri, Option<i32>, Vec<TextEdit>)> = Vec::new();
        for file in &renamed {
            let Some(text) = target.analysis.text(file.file) else {
                continue;
            };
            let Some(uri) = document_uri(&file.document) else {
                continue;
            };
            let index = LineIndex::new(text);
            let edits = file
                .edits
                .iter()
                .filter_map(|edit| {
                    let start = u32::try_from(edit.range.start).ok()?;
                    let end = u32::try_from(edit.range.end).ok()?;
                    Some(TextEdit::new(
                        index.range(Span::new(start, end), encoding)?,
                        edit.text.clone(),
                    ))
                })
                .collect();
            let version = self.versions.get(&file.document).copied();
            files.push((uri, version, edits));
        }
        if document_changes {
            let edits = files
                .into_iter()
                .map(|(uri, version, edits)| TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier { uri, version },
                    edits: edits.into_iter().map(OneOf::Left).collect(),
                })
                .collect();
            Ok(Some(WorkspaceEdit {
                changes: None,
                document_changes: Some(DocumentChanges::Edits(edits)),
                change_annotations: None,
            }))
        } else {
            #[expect(
                clippy::mutable_key_type,
                reason = "lsp-types' WorkspaceEdit takes a map keyed by Uri, whose order and hash use only its text"
            )]
            let changes: HashMap<Uri, Vec<TextEdit>> = files
                .into_iter()
                .map(|(uri, _, edits)| (uri, edits))
                .collect();
            Ok(Some(WorkspaceEdit::new(changes)))
        }
    }

    fn semantic_tokens(
        &mut self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>, ResponseError> {
        let encoding = self.encoding;
        let Some(target) = self.locate(&params.text_document.uri) else {
            return Ok(None);
        };
        let tokens = target.analysis.tokens(target.file);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: legend::encode(&tokens, &target.index, encoding),
        })))
    }

    fn completion(
        &mut self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>, ResponseError> {
        let encoding = self.encoding;
        let position = params.text_document_position;
        let Some(target) = self.locate(&position.text_document.uri) else {
            return Ok(None);
        };
        let at = target.position(position.position, encoding);
        let found = target.analysis.completions(at);
        Ok(Some(CompletionResponse::Array(convert::completions(
            &found,
            &target.index,
            encoding,
        ))))
    }

    fn signature_help(
        &mut self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>, ResponseError> {
        let encoding = self.encoding;
        let position = params.text_document_position_params;
        let Some(target) = self.locate(&position.text_document.uri) else {
            return Ok(None);
        };
        let at = target.position(position.position, encoding);
        Ok(target
            .analysis
            .signature_help(at)
            .map(|help| convert::signature_help(&help)))
    }

    fn code_lens(
        &mut self,
        params: CodeLensParams,
    ) -> Result<Option<Vec<CodeLens>>, ResponseError> {
        let encoding = self.encoding;
        let Some(target) = self.locate(&params.text_document.uri) else {
            return Ok(None);
        };
        let lenses = target.analysis.lenses(target.file);
        Ok(Some(convert::code_lenses(
            &lenses,
            &target.document,
            &target.index,
            encoding,
        )))
    }
}

/// A location as the protocol gives it.
fn location(
    analysis: &Analysis,
    found: &stave_ide::Location,
    encoding: PositionEncoding,
) -> Option<Location> {
    let text = analysis.text(found.file)?;
    Some(Location::new(
        document_uri(&found.document)?,
        LineIndex::new(text).range(found.span, encoding)?,
    ))
}

/// The text of a standard library document, for an editor that can't
/// read `stave-std:` URIs itself: `{"uri": "stave-std:///joinery/dado.stave"}`
/// gives `{"text": "…"}`.
fn standard_document(params: serde_json::Value) -> Result<serde_json::Value, ResponseError> {
    let uri = params
        .get("uri")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            failure(
                ErrorCode::InvalidParams,
                "expected {\"uri\": \"stave-std:///…\"}",
            )
        })?;
    let document = Document::from_uri(uri)
        .filter(Document::is_read_only)
        .ok_or_else(|| {
            failure(
                ErrorCode::InvalidParams,
                format!("'{uri}' is not a standard library file"),
            )
        })?;
    let mut buffers = Buffers::new();
    let text = buffers
        .open(&document)
        .map_err(|error| failure(ErrorCode::InternalError, error.to_string()))?;
    Ok(serde_json::json!({ "text": text }))
}

/// The slow tier: the whole pipeline on a snapshot of the buffers, its
/// diagnostics by document. A folder that can't be loaded gives none
/// (the fast tier already says why).
fn slow_diagnostics(
    folder: &Path,
    overlay: &Overlay,
    encoding: PositionEncoding,
) -> BTreeMap<Document, Vec<lsp_types::Diagnostic>> {
    let mut diagnostics: BTreeMap<Document, Vec<lsp_types::Diagnostic>> = BTreeMap::new();
    let Ok(opened) = stave_app::open_with(folder, overlay) else {
        return diagnostics;
    };
    let sources = CheckSources(&opened.sources);
    for found in &opened.diagnostics {
        if let Some((document, shown)) = convert::diagnostic(found, &sources, encoding) {
            diagnostics.entry(document).or_default().push(shown);
        }
    }
    diagnostics
}

/// The project a file belongs to: the nearest folder at or above the
/// file's own (up to [`PROJECT_SEARCH_DEPTH`] above it) whose `.stave`
/// files declare a `project`, so a library folder inside a project
/// (`hardware/`) is answered as part of it; the file's own folder when
/// none does.
pub(crate) fn project_folder(file: &Path) -> PathBuf {
    let own = file.parent().map(Path::to_path_buf).unwrap_or_default();
    own.ancestors()
        .take(PROJECT_SEARCH_DEPTH.saturating_add(1))
        .find(|folder| declares_project(folder))
        .map_or(own.clone(), Path::to_path_buf)
}

/// `true` if a `.stave` file directly in `folder` has a line starting
/// with the `project` keyword (every item starts with its keyword on a
/// new line).
fn declares_project(folder: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.extension().is_some_and(|e| e == "stave")
            && std::fs::read_to_string(&path).is_ok_and(|text| {
                text.lines().any(|line| {
                    line.strip_prefix("project")
                        .is_some_and(|rest| rest.starts_with([' ', '\t', '"']))
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name)
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn a_library_folder_inside_a_project_belongs_to_it() {
        let cabinet = example("clothing_cabinet");
        assert_eq!(project_folder(&cabinet.join("doors.stave")), cabinet);
        assert_eq!(
            project_folder(&cabinet.join("hardware/handles.stave")),
            cabinet
        );
    }

    fn server() -> (Server, Receiver<Message>) {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let (slow_sender, _) = crossbeam_channel::unbounded();
        let server = Server {
            sender,
            config: Config::default(),
            encoding: PositionEncoding::Utf16,
            document_changes: false,
            buffers: Buffers::new(),
            versions: BTreeMap::new(),
            documents: BTreeMap::new(),
            projects: BTreeMap::new(),
            generations: 0,
            checking: BTreeSet::new(),
            slow_sender,
            shut_down: false,
        };
        (server, receiver)
    }

    fn uri(path: &Path) -> Uri {
        document_uri(&Document::File(path.to_path_buf())).unwrap()
    }

    #[test]
    fn a_check_started_before_a_project_closed_is_dropped_when_it_reopens() {
        let cabinet = example("clothing_cabinet");
        let doors = cabinet.join("doors.stave");
        let text = std::fs::read_to_string(&doors).unwrap();
        let (mut server, _messages) = server();
        let item = |version| lsp_types::TextDocumentItem {
            uri: uri(&doors),
            language_id: "stave".to_owned(),
            version,
            text: text.clone(),
        };
        server.opened(item(1)).unwrap();
        let first = server.projects[&cabinet].generation;
        // A check of the first text starts, then the file is closed and
        // opened again while it runs.
        server.checking.insert(cabinet.clone());
        server.closed(&uri(&doors)).unwrap();
        server.opened(item(2)).unwrap();
        assert_ne!(server.projects[&cabinet].generation, first);
        // The reopened project waits for the running check.
        assert_eq!(server.next_due(), None);
        let stale = lsp_types::Diagnostic {
            message: "stale".to_owned(),
            ..lsp_types::Diagnostic::default()
        };
        server
            .slow_finished(SlowResult {
                folder: cabinet.clone(),
                generation: first,
                diagnostics: BTreeMap::from([(Document::File(doors.clone()), vec![stale])]),
            })
            .unwrap();
        assert!(server.projects[&cabinet].slow.is_empty());
        assert!(!server.checking.contains(&cabinet));
        assert!(server.next_due().is_some());
    }

    #[test]
    fn a_change_to_a_document_never_opened_is_ignored() {
        let doors = example("clothing_cabinet").join("doors.stave");
        let (mut server, _messages) = server();
        server
            .changed(lsp_types::DidChangeTextDocumentParams {
                text_document: lsp_types::VersionedTextDocumentIdentifier {
                    uri: uri(&doors),
                    version: 2,
                },
                content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: "stave 1\n".to_owned(),
                }],
            })
            .unwrap();
        assert!(server.buffers.text(&Document::File(doors)).is_none());
        assert!(server.projects.is_empty());
    }

    #[test]
    fn standard_documents_are_served_read_only() {
        let text = standard_document(serde_json::json!({
            "uri": "stave-std:///joinery/dado.stave"
        }))
        .unwrap();
        assert!(text["text"].as_str().unwrap().contains("kind dado"));
        let refused = standard_document(serde_json::json!({ "uri": "file:///a.stave" }));
        assert!(refused.is_err());
    }
}
