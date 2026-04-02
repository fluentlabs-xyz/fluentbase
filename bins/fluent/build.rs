#![allow(missing_docs)]

use std::{env, process::Command};

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        env::var(key)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

fn build_timestamp() -> String {
    if let Some(epoch) = first_non_empty_env(&["SOURCE_DATE_EPOCH"]) {
        // Keep epoch if provided for reproducible builds.
        return epoch;
    }

    if let Some(ts) = first_non_empty_env(&["BUILD_TIMESTAMP", "CI_BUILD_TIMESTAMP"]) {
        return ts;
    }

    if let Some(ts) = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return ts;
    }

    "unknown".to_string()
}

fn enabled_features() -> String {
    let mut features = env::vars()
        .filter_map(|(key, _)| key.strip_prefix("CARGO_FEATURE_").map(str::to_owned))
        .map(|key| key.to_ascii_lowercase().replace('_', "-"))
        .collect::<Vec<_>>();

    features.sort();
    features.dedup();

    if features.is_empty() {
        "none".to_string()
    } else {
        features.join(",")
    }
}

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-changed=.git/refs/heads");

    for key in [
        "GIT_COMMIT",
        "GITHUB_SHA",
        "CI_COMMIT_SHA",
        "GIT_TAG",
        "CI_COMMIT_TAG",
        "GITHUB_REF_TYPE",
        "GITHUB_REF_NAME",
        "SOURCE_DATE_EPOCH",
        "BUILD_TIMESTAMP",
        "CI_BUILD_TIMESTAMP",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }

    let git_sha = first_non_empty_env(&["GIT_COMMIT", "GITHUB_SHA", "CI_COMMIT_SHA"])
        .or_else(|| git_output(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());

    let git_sha_short = if git_sha == "unknown" {
        "unknown".to_string()
    } else {
        git_sha.chars().take(8).collect()
    };

    let tag_from_ci = if first_non_empty_env(&["GITHUB_REF_TYPE"]).as_deref() == Some("tag") {
        first_non_empty_env(&["GITHUB_REF_NAME"])
    } else {
        None
    };

    let git_tag = first_non_empty_env(&["GIT_TAG", "CI_COMMIT_TAG"])
        .or(tag_from_ci)
        .or_else(|| git_output(&["describe", "--tags", "--exact-match"]))
        .unwrap_or_else(|| "untagged".to_string());

    let is_dirty = Command::new("git")
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .status()
        .map(|status| !status.success())
        .unwrap_or(false);

    let version_suffix = if is_dirty { "-dirty" } else { "" };
    let target_triple = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=FLUENT_GIT_SHA={git_sha}");
    println!("cargo:rustc-env=FLUENT_GIT_SHA_SHORT={git_sha_short}");
    println!("cargo:rustc-env=FLUENT_GIT_TAG={git_tag}");
    println!(
        "cargo:rustc-env=FLUENT_BUILD_TIMESTAMP={}",
        build_timestamp()
    );
    println!("cargo:rustc-env=FLUENT_CARGO_TARGET_TRIPLE={target_triple}");
    println!(
        "cargo:rustc-env=FLUENT_CARGO_FEATURES={}",
        enabled_features()
    );
    println!("cargo:rustc-env=FLUENT_BUILD_PROFILE={profile}");
    println!("cargo:rustc-env=FLUENT_VERSION_SUFFIX={version_suffix}");
}
