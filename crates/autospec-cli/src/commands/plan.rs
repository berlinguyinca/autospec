use autospec_core::spec::{parse_spec, SpecMetadata};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_INPUT: &str = ".autospec/generated-spec-packages/v62-final-platform";

#[derive(Debug)]
struct Options {
    input: PathBuf,
    json: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    let specs = load_specs(&options.input)?;

    if options.json {
        render_json(&options.input, &specs);
    } else {
        render_text(&options.input, &specs);
    }

    Ok(())
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let current_directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let mut input = current_directory.join(DEFAULT_INPUT);
    let mut json = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--input" => {
                index += 1;
                let value = args
                    .get(index)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| "autospec plan --input requires a path".to_string())?;
                input = PathBuf::from(value);
            }
            option => return Err(format!("unknown autospec plan option: {option}")),
        }
        index += 1;
    }

    Ok(Options { input, json })
}

fn load_specs(input: &Path) -> Result<Vec<SpecMetadata>, String> {
    if !input.is_dir() {
        return Err(format!(
            "autospec plan input is not a package directory: {}",
            input.display()
        ));
    }

    let specs_directory = input.join("specs");
    if !specs_directory.is_dir() {
        return Err(format!(
            "generated spec package has no specs directory: {}",
            specs_directory.display()
        ));
    }

    let mut paths = Vec::new();
    collect_spec_paths(&specs_directory, &mut paths)?;
    paths.sort();

    if paths.is_empty() {
        return Err(format!(
            "no generated specs found under: {}",
            specs_directory.display()
        ));
    }

    paths
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            parse_spec(&source)
                .map_err(|error| format!("could not parse {}: {}", path.display(), error.message))
        })
        .collect()
}

fn collect_spec_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("could not read directory entry: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;

        if file_type.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            paths.push(path);
        }
    }

    Ok(())
}

fn render_text(input: &Path, specs: &[SpecMetadata]) {
    println!("AutoSpec plan: {} generated spec(s)", specs.len());
    println!("Input: {}", input.display());
    for spec in specs {
        println!(
            "- {} ({}) {}",
            spec.id.as_str(),
            spec.version.as_str(),
            spec.title
        );
    }
}

fn render_json(input: &Path, specs: &[SpecMetadata]) {
    let rendered_specs = specs
        .iter()
        .map(SpecMetadata::to_json)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"command\":\"plan\",\"input\":\"{}\",\"spec_count\":{},\"specs\":[{}]}}",
        escape_json(&input.display().to_string()),
        specs.len(),
        rendered_specs
    );
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character if character.is_control() => {
                format!("\\u{:04x}", character as u32).chars().collect()
            }
            other => vec![other],
        })
        .collect()
}
