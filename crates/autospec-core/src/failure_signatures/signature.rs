//! Turning a stderr line into a counting key.

/// Longest normalized signature kept; longer lines are truncated on a char
/// boundary so grouping never splits a multi-byte character.
const MAX_SIGNATURE_BYTES: usize = 160;

/// True when `line` was emitted by the scheduler rather than by the job.
///
/// Slurm decorates every job's stderr with its own lines (`slurmstepd:`,
/// `slurm_print_exit_info`, `srun:`), and those describe the allocation, not
/// the failure. They are the first thing an operator learns to skim past, so
/// they must never become a signature.
pub fn is_slurm_noise(line: &str) -> bool {
    let lower = line.trim_start().to_ascii_lowercase();
    [
        "slurm",
        "srun:",
        "sbatch:",
        "scontrol:",
        "sacct:",
        "mpibind",
        "pmix:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || lower.contains("slurmstepd")
        || lower.contains("slurm_load_jobs")
}

/// The last stderr line that came from the job itself, `None` when the log
/// holds nothing but scheduler noise or whitespace.
pub fn last_meaningful_line(stderr: &str) -> Option<&str> {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty() && !is_slurm_noise(line))
}

/// Reduce a stderr line to a counting key: paths masked to `<path>/`, numbers
/// to `<n>`, hex addresses to `<addr>`, UUIDs to `<uuid>`, dates to `<date>`.
///
/// The masking is what makes two runs of the same crash collide: the same
/// compiler error on different nodes at different line numbers, the same
/// allocator failure under different PIDs. It deliberately keeps the file name
/// and the message text, because that is the part that identifies the defect.
pub fn normalize_signature_line(line: &str) -> String {
    let mut joined = String::new();
    for token in line.split_whitespace() {
        if !joined.is_empty() {
            joined.push(' ');
        }
        joined.push_str(&normalize_token(token));
    }
    truncate_signature(&joined)
}

fn normalize_token(token: &str) -> String {
    if is_uuid(token) {
        return "<uuid>".to_string();
    }
    let mut masked = mask_path_prefix(token);
    if is_date(&masked) {
        return "<date>".to_string();
    }
    masked = mask_hex_addresses(&masked);
    mask_digits(&masked)
}

/// Replace the directory portion of a path-bearing token with `<path>/`,
/// keeping the final component (the file name usually identifies the failure).
fn mask_path_prefix(token: &str) -> String {
    match token.rfind('/') {
        Some(index) if index + 1 < token.len() => format!("<path>/{}", &token[index + 1..]),
        Some(_) => "<path>".to_string(),
        None => token.to_string(),
    }
}

fn mask_hex_addresses(token: &str) -> String {
    if !token.contains("0x") && !token.contains("0X") {
        return token.to_string();
    }
    let bytes: Vec<char> = token.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < bytes.len() {
        match hex_run_at(&bytes, index) {
            Some(end) => {
                out.push_str("<addr>");
                index = end;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    out
}

/// End of the `0x…` hex run starting at `index`, `None` when there is none.
fn hex_run_at(bytes: &[char], index: usize) -> Option<usize> {
    if bytes[index] != '0' || index + 1 >= bytes.len() {
        return None;
    }
    if bytes[index + 1] != 'x' && bytes[index + 1] != 'X' {
        return None;
    }
    let mut end = index + 2;
    while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
        end += 1;
    }
    (end > index + 2).then_some(end)
}

fn mask_digits(token: &str) -> String {
    if !token.chars().any(|c| c.is_ascii_digit()) {
        return token.to_string();
    }
    let mut out = String::with_capacity(token.len());
    let mut in_digits = false;
    for c in token.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push_str("<n>");
                in_digits = true;
            }
        } else {
            in_digits = false;
            out.push(c);
        }
    }
    out
}

fn is_date(token: &str) -> bool {
    let bytes: Vec<char> = token.chars().collect();
    bytes.len() >= 10
        && bytes[..4].iter().all(|c| c.is_ascii_digit())
        && bytes[4] == '-'
        && bytes[5..7].iter().all(|c| c.is_ascii_digit())
        && bytes[7] == '-'
        && bytes[8..10].iter().all(|c| c.is_ascii_digit())
}

fn is_uuid(token: &str) -> bool {
    let groups: Vec<&str> = token.split('-').collect();
    if groups.len() != 5 {
        return false;
    }
    let lengths: Vec<usize> = groups.iter().map(|group| group.len()).collect();
    lengths == [8, 4, 4, 4, 12]
        && groups
            .iter()
            .all(|group| !group.is_empty() && group.chars().all(|c| c.is_ascii_hexdigit()))
}

fn truncate_signature(signature: &str) -> String {
    if signature.len() <= MAX_SIGNATURE_BYTES {
        return signature.to_string();
    }
    let mut end = MAX_SIGNATURE_BYTES;
    while end > 0 && !signature.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = signature[..end].to_string();
    truncated.push('…');
    truncated
}
