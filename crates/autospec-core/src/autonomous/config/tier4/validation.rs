pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.is_ascii()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(super) fn valid_host(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 253
        || !value.is_ascii()
        || !value.contains('.')
        || ipv4_like_host(value)
    {
        return false;
    }

    value.split('.').all(valid_dns_label)
}

pub(super) fn valid_path(value: &str) -> bool {
    if !value.starts_with('/') || value.len() > 256 || value.contains(['?', '#', '\\']) {
        return false;
    }
    let segments = &value[1..];
    segments.is_empty()
        || segments
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

pub(super) fn parse_bounded_u32(
    value: &str,
    min: u32,
    max: u32,
    line: usize,
    field: &str,
) -> Result<u32, String> {
    let value = parse_scalar(value, line, field)?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(
            line,
            &format!("tier4 source {field} must be an unsigned decimal integer"),
        ));
    }
    let parsed = value.parse::<u32>().map_err(|_| {
        error(
            line,
            &format!("tier4 source {field} is outside the supported range"),
        )
    })?;
    if !(min..=max).contains(&parsed) {
        return Err(error(
            line,
            &format!("tier4 source {field} must be between {min} and {max}"),
        ));
    }
    Ok(parsed)
}

pub(super) fn parse_scalar(value: &str, line: usize, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(error(
            line,
            &format!("tier4 source {field} must be a non-empty scalar"),
        ));
    }
    if matches!(value.as_bytes().first(), Some(b'[' | b'{' | b'|' | b'>'))
        || value.starts_with("- ")
        || has_unquoted_mapping_delimiter(value)
    {
        return Err(error(
            line,
            &format!("tier4 source {field} must be a scalar value"),
        ));
    }
    let scalar = match value.as_bytes().first() {
        Some(b'\'' | b'\"') => {
            let quote = value.as_bytes()[0] as char;
            if value.len() < 2 || !value.ends_with(quote) {
                return Err(error(
                    line,
                    &format!("unterminated quoted tier4 source {field}"),
                ));
            }
            &value[1..value.len() - 1]
        }
        _ => value,
    }
    .trim();
    if scalar.is_empty() {
        return Err(error(
            line,
            &format!("tier4 source {field} must be a non-empty scalar"),
        ));
    }
    Ok(scalar.to_string())
}

pub(super) fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('\"') {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '\"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == '#'
            && quote.is_none()
            && (index == 0
                || line[..index]
                    .chars()
                    .last()
                    .is_some_and(char::is_whitespace))
        {
            return &line[..index];
        }
    }
    line
}

pub(super) fn error(line: usize, message: &str) -> String {
    format!("invalid .autospec/autonomous.yml at line {line}: {message}")
}

fn valid_dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn ipv4_like_host(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    (1..=4).contains(&parts.len()) && parts.iter().all(|part| ipv4_number(part))
}

fn ipv4_number(value: &str) -> bool {
    if let Some(hex) = value.strip_prefix("0x") {
        return !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn has_unquoted_mapping_delimiter(value: &str) -> bool {
    if matches!(value.as_bytes().first(), Some(b'\'' | b'\"')) {
        return false;
    }
    value.char_indices().any(|(index, character)| {
        character == ':'
            && value[index + character.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
    })
}
