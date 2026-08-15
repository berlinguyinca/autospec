use std::{env, fs};

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let command = args.first().map(String::as_str).unwrap_or_default();
    let subcommand = args.get(1).map(String::as_str).unwrap_or_default();
    match (command, subcommand) {
        ("issue", "edit") => {
            let state = env::var_os("AUTOSPEC_TEST_GH_STATE").expect("gh fixture state");
            if args
                .windows(2)
                .any(|pair| pair == ["--add-label", "in-progress-by-bot"])
            {
                fs::write(state, "claimed").expect("record claimed fixture state");
            } else if args
                .windows(2)
                .any(|pair| pair == ["--add-label", "auto-implement"])
            {
                fs::write(state, "released").expect("record released fixture state");
            }
        }
        ("issue", "view") => {
            if args.iter().any(|arg| arg == "labels,body,title,author") {
                let state = env::var_os("AUTOSPEC_TEST_GH_STATE")
                    .and_then(|path| fs::read_to_string(path).ok())
                    .unwrap_or_default();
                let label = if state == "claimed" {
                    "in-progress-by-bot"
                } else {
                    "auto-implement"
                };
                println!(
                    r###"{{"labels":["{label}","safety:reviewed"],"body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\nRun the portable admission.","title":"Portable admission","author":"fixture"}}"###
                );
            } else {
                println!(
                    "{}",
                    r#"{"labels":[{"name":"auto-implement"},{"name":"safety:reviewed"}]}"#
                );
            }
        }
        ("pr", "list") | ("api", _) => println!("[]"),
        _ => {}
    }
}
