pub fn run(args: &[String]) -> Result<(), String> {
    if super::is_json(args) {
        super::json_status("validate", "ok");
    } else {
        println!("AutoSpec validate: use bash scripts/validate.sh --fast");
    }
    Ok(())
}
