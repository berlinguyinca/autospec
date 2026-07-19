pub mod model;
pub mod parser;

pub use model::{is_valid_spec_id, SpecId, SpecMetadata, SpecStatus, SpecVersion};
pub use parser::{parse_spec, ParseError, ParseErrorKind};
