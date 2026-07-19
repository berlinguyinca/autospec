pub fn run(args: &[String]) -> Result<(), String> {
    let report = autospec_core::growth::GrowthReport::new(
        true,
        true,
        true,
        true,
        true,
        vec![
            "marketing/launch-post-github.md".to_string(),
            "marketing/launch-post-reddit.md".to_string(),
            "marketing/launch-post-hackernews.md".to_string(),
            "marketing/launch-post-linkedin.md".to_string(),
            "marketing/launch-post-x.md".to_string(),
        ],
    );
    if super::is_json(args) {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.to_markdown());
    }
    Ok(())
}
