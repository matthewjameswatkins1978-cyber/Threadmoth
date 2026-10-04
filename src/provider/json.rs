#![forbid(unsafe_code)]

use crate::engine::ByteEdit;
use crate::protocol::{Cardinality, RefusalReason};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum JsonOperation {
    Set {
        path: String,
        value: serde_json::Value,
    },
    Insert {
        path: String,
        key_or_index: String,
        value: serde_json::Value,
    },
    Delete {
        path: String,
    },
    RenameKey {
        path: String,
        new_key: String,
    },
    EnsurePresent {
        path: String,
        value: serde_json::Value,
    },
    EnsureAbsent {
        path: String,
    },
    Unset {
        path: String,
    },
    Rename {
        path: String,
        new_key: String,
    },
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum JsonProviderError {
    #[error("Refused: {0:?}")]
    Refused(RefusalReason),
    #[error("Error: {message}")]
    Error { message: String },
}

pub struct JsonProvider;

impl JsonProvider {
    pub fn plan(
        content: &[u8],
        op: &JsonOperation,
        cardinality: &Cardinality,
    ) -> Result<Vec<ByteEdit>, JsonProviderError> {
        if !matches!(cardinality, Cardinality::ExactlyOne)
            && !matches!(
                op,
                JsonOperation::EnsureAbsent { .. } | JsonOperation::Unset { .. }
            )
        {
            return Err(JsonProviderError::Refused(
                RefusalReason::CardinalityMismatch {
                    expected: "exactly_one (structured paths are unique)".into(),
                    actual: 1,
                },
            ));
        }
        let tree = parse_document(content)?;
        let edit = match op {
            JsonOperation::Set { path, value } => {
                let node = locate(&tree, &parse_path(path)?)?;
                ByteEdit {
                    start: node.start,
                    end: node.end,
                    replacement: serde_json::to_vec(value).map_err(|e| {
                        JsonProviderError::Error {
                            message: e.to_string(),
                        }
                    })?,
                }
            }
            JsonOperation::Insert {
                path,
                key_or_index,
                value,
            } => insert_edit(content, &tree, path, key_or_index, value)?,
            JsonOperation::Delete { path } => delete_edit(&tree, path)?,
            JsonOperation::RenameKey { path, new_key } => rename_edit(&tree, path, new_key)?,
            JsonOperation::EnsurePresent { path, value } => {
                let segments = parse_path(path)?;
                match locate(&tree, &segments) {
                    Ok(node) => ByteEdit {
                        start: node.start,
                        end: node.end,
                        replacement: serde_json::to_vec(value).map_err(|e| {
                            JsonProviderError::Error {
                                message: e.to_string(),
                            }
                        })?,
                    },
                    Err(JsonProviderError::Refused(RefusalReason::MissingTarget { .. })) => {
                        let (parent_path, key) = split_json_path(&segments)?;
                        insert_edit(content, &tree, &parent_path, &key, value)?
                    }
                    Err(error) => return Err(error),
                }
            }
            JsonOperation::EnsureAbsent { path } | JsonOperation::Unset { path } => {
                match delete_edit(&tree, path) {
                    Ok(edit) => edit,
                    Err(JsonProviderError::Refused(RefusalReason::MissingTarget { .. })) => {
                        return Ok(Vec::new())
                    }
                    Err(e) => return Err(e),
                }
            }
            JsonOperation::Rename { path, new_key } => rename_edit(&tree, path, new_key)?,
        };
        if edit.start == edit.end && edit.replacement.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(vec![edit])
        }
    }

    pub fn validate(content: &[u8]) -> Result<(), JsonProviderError> {
        parse_document(content).map(|_| ())
    }
}

