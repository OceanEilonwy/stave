//! The application's link to AI apps (`docs/gui.md`, "Starting Stave");
//! named `ai_link` beside `stave_app::link`, the links between items:
//!
//! - **Show a project** (`app.show-project`, parameter `(ss)`: a folder,
//!   and a patch file's text or `""`). GApplication answers it on the
//!   session bus (`org.freedesktop.Application.ActivateAction`), which is
//!   how an AI app's `show_in_stave` (`stave_link::show_in_stave`) reaches
//!   the running Stave: the folder opens (or its window comes forward)
//!   and the patch is tried and shown as a ghost, a change in progress
//!   like any other, written only if the user adds it.
//! - **Use with an AI App…** (Help menu, `app.use-with-ai-app`): sets
//!   Stave up in every AI app found (`stave_link::install`) off the main
//!   thread, then says what was done in a dialog.

use stave_app::i18n::{fill_in, tr};
use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::{gio, glib};
use stave_link::{Host, Installed, LinkError, Places};

/// The action Use with an AI App… runs.
pub(super) const USE_WITH_AI_APP: &str = "use-with-ai-app";

/// What `app.show-project` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShowProject {
    /// The project folder: an absolute path.
    pub(super) folder: PathBuf,
    /// The patch file's text to show as a ghost, if any.
    pub(super) patch: Option<String>,
}

/// The request in the action's parameter: `None` for a parameter that
/// isn't `(ss)`, or a folder that isn't absolute (it would be read from
/// wherever Stave happened to start).
pub(super) fn show_project_request(parameter: Option<&glib::Variant>) -> Option<ShowProject> {
    let (folder, patch) = parameter?.get::<(String, String)>()?;
    let folder = PathBuf::from(folder);
    if !folder.is_absolute() {
        return None;
    }
    let patch = (!patch.trim().is_empty()).then_some(patch);
    Some(ShowProject { folder, patch })
}

/// Adds `app.show-project` to the application: each request that reads
/// goes to `requested`.
pub(super) fn add_show_project_action(
    application: &impl IsA<gio::ActionMap>,
    requested: impl Fn(ShowProject) + 'static,
) {
    let parameter = <(String, String) as glib::variant::StaticVariantType>::static_variant_type();
    let action = gio::SimpleAction::new(stave_link::SHOW_PROJECT_ACTION, Some(&parameter));
    action.connect_activate(move |_, parameter| match show_project_request(parameter) {
        Some(request) => requested(request),
        None => eprintln!("warning: a request to show a project that doesn't read was ignored"),
    });
    application.add_action(&action);
}

/// What Use with an AI App… did.
#[derive(Debug)]
pub(super) enum Installing {
    /// No AI app was found.
    NoApp,
    /// The `stave` program the apps would run wasn't found.
    NoProgram,
    /// Each app found, and what setting it up did.
    Done(Vec<(Host, Result<Installed, LinkError>)>),
}

/// Sets Stave up in every AI app found. Blocking (it can run `claude`):
/// call it off the main thread.
pub(super) fn install_found() -> Installing {
    let hosts = stave_link::found_hosts();
    if hosts.is_empty() {
        return Installing::NoApp;
    }
    let Some(program) = stave_program() else {
        return Installing::NoProgram;
    };
    let results = stave_link::install(&hosts, &program);
    for (host, result) in hosts.iter().zip(&results) {
        if let Err(error) = result {
            eprintln!("error: setting Stave up in {host}: {error}");
        }
    }
    Installing::Done(hosts.into_iter().zip(results).collect())
}

/// The `stave` program: beside this one (as installed, and as Cargo
/// builds them), or else on `PATH`.
fn stave_program() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|window| window.canonicalize().ok())
        .and_then(|window| window.parent().map(|folder| folder.join("stave")));
    if let Some(beside) = beside
        && is_program(&beside)
    {
        return Some(beside);
    }
    Places::from_environment().ok()?.find_program("stave")
}

/// `true` for an executable file.
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// What the dialog says: a heading and a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Report {
    /// The heading.
    pub(super) heading: String,
    /// The body, a line per app.
    pub(super) body: String,
}

