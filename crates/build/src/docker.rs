use crate::{utils::parse_rustc_version, CARGO_CACHE_VOLUME, DOCKER_PLATFORM};
use anyhow::{bail, Context, Result};
use std::{fmt, path::Path, process::Command};

/// Every digest we accept is a registry manifest digest, which is always sha256.
const DIGEST_PREFIX: &str = "sha256:";
const DIGEST_HEX_LEN: usize = 64;

/// A parsed `[registry/]repository[:tag][@sha256:...]` image reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Repository without tag or digest, e.g. `ghcr.io/fluentlabs-xyz/fluentbase-build`.
    pub repository: String,
    /// Mutable tag, if the reference carried one.
    pub tag: Option<String>,
    /// Immutable digest, if the reference was pinned.
    pub digest: Option<String>,
}

impl ImageRef {
    pub fn parse(reference: &str) -> Result<Self> {
        let reference = reference.trim();
        if reference.is_empty() {
            bail!("Docker image reference is empty");
        }

        let (name, digest) = match reference.split_once('@') {
            Some((name, digest)) => (name, Some(validate_digest(digest)?)),
            None => (reference, None),
        };

        // A colon belongs to the tag only when it comes after the last path separator,
        // otherwise it is a registry port (e.g. `localhost:5000/fluentbase-build`).
        let name_start = name.rfind('/').map(|index| index + 1).unwrap_or(0);
        let (repository, tag) = match name[name_start..].rfind(':') {
            Some(offset) => {
                let index = name_start + offset;
                (&name[..index], Some(name[index + 1..].to_string()))
            }
            None => (name, None),
        };

        if repository.is_empty() {
            bail!("Docker image reference '{reference}' has an empty repository");
        }
        if tag.as_deref().is_some_and(str::is_empty) {
            bail!("Docker image reference '{reference}' has an empty tag");
        }

        Ok(Self {
            repository: repository.to_string(),
            tag,
            digest,
        })
    }

    /// Reference used to fetch the image. Prefers the digest when the reference is pinned.
    fn pull_reference(&self) -> String {
        match (&self.digest, &self.tag) {
            (Some(digest), _) => format!("{}@{}", self.repository, digest),
            (None, Some(tag)) => format!("{}:{}", self.repository, tag),
            (None, None) => format!("{}:latest", self.repository),
        }
    }
}

/// A Docker image whose provenance was checked before it is allowed to execute.
///
/// The only way to obtain one is [`ensure_rust_image`], and every command that runs the
/// image takes this type, so no build path can execute a bare mutable tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedImage {
    /// Immutable reference handed to `docker run` (`repository@sha256:...`, or the local
    /// image ID when verification was explicitly relaxed).
    reference: String,
    /// Repository the image is required to come from.
    repository: String,
    /// Registry digest. `None` only when the caller opted out of verification.
    digest: Option<String>,
    /// Local content-addressed image ID, re-checked right before execution.
    image_id: String,
    /// Reference originally requested, kept for diagnostics and metadata.
    requested: String,
}

impl VerifiedImage {
    /// Immutable reference to pass to Docker.
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Verified registry digest, when the image carries one.
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }

    /// Local image ID recorded at verification time.
    pub fn image_id(&self) -> &str {
        &self.image_id
    }

    /// Reference the build asked for (tag form for unpinned builds).
    pub fn requested(&self) -> &str {
        &self.requested
    }
}

impl fmt::Display for VerifiedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reference)
    }
}

