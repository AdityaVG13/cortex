use std::fmt;

/// Store failures. No Memory/account variants: those APIs do not exist here.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Entropy,
    IntegrityCollision,
    SessionInactive,
    UnknownSession,
    UnknownLoc(u64),
    UnknownObject,
    InvalidSpan { start: u64, end: u64, len: u64 },
    NotYetImplemented(&'static str),
    UnknownProducer(String),
    UnknownAlgorithm(String),
    DigestMismatch { producer: String, algorithm: String },
    ImportConflict,
    Fragment(String),
    Quota,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Sqlite(e) => write!(f, "sqlite: {e}"),
            Self::Entropy => write!(f, "CSPRNG failed"),
            Self::IntegrityCollision => {
                write!(
                    f,
                    "integrity collision: same seal and length, different bytes"
                )
            }
            Self::SessionInactive => write!(f, "session is not active"),
            Self::UnknownSession => write!(f, "unknown session"),
            Self::UnknownLoc(no) => write!(f, "unknown loc {no}"),
            Self::UnknownObject => write!(f, "unknown object"),
            Self::InvalidSpan { start, end, len } => {
                write!(
                    f,
                    "byte span {start}..{end} is invalid for object length {len}"
                )
            }
            Self::NotYetImplemented(api) => write!(f, "{api} is not implemented in this bead"),
            Self::UnknownProducer(p) => write!(f, "unknown producer {p}"),
            Self::UnknownAlgorithm(a) => write!(f, "unknown algorithm {a}"),
            Self::DigestMismatch {
                producer,
                algorithm,
            } => {
                write!(
                    f,
                    "digest mismatch: producer {producer} algorithm {algorithm}"
                )
            }
            Self::ImportConflict => write!(f, "import conflict: same key, different bytes"),
            Self::Fragment(m) => write!(f, "fragment {m}"),
            Self::Quota => write!(f, "loc quota exceeded"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// Copy-paste next four-tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextCall {
    pub tool: String,
    pub args: serde_json::Value,
}

/// Guard failure with teaching `next`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: &'static str,
    pub say: String,
    pub next: NextCall,
}

impl Failure {
    pub fn new(
        code: &'static str,
        say: impl Into<String>,
        tool: &str,
        args: serde_json::Value,
    ) -> Self {
        Self {
            code,
            say: say.into(),
            next: NextCall {
                tool: tool.to_string(),
                args,
            },
        }
    }
}

/// Mint/bind errors. Store faults vs G1–G5.
#[derive(Debug)]
pub enum BindError {
    Store(Error),
    Guard(Failure),
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Guard(failure) => write!(f, "{failure:?}"),
        }
    }
}

impl std::error::Error for BindError {}

impl From<Error> for BindError {
    fn from(e: Error) -> Self {
        Self::Store(e)
    }
}

impl BindError {
    pub fn failure(&self) -> Option<&Failure> {
        match self {
            Self::Guard(f) => Some(f),
            Self::Store(_) => None,
        }
    }
}
