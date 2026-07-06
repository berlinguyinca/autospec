pub fn run(args: &[String]) -> Result<(), String> {
    if super::is_json(args) {
        print!(
            "{}",
            autospec_core::doctor_report_json().replace(
                "\"status\":\"ok\"",
                "\"command\":\"doctor\",\"status\":\"ok\""
            )
        );
    } else {
        println!("AutoSpec doctor: ok");
    }
    Ok(())
}
