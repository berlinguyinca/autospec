use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutospecError {
    Validation {
        message: String,
    },
    Parse {
        context: String,
        message: String,
    },
    State {
        entity: String,
        message: String,
    },
    Schema {
        schema_name: String,
        version: String,
    },
    Invariant {
        message: String,
    },
    Io {
        operation: String,
        path: String,
        source: String,
    },
    Other {
        message: String,
    },
}

impl AutospecError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn parse(context: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Parse {
            context: context.into(),
            message: message.into(),
        }
    }

    pub fn state(entity: impl Into<String>, message: impl Into<String>) -> Self {
        Self::State {
            entity: entity.into(),
            message: message.into(),
        }
    }

    pub fn schema(schema_name: impl Into<String>, version: impl Into<String>) -> Self {
        Self::Schema {
            schema_name: schema_name.into(),
            version: version.into(),
        }
    }

    pub fn invariant(message: impl Into<String>) -> Self {
        Self::Invariant {
            message: message.into(),
        }
    }

    pub fn io(
        operation: impl Into<String>,
        path: impl Into<String>,
        error: impl fmt::Display,
    ) -> Self {
        Self::Io {
            operation: operation.into(),
            path: path.into(),
            source: error.to_string(),
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other {
            message: message.into(),
        }
    }
}

impl fmt::Display for AutospecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutospecError::Validation { message } => write!(formatter, "validation: {message}"),
            AutospecError::Parse { context, message } => {
                write!(formatter, "parse error in {context}: {message}")
            }
            AutospecError::State { entity, message } => {
                write!(formatter, "state error for {entity}: {message}")
            }
            AutospecError::Schema {
                schema_name,
                version,
            } => {
                write!(formatter, "unsupported {schema_name} schema: {version}")
            }
            AutospecError::Invariant { message } => {
                write!(formatter, "invariant violation: {message}")
            }
            AutospecError::Io {
                operation,
                path,
                source,
            } => write!(formatter, "failed to {operation} {path}: {source}"),
            AutospecError::Other { message } => write!(formatter, "{message}"),
        }
    }
}

impl Error for AutospecError {}

impl From<String> for AutospecError {
    fn from(message: String) -> Self {
        AutospecError::Other { message }
    }
}

impl From<&str> for AutospecError {
    fn from(message: &str) -> Self {
        AutospecError::Other {
            message: message.to_string(),
        }
    }
}

impl From<std::io::Error> for AutospecError {
    fn from(error: std::io::Error) -> Self {
        AutospecError::Other {
            message: error.to_string(),
        }
    }
}

impl From<AutospecError> for String {
    fn from(error: AutospecError) -> Self {
        error.to_string()
    }
}
