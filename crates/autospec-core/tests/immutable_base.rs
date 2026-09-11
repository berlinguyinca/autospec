//! Shared-base refresh never disturbs a running reader (issue #3732).
//!
//! A "base refresh" (git fetch + reset --hard on the shared base) used to
//! mutate the directory in place while concurrently dispatched jobs were
//! copying it, so a job's copy failed mid-read ("tar: file changed as we
//! read it") and the job died. The durable fix: the writer publishes each
//! refresh as a new immutable generation directory and swaps the `current`
//! symlink onto it; a job pins the base by resolving the symlink once at
//! start and reads only the resolved real path, which carries the explicit
//! base sha. The acceptance scenarios:
//!
//! * dispatching N issues in a tight loop never produces the
//!   changed-as-we-read failure — a pinned base stays byte-stable while
//!   refreshes land;
//! * a concurrent refresh with 10 job start-ups leaves all 10 jobs alive;
//! * no write lands in any directory a running job can be reading.

// The module under test is `#[cfg(unix)]`; this suite follows it.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::immutable_base::{
    gc_generations, plan_gc, publish_generation, resolve_current, BaseError, BaseGeneration,
    CURRENT_LINK, GENERATIONS_DIR, SHA_RECORD,
};

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-immutable-base-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir is created");
    path
}

/// A 40-hex-digit sha that passes validation.
fn sha(tag: u32) -> String {
    format!(
        "{:040x}",
        u64::from(tag).wrapping_mul(0x9e37_79b9_7f4a_7c15)
    )
}

/// Fill a generation the way a base refresh would: one file whose content
/// is the sha, so any torn or mixed read is detectable.
fn prepare(sha: &str) -> impl FnOnce(&Path) -> Result<(), String> + '_ {
    move |dir| fs::write(dir.join("source.txt"), format!("{sha}\n")).map_err(|e| e.to_string())
}

fn publish(
    root: &Path,
    s: &str,
) -> Result<autospec_core::immutable_base::BasePublishReceipt, BaseError> {
    publish_generation(root, s, prepare(s))
}

/// Read every file in a pinned generation, the way a job tar-copying its
/// base would. Returns the sha the files carry; a changed-as-we-read read
/// would surface as a mismatch against the pinned generation's sha.
fn copy_base(pin: &autospec_core::immutable_base::PinnedBase) -> Result<String, String> {
    for entry in fs::read_dir(&pin.generation.path).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == SHA_RECORD || path.is_dir() {
            continue;
        }
        // The file's content is exactly the generation's sha; a torn or
        // mixed read would surface as a mismatch.
        let content =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if content != format!("{}\n", pin.generation.sha) {
            return Err(format!(
                "file {name} in {} does not match the pinned sha: {content:?}",
                pin.generation.path.display()
            ));
        }
    }
    Ok(pin.generation.sha.clone())
}

/// AC: dispatching N issues in a tight loop never produces the
/// changed-as-we-read failure.
#[test]
fn tight_loop_dispatch_never_reads_a_mutating_base() {
    let root = temp_dir("tight-loop");
    for i in 0..30u32 {
        let s = sha(i % 251 + 1);
        publish(&root, &s).expect("refresh publishes");
        // The job starts: pin once, then copy the base.
        let pin = resolve_current(&root).expect("job pins its base");
        // A refresh lands mid-copy: the writer publishes a new generation
        // and swaps the symlink.
        publish(&root, &s).expect("concurrent refresh publishes");
        let read = copy_base(&pin).expect("copy of a pinned base never fails");
        assert_eq!(read, pin.generation.sha, "job i={i} read a torn base");
    }
}

/// AC: a concurrent refresh with 10 job start-ups leaves all 10 jobs
/// alive.
#[test]
fn concurrent_refresh_with_ten_job_starts_keeps_all_ten_alive() {
    let root = temp_dir("ten-jobs");
    publish(&root, &sha(1)).expect("initial base");

    let mut handles = Vec::new();
    for job in 0..10u32 {
        let root = root.clone();
        handles.push(std::thread::spawn(move || -> Result<String, String> {
            // Job start: pin the base exactly once.
            let pin = resolve_current(&root).map_err(|e| e.to_string())?;
            let read = copy_base(&pin)?;
            if read != pin.generation.sha {
                return Err(format!("job {job} read a torn base"));
            }
            Ok(pin.generation.sha)
        }));
    }
    // One refresh at a time in a tight loop while the jobs start up.
    for i in 0..10u32 {
        publish(&root, &sha(2 + i)).expect("refresh publishes while jobs start");
    }
    let mut alive = 0u32;
    for handle in handles {
        let sha = handle
            .join()
            .expect("job thread panicked")
            .expect("job died");
        assert_eq!(sha.len(), 40, "job read a non-sha base");
        alive += 1;
    }
    assert_eq!(alive, 10, "some jobs died during concurrent refresh");
    // And the base still resolves for the next dispatch.
    let pin = resolve_current(&root).expect("base still resolvable after refreshes");
    assert_eq!(pin.generation.sha, sha(11));
}

