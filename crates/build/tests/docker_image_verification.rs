//! End-to-end checks that the build refuses untrusted Docker images.
//!
//! These tests talk to a real Docker daemon and pull a small public image, so they are
//! ignored by default. Run them with:
//!
//! ```text
//! cargo test -p fluentbase-build --test docker_image_verification -- --ignored
//! ```

use fluentbase_build::docker::ensure_rust_image;
use std::process::Command;

/// Small public image used as a stand-in for the build image.
const UPSTREAM: &str = "alpine";
const UPSTREAM_TAG: &str = "3.20";
/// Repository the poisoned image pretends to be.
const IMPERSONATED: &str = "ghcr.io/fluentlabs-xyz/fluentbase-build-verification-test";

fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("failed to run docker {args:?}: {err}"));
    assert!(
        output.status.success(),
        "docker {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn upstream_reference() -> String {
    format!("{UPSTREAM}:{UPSTREAM_TAG}")
}

/// Pull the upstream image and retag it as the build image, i.e. exactly what an attacker
/// with local Docker access does to get their own code into a build.
fn poison_local_tag(tag: &str) -> String {
    let upstream = upstream_reference();
    docker(&["pull", "--platform", "linux/amd64", &upstream]);
    docker(&["tag", &upstream, tag]);
    docker(&["image", "inspect", "--format", "{{.Id}}", tag])
}

#[test]
#[ignore = "requires a Docker daemon and network access"]
fn rejects_poisoned_local_tag() {
    let tag = format!("{IMPERSONATED}:v0.0.0-poisoned");
    poison_local_tag(&tag);

    let err = ensure_rust_image(&tag, None, false)
        .expect_err("a retagged local image must not be executed");
    let message = format!("{err:#}");
    assert!(
        message.contains("no registry digest"),
        "unexpected error: {message}"
    );

    // The escape hatch is the only way to run it, and it never applies to a pinned build.
    let unverified =
        ensure_rust_image(&tag, None, true).expect("verification can be relaxed explicitly");
    assert!(unverified.digest().is_none());

    let pinned_digest = format!("sha256:{}", "1".repeat(64));
    let err = ensure_rust_image(&tag, Some(&pinned_digest), true)
        .expect_err("--allow-unverified-docker-image must not relax a pinned digest");
    let message = format!("{err:#}");
    assert!(
        message.contains("digest mismatch") && message.contains(&pinned_digest),
        "unexpected error: {message}"
    );

    docker(&["rmi", &tag]);
}

#[test]
#[ignore = "requires a Docker daemon and network access"]
fn accepts_image_pulled_from_its_repository_and_pins_it() {
    let upstream = upstream_reference();
    docker(&["pull", "--platform", "linux/amd64", &upstream]);

    let verified = ensure_rust_image(&upstream, None, false).expect("genuine image is accepted");
    let digest = verified.digest().expect("digest is resolved").to_string();

    assert_eq!(verified.reference(), format!("{UPSTREAM}@{digest}"));
    assert_eq!(verified.requested(), upstream);

    // The same digest, passed explicitly, must be accepted.
    let pinned = ensure_rust_image(&upstream, Some(&digest), false).expect("pinned image is run");
    assert_eq!(pinned.reference(), verified.reference());
    assert_eq!(pinned.image_id(), verified.image_id());

    // Any other digest must fail before the container is started.
    let wrong = format!("sha256:{}", "9".repeat(64));
    let err = ensure_rust_image(&upstream, Some(&wrong), false)
        .expect_err("a digest that does not match the local image must fail");
    assert!(
        format!("{err:#}").contains("digest mismatch"),
        "unexpected error: {err:#}"
    );
}
