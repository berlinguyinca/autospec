use std::collections::BTreeMap;

use autospec_core::state::json::{JsonParser, JsonValue};

#[derive(Debug)]
pub(super) enum ResilienceReject {
    MalformedState,
    ForeignState,
    MalformedFailure,
    MalformedSpend,
}

impl ResilienceReject {
    pub(super) fn reason(&self) -> &'static str {
        match self {
            Self::MalformedState => "malformed_state",
            Self::ForeignState => "foreign_state",
            Self::MalformedFailure => "malformed_failure",
            Self::MalformedSpend => "malformed_spend",
        }
    }
}

#[derive(Default)]
pub(super) struct Spend {
    pub(super) tokens: u64,
    pub(super) issues: u64,
}

impl Spend {
    pub(super) fn parse(raw: &str) -> Result<Self, ResilienceReject> {
        let mut fields = parse_json_object(raw).map_err(|_| ResilienceReject::MalformedSpend)?;
        let schema = number_field(&mut fields, "schema")
            .map_err(|_| ResilienceReject::MalformedSpend)?
            .ok_or(ResilienceReject::MalformedSpend)?;
        if schema != 1 {
            return Err(ResilienceReject::MalformedSpend);
        }
        Ok(Self {
            tokens: number_field(&mut fields, "tokens")
                .map_err(|_| ResilienceReject::MalformedSpend)?
                .ok_or(ResilienceReject::MalformedSpend)?,
            issues: number_field(&mut fields, "issues")
                .map_err(|_| ResilienceReject::MalformedSpend)?
                .ok_or(ResilienceReject::MalformedSpend)?,
        })
    }
}

pub(super) struct ResilienceState {
    pub(super) repo: String,
    pub(super) status: String,
    pub(super) host: Option<String>,
    pub(super) session: Option<String>,
    pub(super) heartbeat_at: Option<u64>,
    pub(super) lock_pid: Option<u32>,
    pub(super) lock_host: Option<String>,
    pub(super) lock_session: Option<String>,
    pub(super) lock_acquired_at: Option<u64>,
    pub(super) lease_token: Option<String>,
    pub(super) lease_generation: Option<u64>,
}

pub(super) struct StatusState {
    pub(super) repo: String,
    pub(super) status: String,
    pub(super) heartbeat_at: Option<u64>,
    pub(super) cycle: Option<u64>,
}

impl StatusState {
    pub(super) fn parse(raw: &str) -> Result<Self, ()> {
        let mut fields = parse_json_object(raw)?;
        Ok(Self {
            repo: string_field(&mut fields, "repo")?.ok_or(())?,
            status: string_field(&mut fields, "status")?.ok_or(())?,
            heartbeat_at: number_field(&mut fields, "heartbeat_at")?,
            cycle: number_field(&mut fields, "cycle")?,
        })
    }
}

impl ResilienceState {
    pub(super) fn parse(raw: &str) -> Result<Self, ()> {
        let mut fields = parse_json_object(raw)?;
        let repo = string_field(&mut fields, "repo")?.ok_or(())?;
        let status = string_field(&mut fields, "status")?.ok_or(())?;
        let host = string_field(&mut fields, "host")?;
        let session = string_field(&mut fields, "session")?;
        let heartbeat_at = number_field(&mut fields, "heartbeat_at")?;
        let lock_pid = number_field(&mut fields, "lock_pid")?
            .map(|pid| match u32::try_from(pid) {
                Ok(0) | Err(_) => Err(()),
                Ok(pid) => Ok(pid),
            })
            .transpose()?;
        let lock_host = string_field(&mut fields, "lock_host")?;
        let lock_session = string_field(&mut fields, "lock_session")?;
        let lock_acquired_at = number_field(&mut fields, "lock_acquired_at")?;
        let lease_token = string_field(&mut fields, "lease_token")?;
        let lease_generation = number_field(&mut fields, "lease_generation")?;
        let state = Self {
            repo,
            status,
            host,
            session,
            heartbeat_at,
            lock_pid,
            lock_host,
            lock_session,
            lock_acquired_at,
            lease_token,
            lease_generation,
        };
        state.validate_lease_state()?;
        Ok(state)
    }