/// AC: no write lands in any directory a running job can be reading.
/// The observable signature of a write into a directory is its mtime
/// changing; a pinned generation's directory mtime must be stable across
/// refreshes.
#[test]
fn no_write_reaches_a_pinned_generation() {
    let root = temp_dir("no-write");
    publish(&root, &sha(7)).expect("initial base");
    let pin = resolve_current(&root).expect("job pins its base");
    let pinned_dir = pin.generation.path.clone();
    let mtime_before = fs::metadata(&pinned_dir)
        .expect("pinned dir exists")
        .modified()
        .expect("mtime");

    for i in 0..5u32 {
        publish(&root, &sha(8 + i)).expect("refresh publishes");
    }

    let mtime_after = fs::metadata(&pinned_dir)
        .expect("pinned dir still exists")
        .modified()
        .expect("mtime");
    assert_eq!(
        mtime_before, mtime_after,
        "a refresh wrote into a directory a running job can read"
    );
    let read = copy_base(&pin).expect("pinned base still reads cleanly");
    assert_eq!(read, pin.generation.sha);
}

#[test]
fn publish_creates_generation_and_swaps_symlink() {
    let root = temp_dir("publish");
    let first = publish(&root, &sha(1)).expect("first publish");
    assert_eq!(first.generation.id, "0000000001");
    assert!(first.previous.is_none(), "first publish has no previous");
    assert_eq!(
        first.generation.path,
        root.join(GENERATIONS_DIR).join("gen-0000000001")
    );
    let target = fs::read_link(root.join(CURRENT_LINK)).expect("current link exists");
    assert_eq!(
        target,
        PathBuf::from(format!("{GENERATIONS_DIR}/gen-0000000001"))
    );
    let record = fs::read_to_string(first.generation.path.join(SHA_RECORD)).expect("sha record");
    assert_eq!(record.trim(), sha(1));

    let second = publish(&root, &sha(2)).expect("second publish");
    assert_eq!(second.generation.id, "0000000002");
    assert_eq!(
        second.previous.as_deref(),
        Some(first.generation.path.as_path())
    );
    // The old generation is untouched and still readable.
    let old = fs::read_to_string(first.generation.path.join("source.txt"))
        .expect("old generation intact");
    assert_eq!(old, format!("{}\n", sha(1)));
    let target = fs::read_link(root.join(CURRENT_LINK)).expect("current link swapped");
    assert_eq!(
        target,
        PathBuf::from(format!("{GENERATIONS_DIR}/gen-0000000002"))
    );
}

#[test]
fn resolve_current_pins_real_path_and_sha() {
    let root = temp_dir("resolve");
    let receipt = publish(&root, &sha(3)).expect("publish");
    let pin = resolve_current(&root).expect("resolve");
    assert_eq!(pin.generation.sha, sha(3));
    assert_eq!(pin.generation.id, "0000000001");
    assert_eq!(pin.generation.path, receipt.generation.path);
    // The pinned path is the real directory, not the symlink.
    assert!(!pin
        .generation
        .path
        .symlink_metadata()
        .expect("metadata")
        .file_type()
        .is_symlink());
}

#[test]
fn resolve_current_fails_closed_without_root() {
    let root = temp_dir("no-current");
    let err = resolve_current(&root).expect_err("no current link yet");
    assert!(matches!(err, BaseError::NoCurrent { .. }), "{err}");

    // A root whose current link is dangling also fails closed.
    fs::create_dir_all(root.join(GENERATIONS_DIR)).expect("generations dir");
    std::os::unix::fs::symlink("generations/gen-9999999999", root.join(CURRENT_LINK))
        .expect("dangling link");
    let err = resolve_current(&root).expect_err("dangling current");
    assert!(matches!(err, BaseError::NoCurrent { .. }), "{err}");
}

