pub(super) fn matches_document(
    contents: &str,
    expected_kind: &str,
    expected_keys: &[&str],
) -> bool {
    is_renderer_json(contents)
        && contents.starts_with(&format!("{{\"schema\":1,\"kind\":\"{expected_kind}\","))
        && top_level_keys(contents).as_deref() == Some(expected_keys)
}

pub(super) fn matches_object_key_orders(contents: &str, expected: &[&[&str]]) -> bool {
    let Some(orders) = object_key_orders(contents) else {
        return false;
    };
    orders.len() == expected.len()
        && orders.iter().zip(expected).all(|(actual, required)| {
            actual.len() == required.len()
                && actual
                    .iter()
                    .zip(*required)
                    .all(|(key, required)| key == required)
        })
}

fn is_renderer_json(contents: &str) -> bool {
    let Some(document) = contents.strip_suffix('\n') else {
        return false;
    };
    let bytes = document.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        match (in_string, bytes[index]) {
            (false, b'\"') => in_string = true,
            (false, byte) if byte.is_ascii_whitespace() => return false,
            (true, b'\"') => in_string = false,
            (true, b'\\') => match escape_width(bytes, index) {
                Some(width) => index += width,
                None => return false,
            },
            (true, byte) if byte < 0x20 => return false,
            _ => {}
        }
        index += 1;
    }
    !in_string
}

fn escape_width(bytes: &[u8], slash: usize) -> Option<usize> {
    match *bytes.get(slash + 1)? {
        b'\"' | b'\\' | b'n' | b'r' | b't' => Some(1),
        b'u' => {
            let digits = bytes.get(slash + 2..slash + 6)?;
            if digits[..2] != *b"00"
                || !digits[2..]
                    .iter()
                    .all(|digit| digit.is_ascii_digit() || matches!(digit, b'a'..=b'f'))
            {
                return None;
            }
            let code = u16::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
            (code < 0x20 && !matches!(code, 0x09 | 0x0a | 0x0d)).then_some(5)
        }
        _ => None,
    }
}

fn top_level_keys(contents: &str) -> Option<Vec<&str>> {
    let bytes = contents.strip_suffix('\n')?.as_bytes();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut expecting_key = false;
    let mut index = 0usize;
    while index < bytes.len() {
        if in_string {
            if bytes[index] == b'\\' {
                index += escape_width(bytes, index)?;
            } else if bytes[index] == b'\"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'{' | b'[' => {
                depth += 1;
                expecting_key = depth == 1 && bytes[index] == b'{';
            }
            b'}' | b']' => depth = depth.checked_sub(1)?,
            b',' if depth == 1 => expecting_key = true,
            b':' if depth == 1 => expecting_key = false,
            b'\"' if depth == 1 && expecting_key => {
                let start = index + 1;
                index = start;
                while let Some(byte) = bytes.get(index) {
                    if *byte == b'\"' {
                        break;
                    }
                    if bytes.get(index) == Some(&b'\\') {
                        return None;
                    }
                    index += 1;
                }
                (bytes.get(index) == Some(&b'\"')).then_some(())?;
                keys.push(std::str::from_utf8(&bytes[start..index]).ok()?);
            }
            b'\"' => in_string = true,
            _ => {}
        }
        index += 1;
    }
    (depth == 0 && !in_string).then_some(keys)
}

fn object_key_orders(contents: &str) -> Option<Vec<Vec<String>>> {
    let bytes = contents.strip_suffix('\n')?.as_bytes();
    let mut index = 0;
    let mut orders = Vec::new();
    scan_value(bytes, &mut index, &mut orders)?;
    (index == bytes.len()).then_some(orders)
}

fn scan_value(bytes: &[u8], index: &mut usize, orders: &mut Vec<Vec<String>>) -> Option<()> {
    match *bytes.get(*index)? {
        b'{' => scan_object(bytes, index, orders),
        b'[' => scan_array(bytes, index, orders),
        b'"' => scan_string(bytes, index),
        _ => {
            while bytes
                .get(*index)
                .is_some_and(|byte| !matches!(*byte, b',' | b']' | b'}'))
            {
                *index += 1;
            }
            Some(())
        }
    }
}

fn scan_object(bytes: &[u8], index: &mut usize, orders: &mut Vec<Vec<String>>) -> Option<()> {
    *index += 1;
    let order_index = orders.len();
    orders.push(Vec::new());
    if bytes.get(*index) == Some(&b'}') {
        *index += 1;
        return Some(());
    }
    loop {
        let key = scan_key(bytes, index)?;
        orders[order_index].push(key);
        (*bytes.get(*index)? == b':').then_some(())?;
        *index += 1;
        scan_value(bytes, index, orders)?;
        match *bytes.get(*index)? {
            b'}' => {
                *index += 1;
                return Some(());
            }
            b',' => *index += 1,
            _ => return None,
        }
    }
}

fn scan_array(bytes: &[u8], index: &mut usize, orders: &mut Vec<Vec<String>>) -> Option<()> {
    *index += 1;
    if bytes.get(*index) == Some(&b']') {
        *index += 1;
        return Some(());
    }
    loop {
        scan_value(bytes, index, orders)?;
        match *bytes.get(*index)? {
            b']' => {
                *index += 1;
                return Some(());
            }
            b',' => *index += 1,
            _ => return None,
        }
    }
}

fn scan_key(bytes: &[u8], index: &mut usize) -> Option<String> {
    (*bytes.get(*index)? == b'"').then_some(())?;
    *index += 1;
    let start = *index;
    while let Some(byte) = bytes.get(*index) {
        match *byte {
            b'"' => {
                let key = std::str::from_utf8(&bytes[start..*index]).ok()?.to_string();
                *index += 1;
                return Some(key);
            }
            b'\\' => return None,
            _ => *index += 1,
        }
    }
    None
}

fn scan_string(bytes: &[u8], index: &mut usize) -> Option<()> {
    (*bytes.get(*index)? == b'"').then_some(())?;
    *index += 1;
    while let Some(byte) = bytes.get(*index) {
        match *byte {
            b'"' => {
                *index += 1;
                return Some(());
            }
            b'\\' => *index += escape_width(bytes, *index)? + 1,
            _ => *index += 1,
        }
    }
    None
}
