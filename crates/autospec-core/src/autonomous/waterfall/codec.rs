use std::collections::BTreeMap;

use crate::state::json::{JsonParser, JsonValue};

use super::{
    sha256, CompletedReceipt, FunnelCounts, SealedEvidence, TierReceipt, TierStatus,
    WaterfallState, WATERFALL_RECEIPT_SCHEMA, WATERFALL_STATE_SCHEMA,
};
use crate::autonomous::no_work::{DryReason, NoWorkTier};

pub(super) fn receipt_json(receipt: &TierReceipt) -> String {
    let mut preimage = receipt_preimage(receipt);
    let closing = preimage.pop().expect("receipt preimage is an object");
    debug_assert_eq!(closing, '}');
    format!("{preimage},\"digest\":\"{}\"}}", receipt.digest)
}

pub(super) fn receipt_digest(receipt: &TierReceipt) -> String {
    sha256::hex(receipt_preimage(receipt).as_bytes())
}

fn receipt_preimage(receipt: &TierReceipt) -> String {
    format!(
        concat!(
            "{{\"schema\":{},\"repo\":\"{}\",\"pass_id\":{},\"tier\":\"{}\",",
            "\"producer_version\":\"{}\",\"started_at\":{},\"completed_at\":{},",
            "\"status\":{},\"funnel\":{},\"evidence\":[{}]}}"
        ),
        WATERFALL_RECEIPT_SCHEMA,
        escape_json(&receipt.repo),
        receipt.pass_id,
        receipt.tier.as_str(),
        escape_json(&receipt.producer_version),
        receipt.started_at,
        receipt.completed_at,
        status_json(&receipt.status),
        funnel_json(&receipt.funnel),
        receipt
            .evidence
            .iter()
            .map(evidence_json)
            .collect::<Vec<_>>()
            .join(","),
    )
}

pub(super) fn parse_receipt(
    input: &str,
    expected_repo: &str,
    expected_pass_id: u64,
    expected_tier: NoWorkTier,
) -> Result<TierReceipt, String> {
    let context = "waterfall receipt";
    let mut object = JsonParser::new(input).parse()?.into_object(context)?;
    require_only_keys(
        &object,
        &[
            "schema",
            "repo",
            "pass_id",
            "tier",
            "producer_version",
            "started_at",
            "completed_at",
            "status",
            "funnel",
            "evidence",
            "digest",
        ],
        context,
    )?;
    let schema = take_required(&mut object, "schema", context)?.into_number("receipt.schema")?;
    if schema != WATERFALL_RECEIPT_SCHEMA {
        return Err(format!("unsupported waterfall receipt schema: {schema}"));
    }
    let repo = take_required(&mut object, "repo", context)?.into_string("receipt.repo")?;
    let pass_id = take_required(&mut object, "pass_id", context)?.into_number("receipt.pass_id")?;
    let tier = NoWorkTier::parse(
        &take_required(&mut object, "tier", context)?.into_string("receipt.tier")?,
    )?;
    let producer_version = take_required(&mut object, "producer_version", context)?
        .into_string("receipt.producer_version")?;
    let started_at =
        take_required(&mut object, "started_at", context)?.into_number("receipt.started_at")?;
    let completed_at =
        take_required(&mut object, "completed_at", context)?.into_number("receipt.completed_at")?;
    let status = parse_status(take_required(&mut object, "status", context)?)?;
    let funnel = parse_funnel(take_required(&mut object, "funnel", context)?)?;
    let evidence = parse_evidence(take_required(&mut object, "evidence", context)?)?;
    let digest = take_required(&mut object, "digest", context)?.into_string("receipt.digest")?;
    let receipt = TierReceipt {
        repo,
        pass_id,
        tier,
        producer_version,
        started_at,
        completed_at,
        status,
        funnel,
        evidence,
        digest,
    };
    receipt.validate()?;
    if receipt.repo != expected_repo {
        return Err("waterfall receipt repository does not match requested scope".to_string());
    }
    if receipt.pass_id != expected_pass_id {
        return Err("waterfall receipt pass does not match requested scope".to_string());
    }
    if receipt.tier != expected_tier {
        return Err("waterfall receipt tier does not match requested scope".to_string());
    }
    Ok(receipt)
}

