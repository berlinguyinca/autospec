pub fn run(args: &[String]) -> Result<(), String> {
    if super::is_json(args) {
        super::json_status("status", "ok");
    } else {
        println!("AutoSpec status: ok");
    }
    Ok(())
}
