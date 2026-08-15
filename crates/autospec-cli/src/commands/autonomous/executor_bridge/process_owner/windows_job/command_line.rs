use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub(super) fn command_line(argv: &[std::ffi::OsString]) -> Result<Vec<u16>, String> {
    let Some((argv_zero, arguments)) = argv.split_first() else {
        return Err("autonomous child argv is empty".to_string());
    };
    let mut encoded = Vec::new();
    push_quoted(&mut encoded, argv_zero)?;
    for argument in arguments {
        encoded.push(b' ' as u16);
        push_quoted(&mut encoded, argument)?;
    }
    encoded.push(0);
    Ok(encoded)
}

fn push_quoted(output: &mut Vec<u16>, value: &OsStr) -> Result<(), String> {
    let wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err("autonomous child command contains an embedded NUL".to_string());
    }
    let needs_quotes = wide.is_empty()
        || wide
            .iter()
            .any(|unit| *unit == b' ' as u16 || *unit == b'\t' as u16 || *unit == b'"' as u16);
    if !needs_quotes {
        output.extend(wide);
        return Ok(());
    }
    output.push(b'"' as u16);
    let mut slashes = 0;
    for unit in wide {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else {
            if unit == b'"' as u16 {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            } else {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            }
            slashes = 0;
            output.push(unit);
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    Ok(())
}

pub(super) fn environment_block(
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<Vec<u16>, String> {
    let mut environment = environment.to_vec();
    environment.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase());
    if environment.is_empty() {
        return Ok(vec![0, 0]);
    }
    let mut block = Vec::new();
    for (key, value) in environment {
        block.extend(wide(&key, "autonomous child environment key")?);
        block.push(b'=' as u16);
        block.extend(wide(&value, "autonomous child environment value")?);
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

pub(super) fn wide_nul(value: &OsStr, label: &str) -> Result<Vec<u16>, String> {
    let mut encoded = wide(value, label)?;
    encoded.push(0);
    Ok(encoded)
}

fn wide(value: &OsStr, label: &str) -> Result<Vec<u16>, String> {
    let encoded: Vec<u16> = value.encode_wide().collect();
    if encoded.contains(&0) {
        Err(format!("{label} contains an embedded NUL"))
    } else {
        Ok(encoded)
    }
}