#[test]
fn resolve_current_rejects_generation_without_sha_record() {
    let root = temp_dir("no-sha");
    fs::create_dir_all(root.join(GENERATIONS_DIR).join("gen-0000000001")).expect("generation dir");
    std::os::unix::fs::symlink(
        format!("{GENERATIONS_DIR}/gen-0000000001"),
        root.join(CURRENT_LINK),
    )
    .expect("current link");
    let err = resolve_current(&root).expect_err("missing sha record");
    assert!(matches!(err, BaseError::MissingShaRecord { .. }), "{err}");
}

#[test]
fn publish_refuses_bad_sha_and_cleans_up_failed_prepare() {
    let root = temp_dir("bad-sha");
    let err = publish_generation(&root, "not a sha", prepare("deadbeef")).expect_err("bad sha");
    assert!(matches!(err, BaseError::BadSha { .. }), "{err}");
    assert!(
        !root.join(GENERATIONS_DIR).exists(),
        "no generation for a refused sha"
    );

    let s = sha(9);
    publish(&root, &s).expect("baseline publish");
    let before = fs::read_link(root.join(CURRENT_LINK)).expect("current before");
    let err = publish_generation(&root, &sha(10), |dir| {
        fs::write(dir.join("junk.txt"), b"half-written").expect("write junk");
        Err("simulated fetch failure".to_owned())
    })
    .expect_err("prepare fails");
    assert!(matches!(err, BaseError::Prepare { .. }), "{err}");
    let gens = root.join(GENERATIONS_DIR);
    let names: Vec<String> = fs::read_dir(&gens)
        .expect("generations dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "failed prepare leaves no generation behind");
    assert_eq!(
        fs::read_link(root.join(CURRENT_LINK)).expect("current after"),
        before,
        "failed publish must not move current"
    );
}

fn gen(id: i64, created_millis: i64) -> BaseGeneration {
    BaseGeneration {
        id: format!("{id:010}"),
        sha: sha(id as u32),
        path: PathBuf::from(format!("gen-{id:010}")),
        created_millis,
    }
}

#[test]
fn plan_gc_retains_current_kept_and_fresh() {
    let now = 10_000_000i64;
    let gens = vec![
        gen(1, 1_000_000),
        gen(2, 2_000_000),
        gen(3, 3_000_000),
        gen(4, now - 1000),
    ];
    // current = gen 2, keep = 1 (gen 4), min age = 1 hour (gen 4 also fresh).
    let plan = plan_gc(&gens, Some("0000000002"), 1, 3600, now);
    assert_eq!(plan.retain, vec!["0000000004", "0000000002"]);
    // Both vectors are in the plan's iteration order (newest first).
    assert_eq!(plan.remove, vec!["0000000003", "0000000001"]);
}

#[test]
fn plan_gc_clock_skew_is_fresh() {
    // A generation "published in the future" (clock skew) must not be
    // removable: its age saturates to zero.
    let now = 5_000_000i64;
    let gens = vec![gen(1, now + 999_999)];
    let plan = plan_gc(&gens, None, 0, 3600, now);
    assert!(plan.remove.is_empty());
    assert_eq!(plan.retain, vec!["0000000001"]);
}

#[test]
fn gc_removes_old_non_current_and_keeps_current() {
    let root = temp_dir("gc");
    let a = publish(&root, &sha(20)).expect("gen 1");
    let b = publish(&root, &sha(21)).expect("gen 2");
    publish(&root, &sha(22)).expect("gen 3, now current");
    // Age the two old generations past any min_age.
    let hour = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(3600);
    for path in [&a.generation.path, &b.generation.path] {
        let dir = fs::File::open(path).expect("open generation dir");
        dir.set_modified(hour).expect("age the generation");
    }
    let report = gc_generations(&root, 2, 0).expect("gc runs");
    assert_eq!(report.removed, vec![a.generation.path]);
    assert!(
        b.generation.path.exists(),
        "newest-kept generation must survive"
    );
    assert!(
        resolve_current(&root)
            .expect("current survives gc")
            .generation
            .sha
            == sha(22),
        "gc must never remove the current generation"
    );
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
}

#[test]
fn gc_skips_generation_without_sha_record() {
    let root = temp_dir("gc-nosha");
    publish(&root, &sha(30)).expect("gen 1");
    let orphan = root.join(GENERATIONS_DIR).join("gen-0000000002");
    fs::create_dir_all(&orphan).expect("orphan dir");
    let report = gc_generations(&root, 0, 0).expect("gc runs");
    assert!(
        orphan.exists(),
        "a generation without a sha record is not garbage"
    );
    let skipped: Vec<&std::path::Path> = report.skipped.iter().map(|(p, _)| p.as_path()).collect();
    assert_eq!(skipped, vec![orphan.as_path()]);
}
