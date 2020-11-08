use atom_syndication::Error as AtomError;
use rss::Error as RSSError;
use std::fmt;
use url::ParseError;

pub type Result<T> = std::result::Result<T, Error>;

// TODO: enum
#[derive(Debug, Clone)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<RSSError> for Error {
    fn from(err: RSSError) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}

impl From<AtomError> for Error {
    fn from(err: AtomError) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}

impl From<url::ParseError> for Error {
    fn from(err: url::ParseError) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}