    pub(super) fn to_json(&self, slug: &str) -> String {
        format!(
            "{{\"repo\":\"{}\",\"slug\":\"{}\",\"status\":\"{}\",\"host\":{},\"session\":{},\"heartbeat_at\":{},\"lock_pid\":{},\"lock_host\":{},\"lock_session\":{},\"lock_acquired_at\":{},\"lease_token\":{},\"lease_generation\":{}}}\n",
            super::super::json_escape(&self.repo),
            super::super::json_escape(slug),
            super::super::json_escape(&self.status),
            optional_json_string(self.host.as_deref()),
            optional_json_string(self.session.as_deref()),
            optional_number(self.heartbeat_at),
            optional_number(self.lock_pid.map(u64::from)),
            optional_json_string(self.lock_host.as_deref()),
            optional_json_string(self.lock_session.as_deref()),
            optional_number(self.lock_acquired_at),
            optional_json_string(self.lease_token.as_deref()),
            optional_number(self.lease_generation),
        )
    }

    fn validate_lease_state(&self) -> Result<(), ()> {
        match (self.lease_token.as_deref(), self.lease_generation) {
            (Some(token), Some(generation))
                if !token.is_empty()
                    && generation > 0
                    && matches!(self.status.as_str(), "claimed" | "running")
                    && self
                        .heartbeat_at
                        .is_some_and(|heartbeat_at| heartbeat_at > 0)
                    && self.lock_pid.is_some()
                    && self
                        .lock_host
                        .as_deref()
                        .is_some_and(|host| !host.is_empty())
                    && self
                        .lock_session
                        .as_deref()
                        .is_none_or(|session| !session.is_empty())
                    && self
                        .lock_acquired_at
                        .is_some_and(|acquired_at| acquired_at > 0) =>
            {
                Ok(())
            }
            (None, Some(generation))
                if generation > 0
                    && self.status == "released"
                    && self.lock_pid.is_none()
                    && self.lock_host.is_none()
                    && self.lock_session.is_none()
                    && self.lock_acquired_at.is_none() =>
            {
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(()),
        }
    }
}

pub(super) fn parse_failures(raw: &str, issue: u64) -> Result<u8, ResilienceReject> {
    let mut fields = parse_json_object(raw).map_err(|_| ResilienceReject::MalformedFailure)?;
    let recorded_issue = failure_issue_field(&mut fields)?;
    if recorded_issue != issue {
        return Err(ResilienceReject::MalformedFailure);
    }
    let failures = number_field(&mut fields, "failures")
        .map_err(|_| ResilienceReject::MalformedFailure)?
        .ok_or(ResilienceReject::MalformedFailure)?;
    Ok(u8::try_from(failures).unwrap_or(u8::MAX))
}

fn failure_issue_field(fields: &mut BTreeMap<String, JsonValue>) -> Result<u64, ResilienceReject> {
    let value = fields
        .remove("issue")
        .ok_or(ResilienceReject::MalformedFailure)?;
    let issue = match value {
        JsonValue::Number(value) => value
            .parse::<u64>()
            .map_err(|_| ResilienceReject::MalformedFailure)?,
        JsonValue::String(value) => {
            parse_positive_decimal(&value).ok_or(ResilienceReject::MalformedFailure)?
        }
        _ => return Err(ResilienceReject::MalformedFailure),
    };
    if issue == 0 {
        return Err(ResilienceReject::MalformedFailure);
    }
    Ok(issue)
}

fn parse_positive_decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|character| character.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn string_field(fields: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<Option<String>, ()> {
    match fields.remove(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value.into_string(key).map(Some).map_err(|_| ()),
    }
}

fn number_field(fields: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<Option<u64>, ()> {
    match fields.remove(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value.into_number(key).map(Some).map_err(|_| ()),
    }
}

fn parse_json_object(input: &str) -> Result<BTreeMap<String, JsonValue>, ()> {
    JsonParser::new(input)
        .parse()
        .and_then(|value| value.into_object("resilience record"))
        .map_err(|_| ())
}

fn optional_number(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn optional_json_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", super::super::json_escape(value)))
        .unwrap_or_else(|| "null".to_string())
}
