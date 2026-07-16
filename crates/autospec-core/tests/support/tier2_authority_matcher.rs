pub(crate) fn code_tokens(code: &str) -> Vec<String> {
    let characters = code.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        if is_identifier_character(characters[index]) {
            let start = index;
            while index < characters.len() && is_identifier_character(characters[index]) {
                index += 1;
            }
            tokens.push(characters[start..index].iter().collect());
        } else if characters[index] == ':' && characters.get(index + 1) == Some(&':') {
            tokens.push("::".to_string());
            index += 2;
        } else if !characters[index].is_whitespace() {
            tokens.push(characters[index].to_string());
            index += 1;
        } else {
            index += 1;
        }
    }
    tokens
}

pub(crate) fn contains_qualified_path(code: &str, path: &str) -> bool {
    contains_tokens(&code_tokens(code), &[path, "::"])
}

pub(crate) fn contains_path_symbol(code: &str, path: &str) -> bool {
    let mut expected = Vec::new();
    for (index, component) in path.split("::").enumerate() {
        if index > 0 {
            expected.push("::");
        }
        expected.push(component);
    }
    contains_tokens(&code_tokens(code), &expected)
}

pub(crate) fn has_forbidden_std_module(code: &str, module: &str) -> bool {
    let tokens = code_tokens(code);
    if contains_tokens(&tokens, &["std", "::", module]) {
        return true;
    }
    for start in 0..tokens.len().saturating_sub(2) {
        if tokens[start].as_str() == "std"
            && tokens[start + 1].as_str() == "::"
            && tokens[start + 2].as_str() == "{"
        {
            let mut depth = 1;
            let mut previous = "{";
            for token in &tokens[start + 3..] {
                match token.as_str() {
                    "{" => depth += 1,
                    "}" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ if depth == 1 && (previous == "{" || previous == ",") && token == module => {
                        return true;
                    }
                    _ => {}
                }
                previous = token;
            }
        }
    }
    false
}

pub(crate) fn has_module_escape(code: &str) -> bool {
    let tokens = code_tokens(code);
    contains_tokens(&tokens, &["#", "[", "path"]) || contains_tokens(&tokens, &["include", "!"])
}

fn contains_tokens(tokens: &[String], expected: &[&str]) -> bool {
    tokens.windows(expected.len()).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
    })
}

fn is_identifier_character(character: char) -> bool {
    character == '_' || character.is_ascii_alphanumeric()
}
