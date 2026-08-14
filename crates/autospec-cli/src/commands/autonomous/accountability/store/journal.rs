use super::*;

pub(super) fn parse_state(document: &str) -> Result<State, AccountabilityError> {
    let value: Value = serde_json::from_str(document).map_err(|error| {
        AccountabilityError::new(format!("invalid accountability state: {error}"))
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| AccountabilityError::new("accountability state must be an object"))?;
    let number = |name: &str| {
        object
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| AccountabilityError::new(format!("invalid state field {name}")))
    };
    if number("schema")? != ACCOUNTABILITY_SCHEMA {
        return Err(AccountabilityError::new(
            "unsupported accountability state schema",
        ));
    }
    let launch = match object.get("launch") {
        Some(Value::Null) | None => None,
        Some(value) => Some(LaunchDescriptor::from_value(value)?),
    };
    let optional_string = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_owned);
    let has_launch = launch.is_some();
    Ok(State {
        launch,
        epic_number: object.get("epic_number").and_then(Value::as_u64),
        epic_url: optional_string("epic_url"),
        event_count: number("event_count")?,
        last_seq: number("last_seq")?,
        projection_revision: number("projection_revision")?,
        desired_digest: optional_string("desired_digest"),
        desired_high_watermark: number("desired_high_watermark")?,
        acknowledged_high_watermark: number("acknowledged_high_watermark")?,
        pending_projection_count: number("pending_projection_count")?,
        journal_segment: number("journal_segment")?,
        prior_remote_digest: optional_string("prior_remote_digest"),
        segment_chain_digest: optional_string("segment_chain_digest").unwrap_or_default(),
        create_attempted: object
            .get("create_attempted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        resume_event_pending: object
            .get("resume_event_pending")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        lifecycle_phase: optional_string("lifecycle_phase").unwrap_or_else(|| {
            if has_launch {
                "bound_not_spawned".to_owned()
            } else {
                String::new()
            }
        }),
        recovery_state: object
            .get("recovery_state")
            .and_then(Value::as_str)
            .map(RecoveryState::parse)
            .transpose()?
            .unwrap_or_default(),
        linked_issues: object
            .get("linked_issues")
            .map(parse_links)
            .transpose()?
            .unwrap_or_default(),
        linked_pull_requests: object
            .get("linked_pull_requests")
            .map(parse_links)
            .transpose()?
            .unwrap_or_default(),
        last_projected_at: object.get("last_projected_at").and_then(Value::as_u64),
        next_projection_retry_at: object
            .get("next_projection_retry_at")
            .and_then(Value::as_u64),
        projection_retry_attempt: object
            .get("projection_retry_attempt")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        created_at: object
            .get("created_at")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        updated_at: object
            .get("updated_at")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

pub(super) fn insert_link(links: &mut Vec<u64>, value: u64) {
    if let Err(index) = links.binary_search(&value) {
        links.insert(index, value);
    }
}

pub(super) fn validate_links(values: &[u64]) -> Result<(), AccountabilityError> {
    if values.contains(&0) || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(AccountabilityError::new(
            "linked identifiers must be positive, sorted, and unique",
        ));
    }
    Ok(())
}

pub(super) fn parse_links(value: &Value) -> Result<Vec<u64>, AccountabilityError> {
    let values = value
        .as_array()
        .ok_or_else(|| AccountabilityError::new("linked identifiers must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| AccountabilityError::new("linked identifier must be unsigned"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_links(&values)?;
    Ok(values)
}

pub(super) fn validate_epic_url(
    identity: &RunIdentity,
    epic_number: u64,
    value: &str,
) -> Result<String, AccountabilityError> {
    let expected = format!(
        "https://github.com/{}/issues/{epic_number}",
        identity.repository().as_str()
    );
    if value != expected {
        return Err(AccountabilityError::new(
            "epic URL must exactly match the manifest repository and issue number",
        ));
    }
    Ok(expected)
}

pub(super) fn enforce_cross_file_invariants(
    root: &Path,
    state: &State,
) -> Result<(), AccountabilityError> {
    if state.launch.is_some() {
        if !root.join(EVENTS_FILE).is_file() || !root.join(OUTBOX_FILE).is_file() {
            return Err(AccountabilityError::new(
                "accountability metadata requires both journal and outbox files",
            ));
        }
        if state.journal_segment == 0 || state.segment_chain_digest.len() != 64 {
            return Err(AccountabilityError::new(
                "invalid journal segment chain metadata",
            ));
        }
    }
    Ok(())
}

pub(super) fn reconcile_outbox(root: &Path, state: &mut State) -> Result<(), AccountabilityError> {
    if state.launch.is_none() {
        return Ok(());
    }
    let path = root.join(OUTBOX_FILE);
    let document = read_private_file(&path)?;
    if document.is_empty() {
        if state.pending_projection_count > 0 {
            return Err(AccountabilityError::new(
                "pending projection metadata has no durable outbox record",
            ));
        }
        return Ok(());
    }
    let value: Value = serde_json::from_str(document.trim_end())
        .map_err(|error| AccountabilityError::new(format!("invalid projection outbox: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| AccountabilityError::new("projection outbox must be an object"))?;
    let revision = super::unsigned(object, "revision")?;
    let digest = super::string(object, "digest")?;
    let high_watermark = super::unsigned(object, "desired_high_watermark")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AccountabilityError::new(
            "projection outbox digest is invalid",
        ));
    }
    if state.pending_projection_count == 0
        && state.projection_revision == revision
        && state.acknowledged_high_watermark >= high_watermark
    {
        atomic_write(&path, b"")?;
    } else if revision > state.projection_revision && high_watermark >= state.desired_high_watermark
    {
        state.projection_revision = revision;
        state.desired_digest = Some(digest.to_owned());
        state.desired_high_watermark = high_watermark;
        state.pending_projection_count = 1;
    } else if state.pending_projection_count != 1
        || state.projection_revision != revision
        || state.desired_digest.as_deref() != Some(digest)
        || state.desired_high_watermark != high_watermark
    {
        return Err(AccountabilityError::new(
            "projection outbox and metadata disagree",
        ));
    }
    Ok(())
}

pub(super) fn recover_events(
    path: &Path,
    launch: Option<&LaunchDescriptor>,
    segment_chain_digest: &str,
    segment_base: u64,
) -> Result<Vec<EventRecord>, AccountabilityError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = private_options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        file.set_len(complete as u64).map_err(io_error)?;
        file.seek(SeekFrom::Start(complete as u64))
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        bytes.truncate(complete);
    }
    let Some(launch) = launch else {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        return Err(AccountabilityError::new(
            "event journal exists without launch identity",
        ));
    };
    let mut records = Vec::new();
    let mut prior = segment_base;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value = serde_json::from_slice(line).map_err(|error| {
            AccountabilityError::new(format!("invalid completed event line: {error}"))
        })?;
        let record =
            EventRecord::from_value(launch.identity.run_id(), segment_chain_digest, &value)?;
        if record.seq
            != prior
                .checked_add(1)
                .ok_or_else(|| AccountabilityError::new("event sequence overflow"))?
        {
            return Err(AccountabilityError::new(
                "event journal sequence is not monotonic",
            ));
        }
        prior = record.seq;
        records.push(record);
    }
    Ok(records)
}
