use autospec_core::state::{SpecLifecycle, SpecStateStore};

#[derive(Debug)]
struct Options {
    specs: Vec<String>,
    json: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    if options.specs.is_empty() {
        return Err("autospec init requires at least one --spec <id>".to_string());
    }

    let store = SpecStateStore::initialize_if_absent(
        ".",
        options.specs.into_iter().map(SpecLifecycle::new),
    )?;
    let spec_count = store.iter().count();

    if options.json {
        println!("{{\"command\":\"init\",\"status\":\"initialized\",\"spec_count\":{spec_count}}}");
    } else {
        println!("AutoSpec initialized {spec_count} planned spec(s)");
    }
    Ok(())
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut specs = Vec::new();
    let mut json = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--spec" => {
                index += 1;
                let spec = args
                    .get(index)
                    .filter(|spec| !spec.is_empty() && !spec.starts_with("--"))
                    .ok_or_else(|| "autospec init --spec requires an id".to_string())?;
                specs.push(spec.clone());
            }
            option => return Err(format!("unknown autospec init option: {option}")),
        }
        index += 1;
    }

    Ok(Options { specs, json })
}
