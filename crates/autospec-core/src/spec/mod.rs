pub mod authority;
pub mod model;
pub mod parser;
pub mod review;
pub mod signals;

pub use model::{is_valid_spec_id, SpecId, SpecMetadata, SpecStatus, SpecVersion};
pub use parser::{parse_spec, ParseError, ParseErrorKind};
pub use review::{lint_spec_safety, SpecSafetyFinding, SAFETY_WITHOUT_MECHANISM_RULE_ID};
pub use signals::{
    lint_consumed_signals, validate_rejection, CitedSignal, Rejection, RejectionRefusal,
    SignalSemanticsFinding, CONSUMED_SIGNALS_SECTION, CONSUMED_SIGNAL_INCOMPLETE_RULE_ID,
    SIGNAL_FIELDS,
};
