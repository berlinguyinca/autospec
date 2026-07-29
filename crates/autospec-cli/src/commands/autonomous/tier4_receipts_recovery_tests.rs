use std::fs;

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::{sha256_hex, SealedEvidence, TierReceipt};

use super::tier2_receipts_tests::{store, TempRoot, REPO};
use super::tier4::Tier4Scan;
use super::tier4_receipts::{record_tier4, Tier4Progress};
use super::tier4_receipts_tests::{
    observation, produced_observation, record_tier4_with_expected_policy, seed_tier_four_cursor,
    source, tier4_store,
};

#[test]
fn changed_retry_clears_only_unreferenced_tier4_artifacts_before_writing() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert!(record_tier4(
        root.path(),
        REPO,
        Tier4Scan::Complete(observation(source(&[]), Vec::new(), Vec::new())),
    )
    .is_err());

    let tier4_dir = root.path().join("waterfall/waterfall/1/tier4");
    assert!(tier4_dir.join("sources.json").exists());
    assert!(!tier4_dir.join("tier4.json").exists());
    let unrelated = tier4_dir.join("leave-this-alone.json");
    fs::write(&unrelated, "unrelated orphan\n").expect("unrelated artifact");

    assert_eq!(
        record_tier4_with_expected_policy(&root, Tier4Scan::Complete(produced_observation()))
            .expect("changed retry clears conflicting artifacts"),
        Tier4Progress::Produced(1)
    );
    assert_eq!(
        fs::read_to_string(&unrelated).expect("unrelated artifact retained"),
        "unrelated orphan\n"
    );
    assert!(tier4_store(&root)
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt")
        .is_some());
}

#[test]
fn tier4_replay_rejects_rehashed_nested_key_order_variants() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4_with_expected_policy(&root, Tier4Scan::Complete(produced_observation()))
            .expect("produced receipt"),
        Tier4Progress::Produced(1)
    );
    let receipt_store = store(&root);
    let prior = receipt_store
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt")
        .expect("sealed receipt");
    let generated_path = root
        .path()
        .join("waterfall")
        .join(&prior.evidence()[2].reference);
    let original = fs::read_to_string(&generated_path).expect("generated evidence");
    let replacement_contents = original.replacen(
        "\"stable_key\":\"candidate-alpha\",\"source_id\":\"alpha\"",
        "\"source_id\":\"alpha\",\"stable_key\":\"candidate-alpha\"",
        1,
    );
    assert_ne!(
        replacement_contents, original,
        "fixture must reorder a nested object"
    );
    let mut evidence = prior.evidence().to_vec();
    evidence[2] = SealedEvidence::new(
        &evidence[2].reference,
        sha256_hex(replacement_contents.as_bytes()),
    )
    .expect("replacement digest");
    let mut replacements = vec![(generated_path, replacement_contents)];
    for index in 3..evidence.len() {
        let path = root
            .path()
            .join("waterfall")
            .join(&prior.evidence()[index].reference);
        let original = fs::read_to_string(&path).expect("chained evidence");
        let contents = original.replacen(
            &format!(
                "\"predecessor_digest\":\"{}\"",
                prior.evidence()[index - 1].digest
            ),
            &format!("\"predecessor_digest\":\"{}\"", evidence[index - 1].digest),
            1,
        );
        assert_ne!(
            contents, original,
            "chained fixture must update predecessor"
        );
        evidence[index] =
            SealedEvidence::new(&evidence[index].reference, sha256_hex(contents.as_bytes()))
                .expect("chained replacement digest");
        replacements.push((path, contents));
    }
    let forged = TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier4,
        prior.producer_version(),
        1,
        1,
        prior.status().clone(),
        prior.funnel().clone(),
        evidence,
    )
    .expect("rehashed receipt");
    for (path, contents) in replacements {
        fs::write(path, contents).expect("replace evidence");
    }
    fs::write(
        receipt_store.receipt_path(&prior).expect("receipt path"),
        format!("{}\n", forged.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);

    assert!(
        record_tier4(root.path(), REPO, Tier4Scan::NotRun).is_err(),
        "rehashed nested key order variants must not replay"
    );
}

