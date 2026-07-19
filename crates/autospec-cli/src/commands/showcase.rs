pub fn run(args: &[String]) -> Result<(), String> {
    let demo = args
        .windows(2)
        .find_map(|window| (window[0] == "--demo").then(|| window[1].as_str()))
        .unwrap_or("examples/hello-autospec");
    if super::is_json(args) {
        println!(
            "{{\"command\":\"showcase\",\"status\":\"ok\",\"demo\":\"{}\",\"network\":\"disabled\"}}",
            demo.replace('"', "\\\"")
        );
    } else {
        println!("AutoSpec showcase: {demo}");
    }
    Ok(())
}
