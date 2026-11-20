//! "Show in Stave": asks the Stave window, if it is running, to open a
//! project folder and show a patch on it as a ghost (never written); and
//! whether the window is running at all.
//!
//! The window is a GApplication, which answers
//! `org.freedesktop.Application.ActivateAction` at its object path on the
//! session bus. Its application action [`SHOW_PROJECT_ACTION`] takes
//! [`SHOW_PROJECT_PARAMETER`]: the folder, and the patch file's text or
//! an empty string. Stave is asked only when its name has an owner, so a
//! Stave that isn't running is never started by the bus.

use std::collections::HashMap;
use std::path::Path;

use zbus::Connection;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::zvariant::{StructureBuilder, Value};

use crate::LinkError;

/// The window's application id, and its name on the session bus.
pub const APPLICATION_ID: &str = "io.github.OceanEilonwy.Stave";

/// The window's object path on the session bus.
pub const OBJECT_PATH: &str = "/io/github/OceanEilonwy/Stave";

/// The application action that opens a project and shows a patch.
pub const SHOW_PROJECT_ACTION: &str = "show-project";

/// The action's parameter type: the folder, and the patch text or `""`.
pub const SHOW_PROJECT_PARAMETER: &str = "(ss)";

/// The interface GApplication answers activation on.
const APPLICATION_INTERFACE: &str = "org.freedesktop.Application";

/// What asking the window did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// The window took the request: it opens the project (and shows the
    /// patch as a ghost).
    Opened,
    /// Stave isn't running.
    NotRunning,
}

/// Asks the running Stave window to open `folder` (an absolute path)
/// and, given a patch file's text, to show it on the design as a ghost.
///
/// # Errors
///
/// The folder isn't absolute or isn't UTF-8 ([`LinkError::Folder`]),
/// there is no session bus ([`LinkError::NoSessionBus`]), or the window
/// refused the call ([`LinkError::Bus`]).
pub async fn show_in_stave(folder: &Path, patch: Option<&str>) -> Result<Shown, LinkError> {
    show_in_application(APPLICATION_ID, OBJECT_PATH, folder, patch).await
}

/// [`show_in_stave`] for the application with this bus name and object
/// path: another build of Stave, or a test's stand-in.
///
/// # Errors
///
/// As [`show_in_stave`].
pub async fn show_in_application(
    name: &str,
    path: &str,
    folder: &Path,
    patch: Option<&str>,
) -> Result<Shown, LinkError> {
    if !folder.is_absolute() {
        return Err(LinkError::Folder {
            folder: folder.to_path_buf(),
            reason: "it isn't an absolute path",
        });
    }
    let Some(folder_text) = folder.to_str() else {
        return Err(LinkError::Folder {
            folder: folder.to_path_buf(),
            reason: "it isn't UTF-8",
        });
    };
    let connection = Connection::session()
        .await
        .map_err(LinkError::NoSessionBus)?;
    let bus_name = BusName::try_from(name).map_err(zbus::Error::from)?;
    if !has_owner(&connection, &bus_name).await? {
        return Ok(Shown::NotRunning);
    }
    let parameter = StructureBuilder::new()
        .add_field(folder_text)
        .add_field(patch.unwrap_or_default())
        .build()
        .map_err(zbus::Error::from)?;
    let platform_data: HashMap<&str, Value<'_>> = HashMap::new();
    let body = (
        SHOW_PROJECT_ACTION,
        vec![Value::from(parameter)],
        platform_data,
    );
    match connection
        .call_method(
            Some(bus_name),
            path,
            Some(APPLICATION_INTERFACE),
            "ActivateAction",
            &body,
        )
        .await
    {
        Ok(_) => Ok(Shown::Opened),
        // It quit between the question and the call.
        Err(zbus::Error::FDO(error))
            if matches!(
                *error,
                zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
            ) =>
        {
            Ok(Shown::NotRunning)
        }
        Err(error) => Err(error.into()),
    }
}

/// Whether the Stave window is running: whether its name on the session
/// bus has an owner. A desktop with no session bus has no window running.
///
/// # Errors
///
/// [`LinkError::Bus`] when the bus couldn't be asked.
pub async fn stave_is_running() -> Result<bool, LinkError> {
    application_is_running(APPLICATION_ID).await
}

/// [`stave_is_running`] for the application with this bus name: another
/// build of Stave, or a test's stand-in.
///
/// # Errors
///
/// As [`stave_is_running`].
pub async fn application_is_running(name: &str) -> Result<bool, LinkError> {
    let connection = match Connection::session().await {
        Ok(connection) => connection,
        // No session bus: nothing can be running on it.
        Err(_) => return Ok(false),
    };
    let bus_name = BusName::try_from(name).map_err(zbus::Error::from)?;
    has_owner(&connection, &bus_name).await
}

/// Whether `name` has an owner on the bus `connection` talks to.
async fn has_owner(connection: &Connection, name: &BusName<'_>) -> Result<bool, LinkError> {
    let bus = DBusProxy::new(connection).await?;
    Ok(bus
        .name_has_owner(name.clone())
        .await
        .map_err(zbus::Error::from)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_in_stave_reports_not_running() {
        // A name nobody owns, so a Stave running on this machine isn't
        // asked anything.
        let shown = pollster::block_on(show_in_application(
            "io.github.OceanEilonwy.StaveTest.NobodyOwnsThis",
            OBJECT_PATH,
            Path::new("/tmp/project"),
            Some("stave 1\n"),
        ));
        match shown {
            Ok(shown) => assert_eq!(shown, Shown::NotRunning),
            Err(LinkError::NoSessionBus(error)) => {
                eprintln!("no session bus, skipped: {error}");
            }
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn a_name_nobody_owns_is_not_running() {
        let running = pollster::block_on(application_is_running(
            "io.github.OceanEilonwy.StaveTest.NobodyOwnsThis",
        ));
        assert!(matches!(running, Ok(false)), "{running:?}");
    }

    #[test]
    fn a_relative_folder_is_refused_before_the_bus() {
        let shown = pollster::block_on(show_in_stave(Path::new("project"), None));
        assert!(matches!(shown, Err(LinkError::Folder { .. })), "{shown:?}");
    }
}
