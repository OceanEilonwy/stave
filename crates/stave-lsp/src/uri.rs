//! Documents as protocol URIs: `file:///…` for files on disk, with the
//! characters a URI can't hold percent-encoded, and
//! `stave-std:///joinery/dado.stave` for the standard library's read-only
//! files ([`stave_ide::Document`]).

use std::path::PathBuf;
use std::str::FromStr;

use lsp_types::Uri;
use stave_ide::{Document, STD_SCHEME};

/// The bytes a path keeps as they are in a `file:` URI: letters, digits,
/// `-._~`, `/` and the sub-delimiters that are safe in a path.
fn is_kept(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
        )
}

/// Percent-encodes a path for a URI.
fn encode(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if is_kept(byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            for nibble in [byte >> 4, byte & 0x0f] {
                if let Some(digit) = HEX.get(usize::from(nibble)) {
                    encoded.push(char::from(*digit));
                }
            }
        }
    }
    encoded
}

/// Decodes a URI path's percent escapes. `None` when an escape is broken
/// or the bytes are not UTF-8.
fn decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        if byte == b'%' {
            let digits = after.get(..2)?;
            let digits = std::str::from_utf8(digits).ok()?;
            bytes.push(u8::from_str_radix(digits, 16).ok()?);
            rest = after.get(2..)?;
        } else {
            bytes.push(byte);
            rest = after;
        }
    }
    String::from_utf8(bytes).ok()
}

/// The URI of a document.
pub fn document_uri(document: &Document) -> Option<Uri> {
    let text = match document {
        Document::File(path) => format!("file://{}", encode(&path.to_string_lossy())),
        Document::Std { .. } => document.uri(),
    };
    Uri::from_str(&text).ok()
}

/// The document a URI names: a `file:` URI (its host, if any, ignored)
/// or a standard library file. `None` for any other URI.
pub fn uri_document(uri: &Uri) -> Option<Document> {
    let text = uri.as_str();
    if let Some(rest) = text.strip_prefix("file://") {
        // `file://host/path` or `file:///path`: the path starts at the
        // first `/` after the host.
        let path = rest.get(rest.find('/')?..)?;
        let path = path.split(['?', '#']).next().unwrap_or(path);
        return Some(Document::File(PathBuf::from(decode(path)?)));
    }
    if text.starts_with(STD_SCHEME) {
        return Document::from_uri(&decode(text)?);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_with_spaces_and_accents_round_trip() {
        let document = Document::File(PathBuf::from("/home/a b/armoire é/doors.stave"));
        let uri = document_uri(&document).unwrap();
        assert_eq!(
            uri.as_str(),
            "file:///home/a%20b/armoire%20%C3%A9/doors.stave"
        );
        assert_eq!(uri_document(&uri), Some(document));
    }

    #[test]
    fn standard_files_have_their_own_scheme() {
        let dado = Document::std("joinery", "dado.stave").unwrap();
        let uri = document_uri(&dado).unwrap();
        assert_eq!(uri.as_str(), "stave-std:///joinery/dado.stave");
        assert_eq!(uri_document(&uri), Some(dado));
    }

    #[test]
    fn other_uris_are_not_documents() {
        let uri = Uri::from_str("untitled:Untitled-1").unwrap();
        assert_eq!(uri_document(&uri), None);
        let broken = Uri::from_str("file:///a%zzb.stave").ok();
        assert_eq!(broken.as_ref().and_then(uri_document), None);
    }

    #[test]
    fn a_host_is_ignored() {
        let uri = Uri::from_str("file://localhost/p/doors.stave").unwrap();
        assert_eq!(
            uri_document(&uri),
            Some(Document::File(PathBuf::from("/p/doors.stave")))
        );
    }
}
