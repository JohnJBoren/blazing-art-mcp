//! v0.2 Task 7: schema validation for the hand-curated gold set.
//!
//! Asserts every record in `eval/goldset/handcurated.jsonl`:
//!   * Parses cleanly under the GoldRecord schema below.
//!   * Has a non-empty query and a non-empty role.
//!   * For non-negative records, references a path that exists under
//!     `tests/fixtures/{sample-repo, sample-multilang}/`.
//!   * No duplicate `(repo, path, name, query)` quadruple.
//!   * Each plan-mandated category appears at least once.
//!
//! The eval runner (Task 8) consumes the same schema, so this test is the
//! gate that prevents typos / schema drift from silently breaking eval CI.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct GoldRecord {
    query: String,
    repo: String,
    path: String,
    kind: String,
    name: String,
    role: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    must_be_empty: bool,
}

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_handcurated() -> Vec<GoldRecord> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/goldset/handcurated.jsonl");
    let text = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str::<GoldRecord>(line)
                .unwrap_or_else(|e| panic!("line {} parse error: {e}\n  line: {line}", i + 1))
        })
        .collect()
}

#[test]
fn handcurated_records_parse_and_have_required_fields() {
    let records = load_handcurated();
    assert!(records.len() >= 25, "expected ~30 records; got {}", records.len());
    for (i, r) in records.iter().enumerate() {
        assert!(!r.query.is_empty(), "record {i}: query must be non-empty");
        assert!(!r.role.is_empty(), "record {i}: role must be non-empty");
        assert!(
            r.role == "definition" || r.role == "reference",
            "record {i}: role must be 'definition' or 'reference'; got '{}'",
            r.role
        );
    }
}

#[test]
fn handcurated_non_negative_records_reference_real_fixture_files() {
    let fx = fixtures_dir();
    let records = load_handcurated();
    for (i, r) in records.iter().enumerate() {
        if r.must_be_empty {
            // Negative cases: path/kind/name may be empty placeholders.
            continue;
        }
        assert!(!r.path.is_empty(), "record {i}: non-negative record must have a path");
        assert!(!r.kind.is_empty(), "record {i}: non-negative record must have a kind");
        assert!(!r.name.is_empty(), "record {i}: non-negative record must have a name");

        // The repo_id maps to a fixture subdirectory by convention.
        let repo_dir = match r.repo.as_str() {
            "sample-repo" => fx.join("sample-repo"),
            "multilang" => fx.join("sample-multilang"),
            other => panic!("record {i}: unknown repo_id '{other}'"),
        };
        let abs = repo_dir.join(&r.path);
        assert!(
            abs.exists(),
            "record {i}: expected file '{}' does not exist (repo={}, path={})",
            abs.display(),
            r.repo,
            r.path
        );
    }
}

#[test]
fn handcurated_records_are_unique() {
    let records = load_handcurated();
    let mut seen: HashSet<(String, String, String, String)> = HashSet::new();
    for (i, r) in records.iter().enumerate() {
        let key = (r.repo.clone(), r.path.clone(), r.name.clone(), r.query.clone());
        assert!(
            seen.insert(key.clone()),
            "record {i}: duplicate (repo, path, name, query) = {key:?}"
        );
    }
}

#[test]
fn handcurated_covers_all_plan_categories() {
    // Plan calls for: ambiguous, cross_language, rename, long_tail, negative.
    let records = load_handcurated();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for r in &records {
        if let Some(cat) = r.category.as_deref() {
            *counts.entry(cat).or_default() += 1;
        }
    }
    for required in &["ambiguous", "cross_language", "rename", "long_tail", "negative"] {
        assert!(
            counts.get(required).copied().unwrap_or(0) >= 3,
            "category '{required}' must have at least 3 records; counts: {counts:?}"
        );
    }
}