pub(super) fn state_json(state: &WaterfallState) -> String {
    format!(
        "{{\"schema\":{},\"repo\":\"{}\",\"next_pass_id\":{},\"current_tier\":\"{}\",\"completed_receipts\":[{}]}}",
        WATERFALL_STATE_SCHEMA,
        escape_json(&state.repo),
        state.next_pass_id,
        state.current_tier.as_str(),
        state
            .completed_receipts
            .iter()
            .map(completed_receipt_json)
            .collect::<Vec<_>>()
            .join(","),
    )
}

pub(super) fn parse_state(input: &str, expected_repo: &str) -> Result<WaterfallState, String> {
    let context = "waterfall state";
    let mut object = JsonParser::new(input).parse()?.into_object(context)?;
    require_only_keys(
        &object,
        &[
            "schema",
            "repo",
            "next_pass_id",
            "current_tier",
            "completed_receipts",
        ],
        context,
    )?;
    let schema = take_required(&mut object, "schema", context)?.into_number("state.schema")?;
    if schema != WATERFALL_STATE_SCHEMA {
        return Err(format!("unsupported waterfall state schema: {schema}"));
    }
    let repo = take_required(&mut object, "repo", context)?.into_string("state.repo")?;
    if repo != expected_repo {
        return Err("waterfall state repository does not match requested scope".to_string());
    }
    let next_pass_id =
        take_required(&mut object, "next_pass_id", context)?.into_number("state.next_pass_id")?;
    let current_tier = NoWorkTier::parse(
        &take_required(&mut object, "current_tier", context)?.into_string("state.current_tier")?,
    )?;
    let completed_receipts =
        parse_completed_receipts(take_required(&mut object, "completed_receipts", context)?)?;
    let state = WaterfallState {
        repo,
        next_pass_id,
        current_tier,
        completed_receipts,
    };
    state.validate()?;
    Ok(state)
}

fn status_json(status: &TierStatus) -> String {
    match status {
        TierStatus::Exhausted { reason } => {
            format!(
                "{{\"kind\":\"exhausted\",\"reason\":\"{}\"}}",
                reason.as_str()
            )
        }
        TierStatus::Produced { count } => format!("{{\"kind\":\"produced\",\"count\":{count}}}"),
        TierStatus::Failed { reason } => format!(
            "{{\"kind\":\"failed\",\"reason\":\"{}\"}}",
            escape_json(reason)
        ),
        TierStatus::Blocked { reason } => format!(
            "{{\"kind\":\"blocked\",\"reason\":\"{}\"}}",
            escape_json(reason)
        ),
        TierStatus::NotRun { reason } => format!(
            "{{\"kind\":\"not_run\",\"reason\":\"{}\"}}",
            escape_json(reason)
        ),
    }
}

fn funnel_json(funnel: &FunnelCounts) -> String {
    format!(
        "{{\"observed\":{},\"deduplicated\":{},\"verified\":{},\"roi_approved\":{},\"ranked\":{}}}",
        funnel.observed, funnel.deduplicated, funnel.verified, funnel.roi_approved, funnel.ranked
    )
}

fn evidence_json(evidence: &SealedEvidence) -> String {
    format!(
        "{{\"reference\":\"{}\",\"digest\":\"{}\"}}",
        escape_json(&evidence.reference),
        evidence.digest
    )
}

fn completed_receipt_json(receipt: &CompletedReceipt) -> String {
    format!(
        "{{\"tier\":\"{}\",\"digest\":\"{}\",\"reference\":\"{}\"}}",
        receipt.tier.as_str(),
        receipt.digest,
        receipt.reference
    )
}

