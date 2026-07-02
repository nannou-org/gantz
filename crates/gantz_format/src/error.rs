//! Errors produced while reading or writing the `.gantz` text format.

use std::fmt;

/// A byte range into the source text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
}

impl Span {
    /// Construct a span from start and end byte offsets.
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
}

/// The kind of a [`FormatError`], for programmatic inspection in tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The underlying Steel reader rejected the input.
    Read(String),
    /// A form was malformed (wrong shape or arity).
    Malformed(String),
    /// An unrecognised top-level form (not `graph`/`layout`/`commits`/`names`/`demo`).
    UnknownForm(String),
    /// An unrecognised node keyword.
    UnknownNodeKeyword(String),
    /// A connection or layout referenced a node label that was not declared.
    UnknownNode(String),
    /// A node label was declared more than once in the same graph.
    DuplicateNode(String),
    /// A port index exceeded a node's input/output count.
    PortOutOfRange {
        /// The node label.
        node: String,
        /// The offending port index.
        port: u16,
        /// The number of available ports.
        max: usize,
    },
    /// An address token could not be parsed.
    BadAddr(String),
    /// References between names formed a cycle.
    CycleInRefs(Vec<String>),
    /// A node failed to deserialize through the node set's serde dispatch.
    NodeDeserialize {
        /// The node's `"type"` tag, if known.
        tag: String,
        /// The serde error message.
        msg: String,
    },
    /// A referenced commit/graph was not present in the file.
    MissingDependency(String),
}

/// An error produced while parsing or serializing a `.gantz` document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError {
    /// 1-based line, if the error has a known source location.
    pub line: Option<usize>,
    /// 1-based column, if the error has a known source location.
    pub col: Option<usize>,
    /// The kind of error.
    pub kind: ErrorKind,
}

impl FormatError {
    /// Construct an error with no source location.
    pub fn new(kind: ErrorKind) -> Self {
        FormatError {
            line: None,
            col: None,
            kind,
        }
    }

    /// Construct an [`ErrorKind::Malformed`] error (wrong shape or arity) with no
    /// source location - the constructor an out-of-crate [`Sugar`](crate::Sugar)
    /// uses to report a malformed form.
    pub fn malformed(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Malformed(msg.into()))
    }

    /// Construct a [`ErrorKind::NodeDeserialize`] error for node `tag`.
    pub(crate) fn node_deserialize(tag: impl Into<String>, msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::NodeDeserialize {
            tag: tag.into(),
            msg: msg.into(),
        })
    }

    /// Attach a source location derived from a byte span within `src`.
    pub fn at(mut self, span: Span, src: &str) -> Self {
        let (line, col) = line_col(src, span.start);
        self.line = Some(line);
        self.col = Some(col);
        self
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.col) {
            (Some(l), Some(c)) => write!(f, "{} (line {l}, col {c})", self.kind),
            _ => write!(f, "{}", self.kind),
        }
    }
}

impl std::error::Error for FormatError {}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Read(m) => write!(f, "failed to read s-expressions: {m}"),
            ErrorKind::Malformed(m) => write!(f, "malformed form: {m}"),
            ErrorKind::UnknownForm(s) => write!(f, "unknown top-level form `{s}`"),
            ErrorKind::UnknownNodeKeyword(s) => write!(f, "unknown node keyword `{s}`"),
            ErrorKind::UnknownNode(s) => write!(f, "reference to undeclared node `{s}`"),
            ErrorKind::DuplicateNode(s) => write!(f, "duplicate node label `{s}`"),
            ErrorKind::PortOutOfRange { node, port, max } => {
                write!(f, "port {port} out of range for node `{node}` (has {max})")
            }
            ErrorKind::BadAddr(s) => write!(f, "invalid address `{s}`"),
            ErrorKind::CycleInRefs(names) => {
                write!(f, "reference cycle between names: {}", names.join(", "))
            }
            ErrorKind::NodeDeserialize { tag, msg } => {
                write!(f, "failed to deserialize node `{tag}`: {msg}")
            }
            ErrorKind::MissingDependency(s) => write!(f, "missing dependency `{s}`"),
        }
    }
}

/// Compute a 1-based `(line, column)` for a byte offset within `src`.
fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in src.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