/// Run command in the Docker container
pub fn run_in_docker(
    image: &VerifiedImage,
    args: &[String],
    mount_dir: &Path,
    work_dir: &Path,
    env_vars: &[(String, String)],
    rust_toolchain: &Option<String>,
) -> Result<()> {
    // Re-check the image immediately before execution: verification and `docker run`
    // are separate daemon calls, and nothing else guarantees the image did not change
    // in between.
    verify_before_run(image)?;

    let mount_dir = mount_dir
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize mount dir: {}", mount_dir.display()))?;

    let work_dir = work_dir
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize work dir: {}", work_dir.display()))?;

    let relative_dir = work_dir.strip_prefix(&mount_dir).with_context(|| {
        format!(
            "Work dir {} is not within mount dir {}",
            work_dir.display(),
            mount_dir.display()
        )
    })?;

    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--rm",
        "--platform",
        DOCKER_PLATFORM,
        "-v",
        &format!("{}:/usr/local/cargo", CARGO_CACHE_VOLUME),
        "-v",
        &format!("{}:/workspace", mount_dir.display()),
        "-w",
        &format!("/workspace/{}", relative_dir.display()),
    ]);

    // Add environment variables
    for (key, value) in env_vars {
        cmd.args(["-e", &format!("{key}={value}")]);
    }

    // Set the rust toolchain ONLY if it's explicitly provided.
    // If it's None, the container's default toolchain will be used.
    if let Some(toolchain) = rust_toolchain {
        cmd.args(["-e", &format!("RUSTUP_TOOLCHAIN={toolchain}")]);
    }

    cmd.arg(image.reference());
    cmd.args(args);

    eprintln!("Docker command: {:?}", cmd);

    let status = cmd.status().context("Failed to execute Docker command")?;

    if !status.success() {
        bail!("Docker command failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Resolve the build image to an immutable digest and verify it before any container runs.
///
/// * `image` is `repository[:tag][@sha256:...]`.
/// * `expected_digest` pins the image: the resolved digest must match it or the build fails.
/// * `allow_unverified` permits an image with no registry digest for `repository` (a locally
///   built image). It never relaxes an explicit `expected_digest`.
pub fn ensure_rust_image(
    image: &str,
    expected_digest: Option<&str>,
    allow_unverified: bool,
) -> Result<VerifiedImage> {
    check_docker()?;
    verify_host_platform()?;

    let image_ref = ImageRef::parse(image)?;
    let expected = match (image_ref.digest.as_deref(), expected_digest) {
        (Some(from_reference), Some(explicit)) => {
            let explicit = parse_expected_digest(&image_ref.repository, explicit)?;
            if from_reference != explicit {
                bail!(
                    "Conflicting Docker image digests\n  \
                     from image reference: {from_reference}\n  \
                     from --docker-digest:  {explicit}"
                );
            }
            Some(explicit)
        }
        (Some(from_reference), None) => Some(from_reference.to_string()),
        (None, Some(explicit)) => Some(parse_expected_digest(&image_ref.repository, explicit)?),
        (None, None) => None,
    };

    let pull_reference = match &expected {
        Some(digest) => format!("{}@{}", image_ref.repository, digest),
        None => image_ref.pull_reference(),
    };

    let inspected = match inspect_image(&pull_reference)? {
        Some(inspected) => inspected,
        None => {
            println!("Pulling base image: {pull_reference} ...");
            match pull_image(&pull_reference) {
                Ok(()) => inspect_image(&pull_reference)?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Docker image {pull_reference} is missing after a successful pull"
                    )
                })?,
                Err(pull_error) => {
                    // A pinned digest that cannot be fetched usually means the local image
                    // under the same reference is a different one. Report that mismatch
                    // instead of the pull failure, which hides the actual problem.
                    if let (Some(expected), Some(local)) = (
                        expected.as_deref(),
                        inspect_image(&image_ref.pull_reference())?,
                    ) {
                        select_verified_digest(
                            &image_ref.repository,
                            &local.repo_digests,
                            Some(expected),
                        )?;
                    }
                    return Err(pull_error);
                }
            }
        }
    };

    let digest = match select_verified_digest(
        &image_ref.repository,
        &inspected.repo_digests,
        expected.as_deref(),
    ) {
        Ok(digest) => Some(digest),
        // Only an unpinned build may fall back to the local image: a digest the caller
        // asked for is a hard requirement.
        Err(err) if allow_unverified && expected.is_none() => {
            eprintln!("WARN: {err:#}");
            eprintln!(
                "WARN: running unverified image {pull_reference} because image verification \
                 was explicitly relaxed (--allow-unverified-docker-image)"
            );
            None
        }
        Err(err) => return Err(err),
    };

    // Run by digest (or by image ID) so the mutable tag cannot be repointed between now
    // and execution.
    let reference = match &digest {
        Some(digest) => format!("{}@{}", image_ref.repository, digest),
        None => inspected.id.clone(),
    };

    match (&digest, &image_ref.digest, &expected) {
        (Some(digest), None, None) => {
            println!("Using image: {reference} (resolved from {image})");
            eprintln!(
                "WARN: Docker tag '{image}' is mutable. Pin release and system builds with \
                 --docker-digest {digest}"
            );
        }
        _ => println!("Using image: {reference}"),
    }

    Ok(VerifiedImage {
        reference,
        repository: image_ref.repository,
        digest,
        image_id: inspected.id,
        requested: image.to_string(),
    })
}