#[derive(Debug, Clone)]
struct Node {
    start: usize,
    end: usize,
    kind: NodeKind,
}
#[derive(Debug, Clone)]
enum NodeKind {
    Object(Vec<Member>),
    Array(Vec<Node>),
    Scalar,
}
#[derive(Debug, Clone)]
struct Member {
    key: String,
    key_start: usize,
    key_end: usize,
    start: usize,
    end: usize,
    value: Node,
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    fn value(&mut self) -> Result<Node, JsonProviderError> {
        self.ws();
        let start = self.pos;
        let b = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| malformed("unexpected end of JSON"))?;
        match b {
            b'{' => self.object(start),
            b'[' => self.array(start),
            b'"' => {
                self.string()?;
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: NodeKind::Scalar,
                })
            }
            _ => {
                while self.pos < self.bytes.len()
                    && !matches!(self.bytes[self.pos], b',' | b']' | b'}')
                    && !self.bytes[self.pos].is_ascii_whitespace()
                {
                    self.pos += 1;
                }
                if self.pos == start {
                    Err(malformed("expected JSON value"))
                } else {
                    Ok(Node {
                        start,
                        end: self.pos,
                        kind: NodeKind::Scalar,
                    })
                }
            }
        }
    }
    fn string(&mut self) -> Result<String, JsonProviderError> {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\\' => {
                    self.pos += 2;
                }
                b'"' => {
                    self.pos += 1;
                    return serde_json::from_slice(&self.bytes[start..self.pos])
                        .map_err(|e| malformed(&format!("invalid JSON string: {e}")));
                }
                _ => self.pos += 1,
            }
        }
        Err(malformed("unterminated JSON string"))
    }
    fn object(&mut self, start: usize) -> Result<Node, JsonProviderError> {
        self.pos += 1;
        self.ws();
        let mut members = Vec::new();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Node {
                start,
                end: self.pos,
                kind: NodeKind::Object(members),
            });
        }
        loop {
            self.ws();
            let key_start = self.pos;
            let key = self.string()?;
            let key_end = self.pos;
            self.ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(malformed("expected ':' after object key"));
            }
            self.pos += 1;
            let value = self.value()?;
            let end = value.end;
            if members.iter().any(|m: &Member| m.key == key) {
                return Err(JsonProviderError::Refused(
                    RefusalReason::CardinalityAmbiguous {
                        path: key.clone(),
                        count: 2,
                    },
                ));
            }
            members.push(Member {
                key,
                key_start,
                key_end,
                start: key_start,
                end,
                value,
            });
            self.ws();
            match self.bytes.get(self.pos) {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(malformed("expected ',' or '}' in object")),
            }
        }
        Ok(Node {
            start,
            end: self.pos,
            kind: NodeKind::Object(members),
        })
    }
    fn array(&mut self, start: usize) -> Result<Node, JsonProviderError> {
        self.pos += 1;
        self.ws();
        let mut values = Vec::new();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Node {
                start,
                end: self.pos,
                kind: NodeKind::Array(values),
            });
        }
        loop {
            values.push(self.value()?);
            self.ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(malformed("expected ',' or ']' in array")),
            }
        }
        Ok(Node {
            start,
            end: self.pos,
            kind: NodeKind::Array(values),
        })
    }
}

fn malformed(details: &str) -> JsonProviderError {
    JsonProviderError::Refused(RefusalReason::MalformedInput {
        details: details.into(),
    })
}
fn parse_document(content: &[u8]) -> Result<Node, JsonProviderError> {
    let body = if content.starts_with(&[0xef, 0xbb, 0xbf]) {
        &content[3..]
    } else {
        content
    };
    serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|e| malformed(&format!("Malformed JSON syntax: {e}")))?;
    let mut p = Parser {
        bytes: content,
        pos: if content.starts_with(&[0xef, 0xbb, 0xbf]) {
            3
        } else {
            0
        },
    };
    let root = p.value()?;
    p.ws();
    if p.pos != content.len() {
        return Err(malformed("trailing non-whitespace after JSON value"));
    }
    Ok(root)
}

