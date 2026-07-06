use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutospecError {
    InvalidCommand(String),
}

impl fmt::Display for AutospecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutospecError::InvalidCommand(command) => {
                write!(formatter, "invalid autospec command: {command}")
            }
        }
    }
}

impl Error for AutospecError {}