/// PUBLIC UTILS
/// Get Rust toolchain version from Docker image
pub fn get_image_rustc_version(image: &VerifiedImage) -> Result<String> {
    verify_before_run(image)?;

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            image.reference(),
            "rustc",
            "--version",
            "--verbose",
        ])
        .output()
        .context("Failed to get Rust version from Docker image")?;

    if !output.status.success() {
        bail!("Failed to get Rust version from image: {image}");
    }

    Ok(parse_rustc_version(String::from_utf8_lossy(&output.stdout)))
}

/// Get platform information from Docker image
pub fn get_image_platform(image: &VerifiedImage) -> Result<String> {
    verify_before_run(image)?;

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            image.reference(),
            "sh",
            "-c",
            "echo $(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)",
        ])
        .output()
        .context("Failed to get platform info from Docker image")?;

    if !output.status.success() {
        bail!("Failed to get platform info from image: {image}");
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

// Helper functions

/// Result of `docker image inspect` for a single image.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectedImage {
    /// Content-addressed local image ID.
    id: String,
    /// `repository@sha256:...` entries recorded when the image was pulled.
    repo_digests: Vec<String>,
}

/// Pick the registry digest the build is allowed to run.
///
/// Rejects images that carry no digest for `repository`, which is what a locally poisoned
/// or retagged image looks like: it never came from the expected repository.
fn select_verified_digest(
    repository: &str,
    repo_digests: &[String],
    expected: Option<&str>,
) -> Result<String> {
    let mut candidates: Vec<&str> = repo_digests
        .iter()
        .filter_map(|entry| {
            let (entry_repository, digest) = entry.split_once('@')?;
            (entry_repository == repository).then_some(digest)
        })
        .collect();
    candidates.sort_unstable();
    candidates.dedup();

    let found = || {
        if candidates.is_empty() {
            let others = repo_digests.join(", ");
            if others.is_empty() {
                "none (the image was never pulled from a registry)".to_string()
            } else {
                format!("none for this repository (image carries: {others})")
            }
        } else {
            candidates.join(", ")
        }
    };

    match expected {
        Some(expected) => {
            if candidates.contains(&expected) {
                Ok(expected.to_string())
            } else {
                bail!(
                    "Refusing to run Docker image: digest mismatch for {repository}\n  \
                     expected: {expected}\n  \
                     resolved: {}\n\
                     The local image does not match the pinned digest. Remove it and let the \
                     build pull {repository}@{expected}.",
                    found()
                )
            }
        }
        None => match candidates.as_slice() {
            [] => bail!(
                "Refusing to run Docker image {repository}: no registry digest for this \
                 repository\n  resolved: {}\n\
                 The local image was built or retagged locally, so its provenance cannot be \
                 verified. Pull it from the registry, pin it with --docker-digest <sha256:...>, \
                 or pass --allow-unverified-docker-image to accept it.",
                found()
            ),
            [only] => Ok(only.to_string()),
            many => bail!(
                "Refusing to run Docker image {repository}: it resolves to several registry \
                 digests ({})\nPin the one you trust with --docker-digest <sha256:...>.",
                many.join(", ")
            ),
        },
    }
}

/// Re-inspect an already verified image and fail unless it is still the same content.
fn verify_before_run(image: &VerifiedImage) -> Result<()> {
    let inspected = inspect_image(image.reference())?.ok_or_else(|| {
        anyhow::anyhow!(
            "Docker image {} disappeared after verification",
            image.reference()
        )
    })?;

    if inspected.id != image.image_id {
        bail!(
            "Refusing to run Docker image {}: it changed after verification\n  \
             verified image ID: {}\n  current image ID:  {}",
            image.reference(),
            image.image_id,
            inspected.id
        );
    }

    if let Some(digest) = image.digest() {
        select_verified_digest(&image.repository, &inspected.repo_digests, Some(digest))?;
    }

    Ok(())
}

/// Accept a pinned digest either bare (`sha256:...`) or as a full repository reference
/// (`repository@sha256:...`), which is the form `docker image inspect` reports.
fn parse_expected_digest(repository: &str, value: &str) -> Result<String> {
    let value = value.trim();

    match value.rsplit_once('@') {
        Some((prefix, digest)) => {
            if !prefix.is_empty() && prefix != repository {
                bail!(
                    "Pinned digest '{value}' belongs to repository '{prefix}', \
                     but the build image is '{repository}'"
                );
            }
            validate_digest(digest)
        }
        None => validate_digest(value),
    }
}

fn validate_digest(digest: &str) -> Result<String> {
    let digest = digest.trim();
    let hex = digest.strip_prefix(DIGEST_PREFIX).ok_or_else(|| {
        anyhow::anyhow!("Invalid image digest '{digest}': expected '{DIGEST_PREFIX}<64 hex chars>'")
    })?;

    if hex.len() != DIGEST_HEX_LEN || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid image digest '{digest}': expected '{DIGEST_PREFIX}<64 hex chars>'");
    }

    Ok(format!("{DIGEST_PREFIX}{}", hex.to_ascii_lowercase()))
}

