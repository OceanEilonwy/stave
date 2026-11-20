//! Who made a revision, and the trailers a revision's description carries.
//!
//! A revision's description is its message, a blank line, then one trailer
//! per line:
//!
//! ```text
//! Widen the drawer
//!
//! Stave-Actor: agent Claude Desktop 3f2a
//! Stave-Suggestion: 5be0c1d2e3f4
//! Stave-Patch: c2V0IGRyYXdlci53aWR0aCA9IDUwMG1t
//! ```
//!
//! `Stave-Actor` is the kind (`user` or `agent`), the name and the id; the
//! id is the last word. `Stave-Patch` is a patch, base64, one per patch.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

const ACTOR: &str = "Stave-Actor: ";
const SUGGESTION: &str = "Stave-Suggestion: ";
const PATCH: &str = "Stave-Patch: ";
const APPLIES: &str = "Stave-Applies: ";

/// Whether a change came from the person using Stave or from an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActorKind {
    /// The person using the Stave window.
    User,
    /// An AI app connected to Stave (one connection).
    Agent,
}

impl ActorKind {
    fn word(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        match word {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// Who made a change: the person, or one agent connection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Actor {
    /// The person or an agent.
    pub kind: ActorKind,
    /// The name shown for them ("Claude Desktop", the person's name).
    pub name: String,
    /// A short id telling apart two connections with the same name.
    pub id: String,
}

impl Actor {
    /// The actor as one line: kind, name, id. The name loses line breaks and
    /// the id loses spaces, so the line reads back the same.
    pub(crate) fn to_line(&self) -> String {
        let name = self.name.replace(['\n', '\r'], " ");
        let id: String = self
            .id
            .chars()
            .map(|letter| if letter.is_whitespace() { '-' } else { letter })
            .collect();
        format!("{} {} {}", self.kind.word(), name.trim(), id)
    }

    /// Reads a line written by [`Actor::to_line`].
    pub(crate) fn from_line(line: &str) -> Option<Self> {
        let (kind, rest) = line.split_once(' ')?;
        let kind = ActorKind::from_word(kind)?;
        let (name, id) = rest.rsplit_once(' ').unwrap_or((rest, ""));
        Some(Self {
            kind,
            name: name.to_owned(),
            id: id.to_owned(),
        })
    }

    /// Whether `other` is the same actor, as the trailers record them.
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.to_line() == other.to_line()
    }
}

/// A revision's description taken apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Description {
    /// The message, without trailers.
    pub message: String,
    /// Who made it, when Stave recorded it.
    pub actor: Option<Actor>,
    /// The suggestion it belongs to.
    pub suggestion: Option<String>,
    /// The suggestion a revision on main applied.
    pub applies: Option<String>,
    /// The patches that made it, in order.
    pub patches: Vec<String>,
}

impl Description {
    /// Reads a description; one without trailers is all message.
    pub fn parse(text: &str) -> Self {
        let text = text.trim_end();
        let (message, trailers) = match text.rsplit_once("\n\n") {
            Some((message, tail)) if tail.lines().all(|line| line.starts_with("Stave-")) => {
                (message, tail)
            }
            _ if text.lines().all(|line| line.starts_with("Stave-")) && !text.is_empty() => {
                ("", text)
            }
            _ => (text, ""),
        };
        let mut description = Self {
            message: message.trim_end().to_owned(),
            ..Self::default()
        };
        for line in trailers.lines() {
            if let Some(actor) = line.strip_prefix(ACTOR) {
                description.actor = Actor::from_line(actor);
            } else if let Some(id) = line.strip_prefix(SUGGESTION) {
                description.suggestion = Some(id.to_owned());
            } else if let Some(id) = line.strip_prefix(APPLIES) {
                description.applies = Some(id.to_owned());
            } else if let Some(patch) = line.strip_prefix(PATCH)
                && let Ok(bytes) = STANDARD.decode(patch)
                && let Ok(patch) = String::from_utf8(bytes)
            {
                description.patches.push(patch);
            }
        }
        description
    }

    /// Writes the description back as text.
    pub fn to_text(&self) -> String {
        let mut text = self.message.trim_end().to_owned();
        let mut trailers = Vec::new();
        if let Some(actor) = &self.actor {
            trailers.push(format!("{ACTOR}{}", actor.to_line()));
        }
        if let Some(id) = &self.suggestion {
            trailers.push(format!("{SUGGESTION}{id}"));
        }
        if let Some(id) = &self.applies {
            trailers.push(format!("{APPLIES}{id}"));
        }
        for patch in &self.patches {
            trailers.push(format!("{PATCH}{}", STANDARD.encode(patch)));
        }
        if !trailers.is_empty() {
            text.push_str("\n\n");
            text.push_str(&trailers.join("\n"));
        }
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> Actor {
        Actor {
            kind: ActorKind::Agent,
            name: "Claude Desktop".to_owned(),
            id: "3f2a".to_owned(),
        }
    }

    #[test]
    fn description_round_trips() {
        let description = Description {
            message: "Widen the drawer\n\nso the shoes fit".to_owned(),
            actor: Some(agent()),
            suggestion: Some("5be0".to_owned()),
            applies: None,
            patches: vec!["set drawer.width = 500mm\n".to_owned()],
        };
        let text = description.to_text();
        assert!(text.contains("Stave-Actor: agent Claude Desktop 3f2a"));
        assert_eq!(Description::parse(&text), description);
    }

    #[test]
    fn plain_message_has_no_trailers() {
        let description = Description::parse("First cut\n\nwith a note\n");
        assert_eq!(description.message, "First cut\n\nwith a note");
        assert_eq!(description.actor, None);
    }

    #[test]
    fn actor_line_survives_odd_names() {
        let actor = Actor {
            kind: ActorKind::User,
            name: "Ann\nLee".to_owned(),
            id: "a b".to_owned(),
        };
        let read = Actor::from_line(&actor.to_line()).expect("reads back");
        assert_eq!(read.name, "Ann Lee");
        assert_eq!(read.id, "a-b");
    }
}