#[derive(Debug, Clone)]
enum Segment {
    Key(String),
    Index(usize),
}
fn parse_path(path: &str) -> Result<Vec<Segment>, JsonProviderError> {
    if path.is_empty() || path == "$" {
        return Ok(Vec::new());
    }
    if path.starts_with('/') {
        return Err(malformed("JSON Pointer selectors are not supported; use dotted or bracket syntax, for example '$.port' or '$.service.port'."));
    }
    let mut s = path.strip_prefix("$").unwrap_or(path);
    if let Some(rest) = s.strip_prefix('.') {
        s = rest;
    }
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '.' {
            i += 1;
            continue;
        }
        if chars[i] == '[' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != ']' {
                i += 1;
            }
            if i == chars.len() {
                return Err(malformed("unclosed bracket in JSON path"));
            }
            let inner: String = chars[start..i].iter().collect();
            i += 1;
            let t = inner.trim();
            if let Ok(n) = t.parse() {
                out.push(Segment::Index(n));
            } else if t.len() >= 2
                && ((t.starts_with('"') && t.ends_with('"'))
                    || (t.starts_with('\'') && t.ends_with('\'')))
            {
                out.push(Segment::Key(t[1..t.len() - 1].into()));
            } else {
                out.push(Segment::Key(t.into()));
            }
        } else {
            let start = i;
            while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                i += 1;
            }
            if start == i {
                return Err(malformed("empty JSON path segment"));
            }
            out.push(Segment::Key(chars[start..i].iter().collect()));
        }
    }
    Ok(out)
}

fn locate<'a>(node: &'a Node, path: &[Segment]) -> Result<&'a Node, JsonProviderError> {
    if path.is_empty() {
        return Ok(node);
    }
    match (&node.kind, &path[0]) {
        (NodeKind::Object(ms), Segment::Key(k)) => ms
            .iter()
            .find(|m| &m.key == k)
            .map(|m| locate(&m.value, &path[1..]))
            .transpose()?
            .ok_or_else(|| missing(&format!("key '{k}' not found"))),
        (NodeKind::Array(xs), Segment::Index(i)) => xs
            .get(*i)
            .map(|n| locate(n, &path[1..]))
            .transpose()?
            .ok_or_else(|| missing(&format!("array index {i} out of bounds"))),
        _ => Err(missing("path traverses a non-container")),
    }
}
fn parent<'a, 'b>(
    root: &'a Node,
    path: &'b [Segment],
) -> Result<(&'a Node, &'b Segment), JsonProviderError> {
    if path.is_empty() {
        return Err(missing("root has no parent"));
    }
    if path.len() == 1 {
        Ok((root, &path[0]))
    } else {
        Ok((
            locate(root, &path[..path.len() - 1])?,
            &path[path.len() - 1],
        ))
    }
}
fn missing(s: &str) -> JsonProviderError {
    JsonProviderError::Refused(RefusalReason::MissingTarget { target: s.into() })
}

fn split_json_path(segments: &[Segment]) -> Result<(String, String), JsonProviderError> {
    let Some(last) = segments.last() else {
        return Err(missing("cannot ensure the JSON root is present"));
    };
    let key = match last {
        Segment::Key(key) => key.clone(),
        Segment::Index(_) => return Err(missing("ensure_present requires an object key")),
    };
    let mut path = String::from("$");
    for segment in &segments[..segments.len() - 1] {
        match segment {
            Segment::Key(key) => {
                path.push('[');
                path.push_str(&serde_json::to_string(key).unwrap());
                path.push(']');
            }
            Segment::Index(index) => {
                path.push('[');
                path.push_str(&index.to_string());
                path.push(']');
            }
        }
    }
    Ok((path, key))
}
fn json_bytes(v: &serde_json::Value) -> Result<Vec<u8>, JsonProviderError> {
    serde_json::to_vec(v).map_err(|e| JsonProviderError::Error {
        message: e.to_string(),
    })
}

