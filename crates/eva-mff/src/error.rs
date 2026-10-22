use std::{fmt, path::PathBuf};

/// Stable error categories; details include the affected path where available.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Invalid(String),
    Unsupported(String),
    Limit(String),
    InvalidSelection(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Invalid(s) => write!(f, "invalid MFF: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported MFF: {s}"),
            Self::Limit(s) => write!(f, "resource limit: {s}"),
            Self::InvalidSelection(s) => write!(f, "invalid selection: {s}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_owned(),
        source,
    }
}
pub type Result<T> = std::result::Result<T, Error>;