/// Parse the `{{.Id}}` + `{{.RepoDigests}}` inspect template output.
fn parse_inspect_output(stdout: &str) -> Result<InspectedImage> {
    let mut lines = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());

    let id = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("docker image inspect returned no image ID"))?
        .to_string();

    Ok(InspectedImage {
        id,
        repo_digests: lines.map(str::to_string).collect(),
    })
}

/// Inspect an image, returning `None` when it is not present locally.
fn inspect_image(reference: &str) -> Result<Option<InspectedImage>> {
    let output = Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{.Id}}\n{{range .RepoDigests}}{{.}}\n{{end}}",
            reference,
        ])
        .output()
        .context("Failed to inspect Docker image")?;

    if !output.status.success() {
        return Ok(None);
    }

    parse_inspect_output(&String::from_utf8_lossy(&output.stdout))
        .with_context(|| format!("Failed to inspect Docker image: {reference}"))
        .map(Some)
}

fn pull_image(reference: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["pull", "--platform", DOCKER_PLATFORM, reference])
        .status()
        .context("Failed to run docker pull")?;

    if !status.success() {
        bail!("Failed to get image: {reference}");
    }

    Ok(())
}

fn check_docker() -> Result<()> {
    let output = Command::new("docker").args(["version"]).output();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => bail!("Docker command failed. Is Docker daemon running?"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Docker not found in PATH.\n\
                \n\
                Please install Docker from https://docker.com or use --no-docker for local builds."
            )
        }
        Err(e) => Err(e).context("Failed to check Docker installation"),
    }
}

