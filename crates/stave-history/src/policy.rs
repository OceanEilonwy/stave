//! The merge policy: whether agents may apply their suggestions to main.
//!
//! It is kept beside the project, not in a `.stave` file: `.stave/agents.toml`
//! in the project folder, with a global default in `agents.toml` in Stave's
//! configuration folder (`~/.config/stave/`). Each file holds one setting:
//!
//! ```toml
//! merge_policy = "ask"   # "off", "ask" or "auto"
//! ```

use std::path::Path;

/// Whether agents may apply suggestions to main.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergePolicy {
    /// Agents can suggest; only the person applies, from the window.
    Off,
    /// Applying is marked as changing the project and the AI app asks the
    /// person first (the default).
    Ask,
    /// An agent may apply any suggestion that checks clean.
    Auto,
}

/// The merge policy for the project in `folder`: its `.stave/agents.toml`,
/// else `<config_dir>/agents.toml`, else [`MergePolicy::Ask`]. A file that
/// is missing, unreadable or holds no valid `merge_policy` is passed over.
pub fn merge_policy(folder: &Path, config_dir: &Path) -> MergePolicy {
    read(&folder.join(".stave").join("agents.toml"))
        .or_else(|| read(&config_dir.join("agents.toml")))
        .unwrap_or(MergePolicy::Ask)
}

fn read(file: &Path) -> Option<MergePolicy> {
    let text = std::fs::read_to_string(file).ok()?;
    let table: toml::Table = text.parse().ok()?;
    match table.get("merge_policy")?.as_str()? {
        "off" => Some(MergePolicy::Off),
        "ask" => Some(MergePolicy::Ask),
        "auto" => Some(MergePolicy::Auto),
        _ => None,
    }
}
