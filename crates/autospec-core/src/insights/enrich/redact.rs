//! Secret redaction and the repository allowlist gate (issue #3848,
//! spec §37/§39).
//!
//! Every batch that reaches an [`super::Enricher`] must pass through
//! [`Redactor::redact`]; every session must pass [`repo_allowed`] before
//! it is dispatched. Both gates are deterministic (spec §4.1
//! deterministic-first): no LLM, no network, no clock reads.

/// Case-insensitive key names whose values are treated as secrets.
const SECRET_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "access_key",
    "auth_token",
    "token",
];

/// Deterministic secret redactor. Replaces recognized secret shapes with
/// `[REDACTED:<kind>]` markers and leaves everything else byte-identical.
pub struct Redactor;

impl Redactor {
    /// Redact all recognized secret shapes in `input`.
    ///
    /// Recognized shapes: AWS access key ids (`AKIA` + 16 `[0-9A-Z]`),
    /// GitHub tokens (`gh[pousr]_` + 36+ alphanumeric), PEM private-key
    /// headers, `Bearer <token>`, and `key=value` / `key: value` pairs
    /// where the key is one of [`SECRET_KEYS`].
    pub fn redact(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some((start, end, kind)) = earliest_match(rest) {
            out.push_str(&rest[..start]);
            out.push_str(&format!("[REDACTED:{kind}]"));
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }

    /// Redact every string leaf in a JSON value (payload evidence is
    /// stored as JSON). Non-string values pass through untouched.
    pub fn redact_json(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => serde_json::Value::String(Self::redact(s)),
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), Self::redact_json(v)))
                    .collect(),
            ),
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(Self::redact_json).collect())
            }
            other => other.clone(),
        }
    }
}

/// Find the earliest (start, end, kind) match of any secret shape in `s`.
fn earliest_match(s: &str) -> Option<(usize, usize, &'static str)> {
    let mut best: Option<(usize, usize, &'static str)> = None;
    for candidate in [
        (aws_access_key_match(s), "aws-access-key"),
        (github_token_match(s), "github-token"),
        (private_key_match(s), "private-key"),
        (bearer_match(s), "bearer-token"),
        (key_value_match(s), "secret-value"),
    ] {
        if let Some((start, end)) = candidate.0 {
            if best.map(|(b, _, _)| start < b).unwrap_or(true) {
                best = Some((start, end, candidate.1));
            }
        }
    }
    best
}

/// `AKIA` followed by at least 16 uppercase-alphanumeric characters.
fn aws_access_key_match(s: &str) -> Option<(usize, usize)> {
    for (i, _) in s.match_indices("AKIA") {
        let tail = &s[i + 4..];
        let len = tail
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            .count();
        if len >= 16 {
            return Some((i, i + 4 + 16));
        }
    }
    None
}

/// `gh` + one of `pousr` + `_` followed by at least 36 token characters.
fn github_token_match(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    for (i, _) in s.match_indices("gh") {
        if i + 4 <= bytes.len() && b"pousr".contains(&bytes[i + 2]) && bytes[i + 3] == b'_' {
            let mut j = i + 4;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'-') {
                j += 1;
            }
            if j - (i + 4) >= 36 {
                return Some((i, j));
            }
        }
    }
    None
}

/// A `-----BEGIN ... PRIVATE KEY-----` header (fail-closed: an unclosed
/// header redacts to end of string).
fn private_key_match(s: &str) -> Option<(usize, usize)> {
    let lower = s.to_ascii_lowercase();
    let i = lower.find("-----begin")?;
    let after = &s[i..];
    if let Some(rel) = after.find("PRIVATE KEY-----") {
        return Some((i, i + rel + "PRIVATE KEY-----".len()));
    }
    if let Some(rel) = after.find("-----") {
        return Some((i, i + rel + 5));
    }
    Some((i, s.len()))
}

/// `Bearer <token>` (case-insensitive, word-boundary) where the token runs
/// to the next whitespace.
fn bearer_match(s: &str) -> Option<(usize, usize)> {
    let lower = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find("bearer") {
        let i = search_from + rel;
        let preceded_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if preceded_ok {
            let mut j = i + 6;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b':' || bytes[j] == b'\t') {
                j += 1;
            }
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j > i + 6 {
                return Some((i, j));
            }
        }
        search_from = i + 1;
    }
    None
}