fn verify_host_platform() -> Result<()> {
    // Windows requires WSL2 for linux/amd64 platform builds
    #[cfg(target_os = "windows")]
    {
        let in_wsl = std::env::var("WSL_DISTRO_NAME").is_ok()
            || std::path::Path::new("/proc/version").exists();

        if !in_wsl {
            bail!(
                "Docker builds on Windows require WSL2.\n\
                \n\
                Fluentbase builds target linux/amd64 platform for reproducibility.\n\
                Please run this command inside WSL2 or use --no-docker for local builds.\n\
                \n\
                Note: Local builds may not be reproducible across different platforms."
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPOSITORY: &str = "ghcr.io/fluentlabs-xyz/fluentbase-build";
    const TRUSTED: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    fn repo_digest(repository: &str, digest: &str) -> String {
        format!("{repository}@{digest}")
    }

    #[test]
    fn parses_tagged_reference() {
        let parsed = ImageRef::parse(&format!("{REPOSITORY}:v0.1.0")).unwrap();
        assert_eq!(parsed.repository, REPOSITORY);
        assert_eq!(parsed.tag.as_deref(), Some("v0.1.0"));
        assert_eq!(parsed.digest, None);
        assert_eq!(parsed.pull_reference(), format!("{REPOSITORY}:v0.1.0"));
    }

    #[test]
    fn parses_pinned_reference() {
        let parsed = ImageRef::parse(&format!("{REPOSITORY}:v0.1.0@{TRUSTED}")).unwrap();
        assert_eq!(parsed.repository, REPOSITORY);
        assert_eq!(parsed.tag.as_deref(), Some("v0.1.0"));
        assert_eq!(parsed.digest.as_deref(), Some(TRUSTED));
        assert_eq!(
            parsed.pull_reference(),
            format!("{REPOSITORY}@{TRUSTED}"),
            "a pinned reference must be fetched by digest"
        );
    }

    #[test]
    fn registry_port_is_not_a_tag() {
        let parsed = ImageRef::parse("localhost:5000/fluentbase-build").unwrap();
        assert_eq!(parsed.repository, "localhost:5000/fluentbase-build");
        assert_eq!(parsed.tag, None);

        let parsed = ImageRef::parse("localhost:5000/fluentbase-build:v1").unwrap();
        assert_eq!(parsed.repository, "localhost:5000/fluentbase-build");
        assert_eq!(parsed.tag.as_deref(), Some("v1"));
    }

    #[test]
    fn rejects_malformed_references_and_digests() {
        assert!(ImageRef::parse("  ").is_err());
        assert!(ImageRef::parse(&format!("{REPOSITORY}:")).is_err());
        assert!(ImageRef::parse(&format!("{REPOSITORY}@sha256:beef")).is_err());
        assert!(ImageRef::parse(&format!("{REPOSITORY}@md5:{}", "0".repeat(64))).is_err());
        assert!(validate_digest(&format!("sha256:{}", "z".repeat(64))).is_err());
        assert_eq!(
            validate_digest(&format!("sha256:{}", "A".repeat(64))).unwrap(),
            format!("sha256:{}", "a".repeat(64)),
            "digests compare case-insensitively"
        );
    }

    #[test]
    fn accepts_pinned_digest_in_bare_and_repository_form() {
        assert_eq!(parse_expected_digest(REPOSITORY, TRUSTED).unwrap(), TRUSTED);
        assert_eq!(
            parse_expected_digest(REPOSITORY, &repo_digest(REPOSITORY, TRUSTED)).unwrap(),
            TRUSTED
        );

        // A digest that belongs to another repository is a configuration mistake.
        let err =
            parse_expected_digest(REPOSITORY, &repo_digest("other.example.com/image", TRUSTED))
                .unwrap_err();
        assert!(
            err.to_string().contains("belongs to repository"),
            "unexpected error: {err}"
        );

        // A local image ID is not a registry digest, but it is shaped like one; it is
        // rejected later, when it fails to match any repository digest.
        assert!(parse_expected_digest(REPOSITORY, "not-a-digest").is_err());
    }

    #[test]
    fn accepts_image_pulled_from_the_expected_repository() {
        let digests = vec![repo_digest(REPOSITORY, TRUSTED)];
        assert_eq!(
            select_verified_digest(REPOSITORY, &digests, None).unwrap(),
            TRUSTED
        );
        assert_eq!(
            select_verified_digest(REPOSITORY, &digests, Some(TRUSTED)).unwrap(),
            TRUSTED
        );
    }

    #[test]
    fn rejects_locally_built_image_wearing_the_expected_tag() {
        // A locally built image has no registry digest at all.
        let err = select_verified_digest(REPOSITORY, &[], None).unwrap_err();
        assert!(
            err.to_string().contains("no registry digest"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_foreign_image_retagged_as_the_build_image() {
        // `docker tag evil/image ghcr.io/fluentlabs-xyz/fluentbase-build:v0.1.0` keeps the
        // digest of the repository the image really came from.
        let digests = vec![repo_digest("evil.example.com/image", TRUSTED)];

        let err = select_verified_digest(REPOSITORY, &digests, None).unwrap_err();
        assert!(
            err.to_string().contains("no registry digest"),
            "unexpected error: {err}"
        );

        let err = select_verified_digest(REPOSITORY, &digests, Some(TRUSTED)).unwrap_err();
        assert!(
            err.to_string().contains("digest mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_pinned_digest_mismatch() {
        let digests = vec![repo_digest(REPOSITORY, OTHER)];
        let err = select_verified_digest(REPOSITORY, &digests, Some(TRUSTED)).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("digest mismatch"), "{message}");
        assert!(message.contains(TRUSTED), "{message}");
        assert!(message.contains(OTHER), "{message}");
    }

    #[test]
    fn rejects_ambiguous_digests_unless_pinned() {
        let digests = vec![
            repo_digest(REPOSITORY, TRUSTED),
            repo_digest(REPOSITORY, OTHER),
        ];
        assert!(select_verified_digest(REPOSITORY, &digests, None).is_err());
        assert_eq!(
            select_verified_digest(REPOSITORY, &digests, Some(OTHER)).unwrap(),
            OTHER
        );
    }

    #[test]
    fn parses_inspect_output() {
        let stdout = format!(
            "sha256:{id}\n{}\n{}\n\n",
            repo_digest(REPOSITORY, TRUSTED),
            repo_digest("mirror.example.com/fluentbase-build", OTHER),
            id = "a".repeat(64),
        );

        let inspected = parse_inspect_output(&stdout).unwrap();
        assert_eq!(inspected.id, format!("sha256:{}", "a".repeat(64)));
        assert_eq!(
            inspected.repo_digests,
            vec![
                repo_digest(REPOSITORY, TRUSTED),
                repo_digest("mirror.example.com/fluentbase-build", OTHER),
            ]
        );

        assert!(parse_inspect_output("   \n").is_err());
    }

    #[test]
    fn image_without_repo_digests_parses_but_is_rejected() {
        let inspected = parse_inspect_output(&format!("sha256:{}\n", "b".repeat(64))).unwrap();
        assert!(inspected.repo_digests.is_empty());
        assert!(select_verified_digest(REPOSITORY, &inspected.repo_digests, None).is_err());
    }
}
