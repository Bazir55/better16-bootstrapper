use alloc::format;
use alloc::string::String;
use core::fmt;

use crate::win;

#[derive(Debug)]
pub enum Error {
    /// The user pressed Cancel.
    Cancelled,
    /// The server answered 404. Kept apart because some files are optional.
    NotFound(String),
    Other(String),
}

pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    pub fn new(msg: impl Into<String>) -> Error {
        Error::Other(msg.into())
    }

    /// Wraps the thread's last Win32 error with a note on what failed.
    pub fn last_os(what: &str) -> Error {
        Error::os(what, win::last_error())
    }

    pub fn os(what: &str, code: u32) -> Error {
        Error::Other(format!("{what}: {} (error {code})", win::error_text(code)))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str("cancelled"),
            Error::NotFound(url) => write!(f, "{url} was not found on the server"),
            Error::Other(msg) => f.write_str(msg),
        }
    }
}

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::Error::new(alloc::format!($($arg)*)))
    };
}
pub(crate) use bail;