/// The dialog's words for what was done.
pub(super) fn report(installing: &Installing) -> Report {
    match installing {
        Installing::NoApp => Report {
            heading: tr("No AI app found"),
            body: tr(
                "Stave works with Claude Desktop, Claude Code and Codex. Install one of them, then choose Use with an AI App… again.",
            ),
        },
        Installing::NoProgram => Report {
            heading: tr("Stave couldn't be set up"),
            body: tr(
                "The AI apps start Stave's command-line program, stave, and it wasn't found beside this window or on your PATH. Install it with Stave, then try again.",
            ),
        },
        Installing::Done(results) => {
            let lines: Vec<String> = results
                .iter()
                .map(|(host, result)| match result {
                    Ok(_) => fill_in(
                        &tr("{app}: ready. Start {app} again to use Stave there."),
                        &[("app", host)],
                    ),
                    Err(error) => fill_in(
                        &tr("{app}: not set up. {reason}"),
                        &[("app", host), ("reason", &reason(*host, error))],
                    ),
                })
                .collect();
            let all_done = results.iter().all(|(_, result)| result.is_ok());
            Report {
                heading: if all_done {
                    tr("Stave is ready in your AI apps")
                } else {
                    tr("Stave couldn't be set up everywhere")
                },
                body: lines.join("\n"),
            }
        }
    }
}

/// Why an app couldn't be set up, in the window's plain, translated
/// words: never the commands run or the settings' own key names (the
/// full error goes to standard error, from [`install_found`]).
fn reason(host: Host, error: &LinkError) -> String {
    let app: &dyn std::fmt::Display = &host;
    match error {
        LinkError::NoHome => fill_in(
            &tr("Stave can't find your home folder, so it can't find {app}'s settings."),
            &[("app", app)],
        ),
        LinkError::NotUtf8(_) => tr(
            "Stave is installed in a folder whose name can't be written into the app's settings.",
        ),
        LinkError::NotAbsolute(_) => {
            tr("Stave couldn't find the full path to its own program to give the app.")
        }
        LinkError::Read { path, .. } => fill_in(
            &tr("Its settings file couldn't be read ({path})."),
            &[("path", &path.display())],
        ),
        LinkError::Write { path, .. } => fill_in(
            &tr("Its settings file couldn't be written ({path})."),
            &[("path", &path.display())],
        ),
        LinkError::Settings { path, .. } => fill_in(
            &tr(
                "Its settings file isn't laid out the way {app} writes it, so it was left as it is ({path}).",
            ),
            &[("app", app), ("path", &path.display())],
        ),
        LinkError::NoCommand { program, .. } => fill_in(
            &tr("Its command, {program}, wasn't found. Is {app} installed?"),
            &[("program", program), ("app", app)],
        ),
        LinkError::Run { .. } | LinkError::Failed { .. } => {
            fill_in(&tr("{app}'s own setup command failed."), &[("app", app)])
        }
        LinkError::Folder { .. } | LinkError::NoSessionBus(_) | LinkError::Bus(_) => {
            fill_in(&tr("Stave couldn't reach {app}."), &[("app", app)])
        }
    }
}

