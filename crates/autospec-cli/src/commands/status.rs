use autospec_core::state::{SpecRunState, SpecStateStore};

pub fn run(args: &[String]) -> Result<(), String> {
    let store = SpecStateStore::load_or_default(".")?;
    let counts = Counts::from_store(&store);

    if super::is_json(args) {
        println!(
            "{{\"command\":\"status\",\"status\":\"ok\",\"specs\":{{\"planned\":{},\"ready\":{},\"running\":{},\"passed\":{},\"failed\":{},\"blocked\":{},\"deferred\":{},\"superseded\":{}}}}}",
            counts.planned,
            counts.ready,
            counts.running,
            counts.passed,
            counts.failed,
            counts.blocked,
            counts.deferred,
            counts.superseded
        );
    } else {
        println!(
            "AutoSpec status: planned={} ready={} running={} passed={} failed={} blocked={} deferred={} superseded={}",
            counts.planned,
            counts.ready,
            counts.running,
            counts.passed,
            counts.failed,
            counts.blocked,
            counts.deferred,
            counts.superseded
        );
    }
    Ok(())
}

#[derive(Default)]
struct Counts {
    planned: usize,
    ready: usize,
    running: usize,
    passed: usize,
    failed: usize,
    blocked: usize,
    deferred: usize,
    superseded: usize,
}

impl Counts {
    fn from_store(store: &SpecStateStore) -> Self {
        let mut counts = Self::default();
        for lifecycle in store.iter() {
            match lifecycle.state {
                SpecRunState::Planned => counts.planned += 1,
                SpecRunState::Ready => counts.ready += 1,
                SpecRunState::Running => counts.running += 1,
                SpecRunState::Passed => counts.passed += 1,
                SpecRunState::Failed => counts.failed += 1,
                SpecRunState::Blocked => counts.blocked += 1,
                SpecRunState::Deferred => counts.deferred += 1,
                SpecRunState::Superseded => counts.superseded += 1,
            }
        }
        counts
    }
}
