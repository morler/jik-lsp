use std::collections::HashMap;
use tower_lsp::lsp_types::{Position, Url};

#[derive(Debug)]
struct Document {
    text: String,
    version: i32,
    saved: bool,
}

#[derive(Default)]
pub struct Documents(HashMap<Url, Document>);

impl Documents {
    pub fn texts(&self) -> HashMap<Url, String> {
        self.0
            .iter()
            .map(|(uri, doc)| (uri.clone(), doc.text.clone()))
            .collect()
    }

    pub fn open(&mut self, uri: Url, text: String, version: i32) -> bool {
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(entry) = self.0.entry(uri) {
            entry.insert(Document {
                text,
                version,
                saved: false,
            });
            true
        } else {
            false
        }
    }

    pub fn change(&mut self, uri: &Url, text: String, version: i32) -> bool {
        let Some(document) = self.0.get_mut(uri) else {
            return false;
        };
        if version <= document.version {
            return false;
        }
        document.text = text;
        document.version = version;
        document.saved = false;
        true
    }

    pub fn save(&mut self, uri: &Url) -> bool {
        let Some(document) = self.0.get_mut(uri) else {
            return false;
        };
        document.saved = true;
        true
    }

    pub fn close(&mut self, uri: &Url) -> bool {
        self.0.remove(uri).is_some()
    }
}

/// Convert a UTF-8 byte boundary to a UTF-16 LSP position. An offset within a
/// CRLF pair, a multibyte character, or beyond the text is invalid.
pub fn byte_to_position(text: &str, offset: usize) -> Option<Position> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let bytes = text.as_bytes();
    let inside_crlf = offset
        .checked_sub(1)
        .and_then(|start| bytes.get(start..=offset))
        == Some(b"\r\n");
    if inside_crlf {
        return None;
    }
    let mut line = 0u32;
    let mut character = 0u32;
    let mut chars = text[..offset].chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                line = line.checked_add(1)?;
                character = 0;
            }
            '\n' => {
                line = line.checked_add(1)?;
                character = 0;
            }
            _ => character = character.checked_add(ch.len_utf16() as u32)?,
        }
    }
    Some(Position::new(line, character))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_are_utf16_and_handle_line_endings() {
        let text = "a😀\r\n中\rb\n";
        for (offset, expected) in [
            (0, Some((0, 0))),
            (1, Some((0, 1))),
            (2, None),
            (5, Some((0, 3))),
            (6, None),
            (7, Some((1, 0))),
            (10, Some((1, 1))),
            (11, Some((2, 0))),
            (13, Some((3, 0))),
            (14, None),
        ] {
            assert_eq!(
                byte_to_position(text, offset),
                expected.map(|(l, c)| Position::new(l, c)),
                "offset {offset}"
            );
        }
        assert_eq!(byte_to_position("", 0), Some(Position::new(0, 0)));
        assert_eq!(byte_to_position("\n", 1), Some(Position::new(1, 0)));
    }

    #[test]
    fn document_lifecycle_and_versions() {
        let uri = Url::parse("file:///test.jik").unwrap();
        let mut docs = Documents::default();
        assert!(!docs.change(&uri, "missing".into(), 1));
        assert!(!docs.save(&uri));
        assert!(docs.open(uri.clone(), "first".into(), 3));
        assert!(!docs.open(uri.clone(), "duplicate".into(), 4));
        assert!(!docs.change(&uri, "old".into(), 3));
        assert!(!docs.change(&uri, "older".into(), 2));
        assert!(docs.change(&uri, "new".into(), 4));
        assert_eq!(docs.0[&uri].text, "new");
        assert!(!docs.0[&uri].saved);
        assert!(docs.save(&uri));
        assert!(docs.0[&uri].saved);
        assert!(docs.close(&uri));
        assert!(!docs.close(&uri));
        assert!(docs.open(uri, "reopened".into(), 1));
    }
}
