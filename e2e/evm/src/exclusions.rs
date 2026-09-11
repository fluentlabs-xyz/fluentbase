use revm_statetest_types::Test;
use serde::Deserialize;
use std::{path::Path, sync::OnceLock};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exclusion {
    fork: String,
    path: String,
    reason: String,
    source: String,
    cases: Option<Vec<String>>,
    exceptions: Option<Vec<String>>,
}

impl Exclusion {
    fn matches(&self, path: &Path, name: &str, test: &Test) -> bool {
        let suffix = format!("for_{}/{}", self.fork.to_lowercase(), self.path);
        path.ends_with(suffix)
            && self
                .cases
                .as_ref()
                .is_none_or(|cases| cases.iter().any(|case| case == name))
            && self.exceptions.as_ref().is_none_or(|exceptions| {
                test.expect_exception.as_deref().is_some_and(|expected| {
                    expected
                        .split('|')
                        .any(|part| exceptions.iter().any(|value| value == part))
                })
            })
    }
}

/// Apply only reviewed, version-controlled exclusions. Fully excluded fixtures
/// are also generated with #[ignore]; partial fixtures retain all other cases.
pub(crate) fn excluded_case_reason(path: &Path, name: &str, test: &Test) -> Option<&'static str> {
    static EXCLUSIONS: OnceLock<Vec<Exclusion>> = OnceLock::new();
    EXCLUSIONS
        .get_or_init(|| {
            let entries: Vec<Exclusion> =
                serde_json::from_str(include_str!("../excluded-tests.json")).unwrap();
            assert!(entries
                .iter()
                .all(|entry| !entry.reason.trim().is_empty() && !entry.source.trim().is_empty()));
            entries
        })
        .iter()
        .find(|entry| entry.matches(path, name, test))
        .map(|entry| entry.reason.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm::primitives::B256;
    use serde_json::json;

    #[test]
    fn partial_exclusions_preserve_other_cases_and_forks() {
        let entry: Exclusion = serde_json::from_value(json!({
            "fork": "Osaka", "path": "mixed.json", "reason": "protocol difference",
            "source": "specification", "cases": ["excluded"], "exceptions": ["FloorGas"]
        }))
        .unwrap();
        let mut post: Test = serde_json::from_value(json!({
            "hash": B256::ZERO, "logs": B256::ZERO,
            "indexes": {"data": 0, "gas": 0, "value": 0},
            "expectException": "IntrinsicGas|FloorGas"
        }))
        .unwrap();
        let path = Path::new("fixtures/for_osaka/mixed.json");
        assert!(entry.matches(path, "excluded", &post));
        assert!(!entry.matches(path, "supported", &post));
        assert!(!entry.matches(
            Path::new("fixtures/for_prague/mixed.json"),
            "excluded",
            &post
        ));
        assert!(!entry.matches(
            Path::new("fixtures/for_osaka/unrelated/mixed.json"),
            "excluded",
            &post
        ));
        post.expect_exception = Some("OtherError".into());
        assert!(!entry.matches(path, "excluded", &post));
        post.expect_exception = None;
        assert!(!entry.matches(path, "excluded", &post));
    }
}
