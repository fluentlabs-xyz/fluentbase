/// Parse Rust version from verbose output into unified format
pub(crate) fn parse_rustc_version<S: AsRef<str>>(output: S) -> String {
    let output_str = output.as_ref();

    // Extract version and commit hash
    let mut version = String::new();
    let mut commit = String::new();

    for line in output_str.lines() {
        if version.is_empty() && line.starts_with("rustc") {
            version = line.trim().to_string();
        } else if line.starts_with("commit-hash: ") {
            if let Some(hash) = line.strip_prefix("commit-hash: ") {
                commit = hash.chars().take(7).collect();
            }
        }
    }

    if !commit.is_empty() {
        format!("{version} ({commit})")
    } else {
        version
    }
}
