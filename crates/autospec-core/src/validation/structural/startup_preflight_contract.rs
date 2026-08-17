use std::path::Path;

/// Assert the injected `## Startup self-update` block is harness-safe, then
/// return whichever text actually carries the executable preflight body.
///
/// The block is expanded into every installed skill file (Claude `SKILL.md`,
/// Codex `prompts/<skill>.md`, OpenCode `agent/<skill>.md`), and a harness
/// substitutes positional parameters inside a *rendered* skill body at load
/// time (issue #3177). An assignment such as `target="$1"` therefore became the
/// caller's slash-command argument, which the wrapper-healing loop then wrote a
/// script over — before the daily throttle ever ran. No injected block may
/// carry one; the shell belongs in `scripts/autospec-startup-self-update.sh`,
/// which no harness renders.
///
/// Trees that predate the extraction (and the validator's own fixtures) keep the
/// legacy inline body as the contract source.
pub(super) fn resolve_contract_source(root: &Path, block: &str) -> Result<String, String> {
    for forbidden in ["=\"$1\"", "=\"$2\""] {
        if block.contains(forbidden) {
            return Err(format!(
                "startup preflight block assigns a positional parameter ({forbidden}); harness argument substitution rewrites it — move the shell into scripts/autospec-startup-self-update.sh"
            ));
        }
    }

    let script = root.join("scripts/autospec-startup-self-update.sh");
    if !script.is_file() {
        return Ok(block.to_string());
    }
    if !block.contains("autospec-startup-self-update.sh") {
        return Err(
            "templates/skill-blocks/startup-self-update.md must invoke scripts/autospec-startup-self-update.sh rather than inline its body"
                .to_string(),
        );
    }
    if !block.contains("AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts") {
        return Err(
            "startup preflight block must keep the installed-scripts resolution fallback"
                .to_string(),
        );
    }
    super::read(&script)
}

pub(super) fn validate_reliability(canonical: &str) -> Result<(), String> {
    if !canonical.contains("raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh") {
        return Err("startup preflight must call the curl-safe suite bootstrap.sh".to_string());
    }
    if !canonical.contains("--skill all --harness all --update") {
        return Err("startup preflight must update all skills across all harnesses".to_string());
    }
    if canonical.contains("raw.githubusercontent.com/berlinguyinca/autospec/main/install.sh")
        || canonical.contains("raw.githubusercontent.com/berlinguyinca/autospec/main/skills/")
    {
        return Err("startup preflight must not call a raw installer directly".to_string());
    }
    for required in [
        "last-update-failure.json",
        "self-update.log",
        "remote-version",
        "tail -c 65536",
        "installer_exit_code",
        "state publication failed ($INSTALLED)",
        "umask 077",
        "INSTALLED_BACKUP",
    ] {
        if !canonical.contains(required) {
            return Err(format!(
                "startup preflight reliability contract missing {required}"
            ));
        }
    }
    let remote_match = canonical
        .find("if [ \"$REMOTE\" = \"$LOCAL\" ]")
        .ok_or_else(|| "startup preflight must detect an up-to-date install".to_string())?;
    let first_success_write = canonical
        .find("> \"$LAST.tmp\"")
        .ok_or_else(|| "startup preflight must persist a successful update check".to_string())?;
    if first_success_write < remote_match {
        return Err("startup preflight must not advance last-update-check before success".into());
    }
    Ok(())
}