fn rename_edit(root: &Node, path: &str, new_key: &str) -> Result<ByteEdit, JsonProviderError> {
    let segs = parse_path(path)?;
    let (p, last) = parent(root, &segs)?;
    let (ms, k) = match (&p.kind, last) {
        (NodeKind::Object(ms), Segment::Key(k)) => (ms, k),
        _ => return Err(missing("rename target is not an object key")),
    };
    let m = ms
        .iter()
        .find(|m| &m.key == k)
        .ok_or_else(|| missing(&format!("key '{k}' not found")))?;
    if ms.iter().any(|x| x.key == new_key) {
        return Err(JsonProviderError::Refused(
            RefusalReason::CardinalityAmbiguous {
                path: new_key.into(),
                count: 2,
            },
        ));
    }
    Ok(ByteEdit {
        start: m.key_start,
        end: m.key_end,
        replacement: serde_json::to_vec(new_key).unwrap(),
    })
}

fn delete_edit(root: &Node, path: &str) -> Result<ByteEdit, JsonProviderError> {
    let segs = parse_path(path)?;
    let (p, last) = parent(root, &segs)?;
    match (&p.kind, last) {
        (NodeKind::Object(ms), Segment::Key(k)) => {
            let i = ms
                .iter()
                .position(|m| &m.key == k)
                .ok_or_else(|| missing(&format!("key '{k}' not found")))?;
            let (start, end) = if ms.len() == 1 {
                (ms[i].start, ms[i].end)
            } else if i + 1 < ms.len() {
                (ms[i].start, ms[i + 1].start)
            } else {
                (ms[i - 1].end, ms[i].end)
            };
            Ok(ByteEdit {
                start,
                end,
                replacement: Vec::new(),
            })
        }
        (NodeKind::Array(xs), Segment::Index(i)) => {
            if *i >= xs.len() {
                return Err(missing(&format!("array index {i} out of bounds")));
            }
            let (start, end) = if xs.len() == 1 {
                (xs[*i].start, xs[*i].end)
            } else if *i + 1 < xs.len() {
                (xs[*i].start, xs[*i + 1].start)
            } else {
                (xs[*i - 1].end, xs[*i].end)
            };
            Ok(ByteEdit {
                start,
                end,
                replacement: Vec::new(),
            })
        }
        _ => Err(missing("delete target is not a member or array element")),
    }
}

