pub fn run(args: &[String]) -> Result<(), String> {
    if super::is_json(args) {
        super::json_status("report", "ok");
    } else {
        println!("AutoSpec report: release report rendering available in core");
    }
    Ok(())
}
