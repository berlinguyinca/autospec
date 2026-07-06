pub fn run(args: &[String]) -> Result<(), String> {
    if super::is_json(args) {
        super::json_status("plan", "ok");
    } else {
        println!("AutoSpec plan: V62+ package inspection available");
    }
    Ok(())
}