/// Shows the report in a dialog over `parent`.
pub(super) fn present(report: &Report, parent: Option<&gtk::Window>) {
    let dialog = adw::AlertDialog::new(Some(&report.heading), Some(&report.body));
    dialog.add_response("close", &tr("OK"));
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.present(parent);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameter(folder: &str, patch: &str) -> glib::Variant {
        (folder.to_owned(), patch.to_owned()).to_variant()
    }

    #[test]
    fn the_action_takes_what_stave_link_sends() {
        let parameter =
            <(String, String) as glib::variant::StaticVariantType>::static_variant_type();
        assert_eq!(parameter.as_str(), stave_link::SHOW_PROJECT_PARAMETER);
        assert_eq!(super::super::app::APP_ID, stave_link::APPLICATION_ID);
        assert_eq!(
            stave_link::OBJECT_PATH,
            format!("/{}", stave_link::APPLICATION_ID.replace('.', "/"))
        );
    }

    #[test]
    fn a_request_reads_its_folder_and_patch() {
        let patch = "stave 1\n\npatch \"Wider\" {\n  set width = 900mm\n}\n";
        assert_eq!(
            show_project_request(Some(&parameter("/home/me/chest", patch))),
            Some(ShowProject {
                folder: PathBuf::from("/home/me/chest"),
                patch: Some(patch.to_owned()),
            })
        );
        assert_eq!(
            show_project_request(Some(&parameter("/home/me/chest", ""))),
            Some(ShowProject {
                folder: PathBuf::from("/home/me/chest"),
                patch: None,
            })
        );
        assert_eq!(
            show_project_request(Some(&parameter("chest", ""))),
            None,
            "a relative folder"
        );
        assert_eq!(show_project_request(Some(&"/home".to_variant())), None);
        assert_eq!(show_project_request(None), None);
    }

    #[test]
    fn the_report_says_what_was_done_in_each_app() {
        let done = Installing::Done(vec![
            (
                Host::ClaudeDesktop,
                Ok(Installed {
                    host: Host::ClaudeDesktop,
                    how: String::new(),
                }),
            ),
            (
                Host::ClaudeCode,
                Err(LinkError::NoCommand {
                    host: Host::ClaudeCode,
                    program: "claude",
                }),
            ),
        ]);
        let report = report(&done);
        assert_eq!(report.heading, "Stave couldn't be set up everywhere");
        let lines: Vec<&str> = report.body.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("Claude Desktop: ready."),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].starts_with("Claude Code: not set up."),
            "{}",
            lines[1]
        );
        assert_eq!(super::report(&Installing::NoApp).heading, "No AI app found");
        // The window never names the protocol.
        for installing in [Installing::NoApp, Installing::NoProgram, done] {
            let report = super::report(&installing);
            assert!(!report.body.contains("MCP") && !report.heading.contains("MCP"));
        }
    }

    /// Errors whose own text names the commands run or the settings' key
    /// names come out in plain words, with no "mcp" in any case.
    #[test]
    fn the_report_puts_every_error_in_plain_words() {
        let failed = |host: Host, error: LinkError| (host, Err(error));
        let done = Installing::Done(vec![
            failed(
                Host::ClaudeCode,
                LinkError::Failed {
                    command: "claude mcp add --scope user stave -- /usr/bin/stave mcp".to_owned(),
                    output: "mcp add: already exists".to_owned(),
                },
            ),
            failed(
                Host::ClaudeDesktop,
                LinkError::Settings {
                    path: PathBuf::from("/home/me/.config/Claude/claude_desktop_config.json"),
                    message: "its mcpServers isn't a JSON object".to_owned(),
                },
            ),
            failed(
                Host::Codex,
                LinkError::Settings {
                    path: PathBuf::from("/home/me/.codex/config.toml"),
                    message: "its mcp_servers isn't a table".to_owned(),
                },
            ),
            failed(Host::Codex, LinkError::NoHome),
            failed(
                Host::ClaudeCode,
                LinkError::NoCommand {
                    host: Host::ClaudeCode,
                    program: "claude",
                },
            ),
        ]);
        let report = report(&done);
        let lines: Vec<&str> = report.body.lines().collect();
        assert_eq!(lines.len(), 5, "{}", report.body);
        for line in &lines {
            assert!(
                !line.to_lowercase().contains("mcp"),
                "names the protocol: {line}"
            );
            assert!(line.contains(": not set up. "), "{line}");
        }
        assert_eq!(
            lines[0],
            "Claude Code: not set up. Claude Code's own setup command failed."
        );
        assert!(lines[1].contains("left as it is"), "{}", lines[1]);
        assert!(
            lines[1].contains("claude_desktop_config.json"),
            "{}",
            lines[1]
        );
        assert!(lines[2].contains("config.toml"), "{}", lines[2]);
        assert!(lines[3].contains("home folder"), "{}", lines[3]);
        assert!(lines[4].contains("claude"), "{}", lines[4]);
    }

    #[test]
    fn a_request_over_the_session_bus_reaches_the_action() {
        crate::ui::gtk_test::on_gtk_thread(request_over_the_bus);
    }

    /// A stand-in application with its own name on the session bus gets
    /// the request `stave_link` sends, through the same action wiring.
    fn request_over_the_bus() {
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::time::{Duration, Instant};

        let id = format!(
            "io.github.OceanEilonwy.StaveTest.Link{}",
            std::process::id()
        );
        let path = format!("/{}", id.replace('.', "/"));
        let application = gio::Application::new(Some(&id), gio::ApplicationFlags::empty());
        let got: Rc<RefCell<Option<ShowProject>>> = Rc::default();
        let into = Rc::clone(&got);
        add_show_project_action(&application, move |request| {
            *into.borrow_mut() = Some(request);
        });
        if let Err(error) = application.register(None::<&gio::Cancellable>) {
            eprintln!("skipped: no session bus to register on ({error})");
            return;
        }
        let patch = "stave 1\n\npatch \"Wider\" {\n  set width = 900mm\n}\n";
        let sent = std::thread::spawn({
            let id = id.clone();
            move || {
                pollster::block_on(stave_link::show_in_application(
                    &id,
                    &path,
                    Path::new("/home/me/chest"),
                    Some(patch),
                ))
                .map_err(|error| error.to_string())
            }
        });
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(10);
        while (got.borrow().is_none() || !sent.is_finished()) && Instant::now() < deadline {
            if !context.iteration(false) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let shown = sent.join().unwrap();
        assert_eq!(shown, Ok(stave_link::Shown::Opened));
        assert_eq!(
            got.borrow().clone(),
            Some(ShowProject {
                folder: PathBuf::from("/home/me/chest"),
                patch: Some(patch.to_owned()),
            })
        );
    }
}
