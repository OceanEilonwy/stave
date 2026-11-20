//! Codex: the table `[mcp_servers.stave]` in its settings file,
//! `~/.codex/config.toml`, edited in place with `toml_edit` so other
//! tables, their order and comments stay as they were.

use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table, Value, value};

use crate::{Host, Installed, LinkError, Places, SERVER_ARGUMENT, SERVER_NAME, files, utf8};

/// The table holding the servers.
const SERVERS: &str = "mcp_servers";

/// Adds (or updates) Stave's table.
pub(crate) fn install(places: &Places, stave: &Path) -> Result<Installed, LinkError> {
    let path = places.codex_config();
    let text = files::read(&path)?.unwrap_or_default();
    let mut document = parse(&path, &text)?;
    let servers = servers(&mut document, &path)?;
    let command = utf8(stave)?;
    let where_ = path.display();
    if is_entry(servers.get(SERVER_NAME), command) {
        return Ok(Installed {
            host: Host::Codex,
            how: format!("Stave was already set up in Codex's settings ({where_})"),
        });
    }
    let mut entry = Table::new();
    entry.insert("command", value(command));
    let mut arguments = Array::new();
    arguments.push(SERVER_ARGUMENT);
    entry.insert("args", value(arguments));
    servers.insert(SERVER_NAME, Item::Table(entry));
    files::write(&path, &document.to_string())?;
    Ok(Installed {
        host: Host::Codex,
        how: format!("added Stave to Codex's settings ({where_}); start Codex again to use it"),
    })
}

/// Removes Stave's table, and nothing else.
pub(crate) fn uninstall(places: &Places) -> Result<Installed, LinkError> {
    let path = places.codex_config();
    let where_ = path.display();
    let absent = Installed {
        host: Host::Codex,
        how: format!("Stave wasn't set up in Codex's settings ({where_})"),
    };
    let Some(text) = files::read(&path)? else {
        return Ok(absent);
    };
    let mut document = parse(&path, &text)?;
    let Some(servers) = document.get_mut(SERVERS) else {
        return Ok(absent);
    };
    let Some(servers) = servers.as_table_like_mut() else {
        return Err(not_settings(&path, "its mcp_servers isn't a table"));
    };
    if servers.remove(SERVER_NAME).is_none() {
        return Ok(absent);
    }
    files::write(&path, &document.to_string())?;
    Ok(Installed {
        host: Host::Codex,
        how: format!("removed Stave from Codex's settings ({where_})"),
    })
}

/// The settings as an editable TOML document.
fn parse(path: &Path, text: &str) -> Result<DocumentMut, LinkError> {
    text.parse()
        .map_err(|error: toml_edit::TomlError| not_settings(path, &error.to_string()))
}

/// The servers table, made when missing (as a table with no header of
/// its own: `[mcp_servers.stave]` alone is written). An inline table
/// (`mcp_servers = { files = { … } }`, which Codex reads too) becomes a
/// table of its own, its servers kept.
fn servers<'d>(document: &'d mut DocumentMut, path: &Path) -> Result<&'d mut Table, LinkError> {
    let servers = document.entry(SERVERS).or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    if servers.is_inline_table()
        && let Item::Value(Value::InlineTable(inline)) = std::mem::take(servers)
    {
        *servers = Item::Table(inline.into_table());
    }
    servers
        .as_table_mut()
        .ok_or_else(|| not_settings(path, "its mcp_servers isn't a table"))
}

/// `true` when the entry already runs `command mcp`, and holds nothing
/// else.
fn is_entry(entry: Option<&Item>, command: &str) -> bool {
    let Some(table) = entry.and_then(Item::as_table_like) else {
        return false;
    };
    let arguments = table
        .get("args")
        .and_then(Item::as_array)
        .map(|arguments| arguments.iter().map(|a| a.as_str()).collect::<Vec<_>>());
    table.len() == 2
        && table.get("command").and_then(Item::as_str) == Some(command)
        && arguments == Some(vec![Some(SERVER_ARGUMENT)])
}

/// A settings file Codex wouldn't write.
fn not_settings(path: &Path, message: &str) -> LinkError {
    LinkError::Settings {
        path: path.to_path_buf(),
        message: message.to_owned(),
    }
}