#[test]
fn tier4_replay_rejects_source_envelopes_outside_the_checked_in_policy() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4_with_expected_policy(&root, Tier4Scan::Complete(produced_observation()))
            .expect("produced receipt"),
        Tier4Progress::Produced(1)
    );
    let receipt_store = store(&root);
    let prior = receipt_store
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt")
        .expect("sealed receipt");
    let policy_path = root
        .path()
        .join("waterfall")
        .join(&prior.evidence()[0].reference);
    let original = fs::read_to_string(&policy_path).expect("source policy evidence");
    let policy_contents = original.replacen("\"id\":\"alpha\"", "\"id\":\"beta\"", 1);
    assert_ne!(
        policy_contents, original,
        "fixture must replace the source policy identifier"
    );
    let mut evidence = prior.evidence().to_vec();
    evidence[0] = SealedEvidence::new(
        &evidence[0].reference,
        sha256_hex(policy_contents.as_bytes()),
    )
    .expect("source policy digest");
    let mut replacements = vec![(policy_path, policy_contents)];
    for index in 1..evidence.len() {
        let path = root
            .path()
            .join("waterfall")
            .join(&prior.evidence()[index].reference);
        let original = fs::read_to_string(&path).expect("chained evidence");
        let contents = original.replacen(
            &format!(
                "\"predecessor_digest\":\"{}\"",
                prior.evidence()[index - 1].digest
            ),
            &format!("\"predecessor_digest\":\"{}\"", evidence[index - 1].digest),
            1,
        );
        evidence[index] =
            SealedEvidence::new(&evidence[index].reference, sha256_hex(contents.as_bytes()))
                .expect("chained digest");
        replacements.push((path, contents));
    }
    let forged = TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier4,
        prior.producer_version(),
        1,
        1,
        prior.status().clone(),
        prior.funnel().clone(),
        evidence,
    )
    .expect("rehashed receipt");
    for (path, contents) in replacements {
        fs::write(path, contents).expect("replace evidence");
    }
    fs::write(
        receipt_store.receipt_path(&prior).expect("receipt path"),
        format!("{}\n", forged.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);

    assert!(
        record_tier4(root.path(), REPO, Tier4Scan::NotRun).is_err(),
        "source envelopes must remain bound to the checked-in policy"
    );
}

#[test]
fn tier4_replay_rejects_missing_extra_reordered_and_rehashed_receipt_evidence() {
    assert_receipt_rejected(|prior| {
        receipt_with(
            prior,
            prior.producer_version(),
            prior.evidence()[..5].to_vec(),
        )
    });
    assert_receipt_rejected(|prior| {
        let mut evidence = prior.evidence().to_vec();
        evidence.push(prior.evidence()[5].clone());
        receipt_with(prior, prior.producer_version(), evidence)
    });
    assert_receipt_rejected(|prior| {
        let mut evidence = prior.evidence().to_vec();
        evidence.swap(0, 1);
        receipt_with(prior, prior.producer_version(), evidence)
    });
    assert_receipt_rejected(|prior| {
        let mut evidence = prior.evidence().to_vec();
        evidence[0] =
            SealedEvidence::new(&evidence[0].reference, "b".repeat(64)).expect("forged digest");
        receipt_with(prior, prior.producer_version(), evidence)
    });
    assert_receipt_rejected(|prior| {
        receipt_with(prior, "forged-tier4-producer-v1", prior.evidence().to_vec())
    });
}

#[test]
fn tier4_replay_rejects_raw_body_fields_and_altered_predecessors() {
    assert_document_rejected(1, |contents| {
        contents.replacen(
            "\"body_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"facts\"",
            "\"body_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"raw_body\":\"forbidden\",\"facts\"",
            1,
        )
    });
    assert_document_rejected(1, |contents| {
        contents.replacen(
            "\"predecessor_digest\":\"",
            "\"predecessor_digest\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
        )
    });
}

fn assert_receipt_rejected(forge: impl FnOnce(&TierReceipt) -> TierReceipt) {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4_with_expected_policy(&root, Tier4Scan::Complete(produced_observation()))
            .expect("produced receipt"),
        Tier4Progress::Produced(1)
    );
    let receipt_store = store(&root);
    let prior = receipt_store
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt")
        .expect("sealed receipt");
    let forged = forge(&prior);
    fs::write(
        receipt_store.receipt_path(&prior).expect("receipt path"),
        format!("{}\n", forged.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(record_tier4(root.path(), REPO, Tier4Scan::NotRun).is_err());
}

fn assert_document_rejected(index: usize, rewrite: impl FnOnce(String) -> String) {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4_with_expected_policy(&root, Tier4Scan::Complete(produced_observation()))
            .expect("produced receipt"),
        Tier4Progress::Produced(1)
    );
    let receipt_store = store(&root);
    let prior = receipt_store
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt")
        .expect("sealed receipt");
    let path = root
        .path()
        .join("waterfall")
        .join(&prior.evidence()[index].reference);
    let contents = rewrite(fs::read_to_string(&path).expect("evidence"));
    let mut evidence = prior.evidence().to_vec();
    evidence[index] =
        SealedEvidence::new(&evidence[index].reference, sha256_hex(contents.as_bytes()))
            .expect("changed evidence digest");
    let forged = receipt_with(&prior, prior.producer_version(), evidence);
    fs::write(path, contents).expect("replace evidence");
    fs::write(
        receipt_store.receipt_path(&prior).expect("receipt path"),
        format!("{}\n", forged.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(record_tier4(root.path(), REPO, Tier4Scan::NotRun).is_err());
}

fn receipt_with(prior: &TierReceipt, producer: &str, evidence: Vec<SealedEvidence>) -> TierReceipt {
    TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier4,
        producer,
        1,
        1,
        prior.status().clone(),
        prior.funnel().clone(),
        evidence,
    )
    .expect("syntactic forged receipt")
}
