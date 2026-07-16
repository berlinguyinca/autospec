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
    if contains_std_path_component(&tokens, module) {
        return true;
    }
    std_use_tree_contains(&tokens, module)
}

pub(crate) fn has_module_escape(code: &str) -> bool {
    let tokens = code_tokens(code);
    has_attribute_path(&tokens) || contains_tokens(&tokens, &["include", "!"])
}

fn contains_tokens(tokens: &[String], expected: &[&str]) -> bool {
    tokens.windows(expected.len()).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
    })
}

fn contains_std_path_component(tokens: &[String], module: &str) -> bool {
    for start in 0..tokens.len().saturating_sub(2) {
        if tokens[start].as_str() != "std" || tokens[start + 1].as_str() != "::" {
            continue;
        }
        let mut cursor = start + 2;
        while cursor < tokens.len() {
            if tokens[cursor] == module {
                return true;
            }
            if tokens.get(cursor + 1).is_some_and(|token| token == "::") {
                cursor += 2;
            } else {
                break;
            }
        }
    }
    false
}

fn std_use_tree_contains(tokens: &[String], module: &str) -> bool {
    for start in 0..tokens.len().saturating_sub(3) {
        if tokens[start].as_str() != "use"
            || tokens[start + 1].as_str() != "std"
            || tokens[start + 2].as_str() != "::"
        {
            continue;
        }
        for cursor in start + 3..tokens.len() {
            let token = tokens[cursor].as_str();
            if token == ";" {
                break;
            }
            if token == module
                && matches!(
                    tokens.get(cursor.wrapping_sub(1)).map(String::as_str),
                    Some("::" | "{" | ",")
                )
            {
                return true;
            }
        }
    }
    false
}

fn has_attribute_path(tokens: &[String]) -> bool {
    for start in 0..tokens.len().saturating_sub(2) {
        if tokens[start].as_str() != "#" || tokens[start + 1].as_str() != "[" {
            continue;
        }
        for token in &tokens[start + 2..] {
            if token == "]" {
                break;
            }
            if token == "path" {
                return true;
            }
        }
    }
    false
}

fn is_identifier_character(character: char) -> bool {
    character == '_' || character.is_ascii_alphanumeric()
}
