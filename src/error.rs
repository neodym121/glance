use std::fmt;

/// A message meant for the user. Built from provider responses only after API keys
/// have been redacted (see `http::redact`), so it is safe to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<windows::core::Error> for Error {
    fn from(error: windows::core::Error) -> Self {
        let message = error.message();
        if message.is_empty() {
            Self(format!("Windows error 0x{:08X}", error.code().0))
        } else {
            Self(format!("{message} (0x{:08X})", error.code().0))
        }
    }
}

impl From<slint::PlatformError> for Error {
    fn from(error: slint::PlatformError) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
