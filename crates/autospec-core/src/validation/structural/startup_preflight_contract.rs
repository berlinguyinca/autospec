pub(super) fn validate_reliability(canonical: &str) -> Result<(), String> {
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