/// `<secret-key> = value` / `<secret-key>: value` where the key is a whole
/// word from [`SECRET_KEYS`]; the value runs to the next whitespace.
fn key_value_match(s: &str) -> Option<(usize, usize)> {
    let lower = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    for key in SECRET_KEYS {
        let mut search_from = 0usize;
        while let Some(rel) = lower[search_from..].find(key) {
            let i = search_from + rel;
            let before_ok =
                i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let after = i + key.len();
            let after_ok = after >= bytes.len()
                || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
            if before_ok && after_ok {
                let mut j = after;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'=' || bytes[j] == b':') {
                    let mut value_start = j + 1;
                    while value_start < bytes.len()
                        && (bytes[value_start] == b' ' || bytes[value_start] == b'\t')
                    {
                        value_start += 1;
                    }
                    let mut value_end = value_start;
                    while value_end < bytes.len() && !bytes[value_end].is_ascii_whitespace() {
                        value_end += 1;
                    }
                    if value_end > value_start {
                        // Redact the value only — the key name is evidence,
                        // not a secret.
                        return Some((value_start, value_end));
                    }
                }
            }
            search_from = i + 1;
        }
    }
    None
}

/// Repository allowlist gate (spec §39): `repo` is allowed when the
/// allowlist is non-empty and contains the exact repo or its owning
/// organization (an entry `acme` allows `acme/any-repo`).
pub fn repo_allowed(repo: Option<&str>, allowlist: &[String]) -> bool {
    let Some(repo) = repo.map(str::trim).filter(|r| !r.is_empty()) else {
        return false;
    };
    allowlist.iter().any(|entry| {
        let entry = entry.trim();
        !entry.is_empty() && (entry == repo || repo.starts_with(&format!("{entry}/")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWS: &str = "AKIAIOSFODNN7EXAMPLE"; // linter:allow-SECURITY AWS doc example key, non-secret fixture
    const GHT: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789"; // linter:allow-SECURITY fabricated shape, non-secret fixture

    #[test]
    fn redact_aws_access_key() {
        let out = Redactor::redact(&format!("key={AWS} end"));
        assert_eq!(out, "key=[REDACTED:aws-access-key] end");
    }

    #[test]
    fn redact_github_token() {
        let out = Redactor::redact(&format!("token: {GHT}."));
        assert_eq!(out, "token: [REDACTED:github-token].");
    }

    #[test]
    fn redact_bearer_token() {
        let out = Redactor::redact("Authorization: Bearer abc.def.ghi123 rest");
        assert_eq!(out, "Authorization: [REDACTED:bearer-token] rest");
    }

    #[test]
    fn redact_private_key_header() {
        let out = Redactor::redact("pem: -----BEGIN RSA PRIVATE KEY-----abc"); // linter:allow-SECURITY header string only, no key material
        assert!(!out.contains("PRIVATE KEY-----"), "{out}");
        assert!(out.contains("[REDACTED:private-key]"), "{out}");
    }

    #[test]
    fn redact_password_and_api_key_values() {
        let out = Redactor::redact("password=hunter2 api_key=sk-live-abc");
        assert_eq!(
            out,
            "password=[REDACTED:secret-value] api_key=[REDACTED:secret-value]"
        );
    }

    #[test]
    fn redact_word_boundary_not_prefix_collision() {
        // `secretary=` is not the `secret` key; `mytoken` is not `token`.
        let out = Redactor::redact("secretary=alice mytoken=plain");
        assert_eq!(out, "secretary=alice mytoken=plain");
    }

    #[test]
    fn redact_leaves_clean_text_untouched() {
        let clean = "session summary: 12 tool calls in repo inferweave/autospec";
        assert_eq!(Redactor::redact(clean), clean);
    }

    #[test]
    fn redact_json_walks_strings_only() {
        let value = serde_json::json!({
            "note": "token: hunter2",
            "count": 7,
            "items": ["AKIAIOSFODNN7EXAMPLE", null, true] // linter:allow-SECURITY AWS doc example key, non-secret fixture
        });
        let redacted = Redactor::redact_json(&value);
        assert_eq!(redacted["note"], "token: [REDACTED:secret-value]");
        assert_eq!(redacted["count"], 7);
        assert!(redacted["items"][0]
            .as_str()
            .unwrap()
            .starts_with("[REDACTED:"));
        assert!(redacted["items"][1].is_null());
        assert_eq!(redacted["items"][2], true);
    }

    #[test]
    fn repo_allowed_exact_match() {
        let allow = vec!["acme/web".to_string()];
        assert!(repo_allowed(Some("acme/web"), &allow));
        assert!(!repo_allowed(Some("acme/other"), &allow));
    }

    #[test]
    fn repo_allowed_org_prefix() {
        let allow = vec!["acme".to_string()];
        assert!(repo_allowed(Some("acme/web"), &allow));
        assert!(!repo_allowed(Some("acme-evil/web"), &allow));
    }

    #[test]
    fn repo_allowed_empty_or_missing_repo_is_denied() {
        let allow = vec!["acme".to_string()];
        assert!(!repo_allowed(None, &allow));
        assert!(!repo_allowed(Some(""), &allow));
        assert!(!repo_allowed(Some("acme/web"), &[]));
    }
}