fn parse_status(value: JsonValue) -> Result<TierStatus, String> {
    let context = "waterfall receipt.status";
    let mut object = value.into_object(context)?;
    let kind = take_required(&mut object, "kind", context)?.into_string("receipt.status.kind")?;
    match kind.as_str() {
        "exhausted" => {
            require_only_keys(&object, &["reason"], context)?;
            Ok(TierStatus::Exhausted {
                reason: DryReason::parse(
                    &take_required(&mut object, "reason", context)?
                        .into_string("receipt.status.reason")?,
                )?,
            })
        }
        "produced" => {
            require_only_keys(&object, &["count"], context)?;
            Ok(TierStatus::Produced {
                count: take_required(&mut object, "count", context)?
                    .into_number("receipt.status.count")?,
            })
        }
        "failed" => Ok(TierStatus::Failed {
            reason: parse_reason(&mut object, context)?,
        }),
        "blocked" => Ok(TierStatus::Blocked {
            reason: parse_reason(&mut object, context)?,
        }),
        "not_run" => Ok(TierStatus::NotRun {
            reason: parse_reason(&mut object, context)?,
        }),
        _ => Err(format!("unknown waterfall receipt status: {kind}")),
    }
}

fn parse_reason(object: &mut BTreeMap<String, JsonValue>, context: &str) -> Result<String, String> {
    require_only_keys(object, &["reason"], context)?;
    take_required(object, "reason", context)?.into_string("receipt.status.reason")
}

fn parse_funnel(value: JsonValue) -> Result<FunnelCounts, String> {
    let context = "waterfall receipt.funnel";
    let mut object = value.into_object(context)?;
    require_only_keys(
        &object,
        &[
            "observed",
            "deduplicated",
            "verified",
            "roi_approved",
            "ranked",
        ],
        context,
    )?;
    FunnelCounts::new(
        take_required(&mut object, "observed", context)?.into_number("receipt.funnel.observed")?,
        take_required(&mut object, "deduplicated", context)?
            .into_number("receipt.funnel.deduplicated")?,
        take_required(&mut object, "verified", context)?.into_number("receipt.funnel.verified")?,
        take_required(&mut object, "roi_approved", context)?
            .into_number("receipt.funnel.roi_approved")?,
        take_required(&mut object, "ranked", context)?.into_number("receipt.funnel.ranked")?,
    )
}

fn parse_evidence(value: JsonValue) -> Result<Vec<SealedEvidence>, String> {
    value
        .into_array("waterfall receipt.evidence")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("waterfall receipt.evidence[{index}]");
            let mut object = value.into_object(&context)?;
            require_only_keys(&object, &["reference", "digest"], &context)?;
            SealedEvidence::new(
                take_required(&mut object, "reference", &context)?
                    .into_string(&format!("{context}.reference"))?,
                take_required(&mut object, "digest", &context)?
                    .into_string(&format!("{context}.digest"))?,
            )
        })
        .collect()
}

fn parse_completed_receipts(value: JsonValue) -> Result<Vec<CompletedReceipt>, String> {
    value
        .into_array("waterfall state.completed_receipts")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("waterfall state.completed_receipts[{index}]");
            let mut object = value.into_object(&context)?;
            require_only_keys(&object, &["tier", "digest", "reference"], &context)?;
            Ok(CompletedReceipt {
                tier: NoWorkTier::parse(
                    &take_required(&mut object, "tier", &context)?
                        .into_string(&format!("{context}.tier"))?,
                )?,
                digest: take_required(&mut object, "digest", &context)?
                    .into_string(&format!("{context}.digest"))?,
                reference: take_required(&mut object, "reference", &context)?
                    .into_string(&format!("{context}.reference"))?,
            })
        })
        .collect()
}

fn require_only_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        return Err(format!("unexpected {context} field: {key}"));
    }
    Ok(())
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {context} field: {key}"))
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character if character.is_control() => format!("\\u{:04x}", character as u32)
                .chars()
                .collect::<Vec<_>>(),
            character => vec![character],
        })
        .collect()
}