fn insert_edit(
    content: &[u8],
    root: &Node,
    path: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<ByteEdit, JsonProviderError> {
    let node = locate(root, &parse_path(path)?)?;
    let member = if matches!(node.kind, NodeKind::Object(_)) {
        format!(
            "{}:{}",
            serde_json::to_string(key).unwrap(),
            String::from_utf8(json_bytes(value)?).unwrap()
        )
    } else {
        String::from_utf8(json_bytes(value)?).unwrap()
    };
    match &node.kind {
        NodeKind::Object(ms) => {
            if ms.iter().any(|m| m.key == key) {
                return Err(JsonProviderError::Refused(
                    RefusalReason::CardinalityAmbiguous {
                        path: key.into(),
                        count: 2,
                    },
                ));
            }
            let close = node.end - 1;
            if let Some(last) = ms.last() {
                let layout = &content[last.end..close];
                if layout.contains(&b'\n') {
                    let nl = if layout.windows(2).any(|w| w == b"\r\n") {
                        b"\r\n".as_slice()
                    } else {
                        b"\n".as_slice()
                    };
                    let line_start = content[..last.start]
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .map(|n| n + 1)
                        .unwrap_or(0);
                    let indent = &content[line_start..last.start];
                    return Ok(ByteEdit {
                        start: last.end,
                        end: close,
                        replacement: [
                            vec![b','],
                            nl.to_vec(),
                            indent.to_vec(),
                            member.into_bytes(),
                            layout.to_vec(),
                        ]
                        .concat(),
                    });
                }
                return Ok(ByteEdit {
                    start: last.end,
                    end: close,
                    replacement: [
                        vec![b','],
                        layout.to_vec(),
                        member.into_bytes(),
                        layout.to_vec(),
                    ]
                    .concat(),
                });
            }
            Ok(ByteEdit {
                start: node.start + 1,
                end: close,
                replacement: member.into_bytes(),
            })
        }
        NodeKind::Array(xs) => {
            let close = node.end - 1;
            if key != "push" && key != "append" && key.parse::<usize>().is_err() {
                return Err(malformed(
                    "array insertion key must be an index, push, or append",
                ));
            }
            if let Ok(index) = key.parse::<usize>() {
                if index > xs.len() {
                    return Err(missing(&format!(
                        "array insertion index {index} out of bounds"
                    )));
                }
                let value_bytes = json_bytes(value)?;
                if index < xs.len() {
                    let start = if index == 0 {
                        node.start + 1
                    } else {
                        xs[index - 1].end
                    };
                    let end = xs[index].start;
                    let layout = &content[start..end];
                    let replacement = if index == 0 {
                        [value_bytes, vec![b','], layout.to_vec()].concat()
                    } else {
                        [layout.to_vec(), value_bytes, layout.to_vec()].concat()
                    };
                    return Ok(ByteEdit {
                        start,
                        end,
                        replacement,
                    });
                }
            }
            if let Some(last) = xs.last() {
                let layout = &content[last.end..close];
                let sep = if layout.contains(&b'\n') {
                    let nl: &[u8] = if layout.windows(2).any(|w| w == b"\r\n") {
                        b"\r\n"
                    } else {
                        b"\n"
                    };
                    [vec![b','], nl.to_vec(), json_bytes(value)?, layout.to_vec()].concat()
                } else {
                    [
                        vec![b','],
                        layout.to_vec(),
                        json_bytes(value)?,
                        layout.to_vec(),
                    ]
                    .concat()
                };
                return Ok(ByteEdit {
                    start: last.end,
                    end: close,
                    replacement: sep,
                });
            }
            Ok(ByteEdit {
                start: node.start + 1,
                end: close,
                replacement: json_bytes(value)?,
            })
        }
        NodeKind::Scalar => Err(missing("insert target is not an object or array")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::apply_byte_edits;
    fn apply(c: &[u8], o: JsonOperation) -> Vec<u8> {
        apply_byte_edits(
            c,
            &JsonProvider::plan(c, &o, &Cardinality::ExactlyOne).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn set_preserves_unrelated_formatting() {
        let c = b"{  \"a\" : 1, \"b\": [ 2,3 ] }  \n";
        let out = apply(
            c,
            JsonOperation::Set {
                path: "$.a".into(),
                value: serde_json::json!(10),
            },
        );
        assert_eq!(out, b"{  \"a\" : 10, \"b\": [ 2,3 ] }  \n");
    }
    #[test]
    fn minified_set_is_local() {
        let c = br#"{"x":1,"y":2}"#;
        assert_eq!(
            apply(
                c,
                JsonOperation::Set {
                    path: "$.x".into(),
                    value: serde_json::json!(9)
                }
            ),
            br#"{"x":9,"y":2}"#
        );
    }
    #[test]
    fn insert_delete_rename_work() {
        let c = br#"{"a":1,"b":[2,3]}"#;
        let c = apply(
            c,
            JsonOperation::Insert {
                path: "$.b".into(),
                key_or_index: "1".into(),
                value: serde_json::json!(8),
            },
        );
        assert_eq!(c, br#"{"a":1,"b":[2,8,3]}"#);
        let c = apply(
            &c,
            JsonOperation::RenameKey {
                path: "$.a".into(),
                new_key: "z".into(),
            },
        );
        assert!(String::from_utf8(c).unwrap().contains("\"z\""));
    }
    #[test]
    fn malformed_refused() {
        assert!(JsonProvider::plan(
            br#"{"a": }"#,
            &JsonOperation::Set {
                path: "$.a".into(),
                value: serde_json::json!(1)
            },
            &Cardinality::ExactlyOne
        )
        .is_err());
    }
}
