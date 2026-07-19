use autospec_core::evidence::ReleaseReport;
use autospec_core::state::SpecStateStore;

pub fn run(args: &[String]) -> Result<(), String> {
    let store = SpecStateStore::load_or_default(".")?;
    let states = store.iter().cloned().collect::<Vec<_>>();
    let report = ReleaseReport::from_states("current", &states)?;

    if super::is_json(args) {
        let report = report.to_json();
        let fields = report
            .strip_prefix('{')
            .ok_or_else(|| "release report JSON must be an object".to_string())?;
        println!("{{\"command\":\"report\",{fields}");
    } else {
        print!("{}", report.to_markdown());
    }
    Ok(())
}
