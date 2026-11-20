//! Claude Desktop: the server entry `mcpServers.stave` in its settings
//! file, `~/.config/Claude/claude_desktop_config.json`. The file is read
//! as JSON and written back with every other key kept (in sorted order,
//! two-space indented).

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::{Host, Installed, LinkError, Places, SERVER_ARGUMENT, SERVER_NAME, files, utf8};

/// The key holding the servers.
const SERVERS: &str = "mcpServers";

/// Adds (or updates) Stave's entry.
pub(crate) fn install(places: &Places, stave: &Path) -> Result<Installed, LinkError> {
    let path = places.claude_desktop_config();
    let mut settings = read(&path)?;
    let entry = json!({ "command": utf8(stave)?, "args": [SERVER_ARGUMENT] });
    let servers = servers(&mut settings, &path)?;
    let where_ = path.display();
    if servers.get(SERVER_NAME) == Some(&entry) {
        return Ok(Installed {
            host: Host::ClaudeDesktop,
            how: format!("Stave was already set up in Claude Desktop's settings ({where_})"),
        });
    }
    servers.insert(SERVER_NAME.to_owned(), entry);
    write(&path, &settings)?;
    Ok(Installed {
        host: Host::ClaudeDesktop,
        how: format!(
            "added Stave to Claude Desktop's settings ({where_}); restart Claude Desktop to use it"
        ),
    })
}

/// Removes Stave's entry, and nothing else.
pub(crate) fn uninstall(places: &Places) -> Result<Installed, LinkError> {
    let path = places.claude_desktop_config();
    let where_ = path.display();
    let absent = || Installed {
        host: Host::ClaudeDesktop,
        how: format!("Stave wasn't set up in Claude Desktop's settings ({where_})"),
    };
    if files::read(&path)?.is_none() {
        return Ok(absent());
    }
    let mut settings = read(&path)?;
    let removed = servers(&mut settings, &path)?.remove(SERVER_NAME).is_some();
    if !removed {
        return Ok(absent());
    }
    write(&path, &settings)?;
    Ok(Installed {
        host: Host::ClaudeDesktop,
        how: format!(
            "removed Stave from Claude Desktop's settings ({where_}); restart Claude Desktop"
        ),
    })
}

/// The settings as a JSON object: an empty one for a missing (or empty)
/// file.
fn read(path: &Path) -> Result<Value, LinkError> {
    let text = files::read(path)?.unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let settings: Value = serde_json::from_str(&text).map_err(|error| LinkError::Settings {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if !settings.is_object() {
        return Err(LinkError::Settings {
            path: path.to_path_buf(),
            message: "it isn't a JSON object".to_owned(),
        });
    }
    Ok(settings)
}

/// The servers object, made when missing.
fn servers<'s>(
    settings: &'s mut Value,
    path: &Path,
) -> Result<&'s mut Map<String, Value>, LinkError> {
    let not_settings = |message: &str| LinkError::Settings {
        path: path.to_path_buf(),
        message: message.to_owned(),
    };
    settings
        .as_object_mut()
        .ok_or_else(|| not_settings("it isn't a JSON object"))?
        .entry(SERVERS)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| not_settings("its mcpServers isn't a JSON object"))
}

/// Writes the settings back, pretty, ending with a newline.
fn write(path: &Path, settings: &Value) -> Result<(), LinkError> {
    let mut text = serde_json::to_string_pretty(settings).map_err(|error| LinkError::Settings {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    text.push('\n');
    files::write(path, &text)
}
