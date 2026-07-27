//! v0.3 Task 5: persistence e2e.
//!
//! Snapshot the in-memory index after a real ingest, drop the original Memory,
//! reload from disk into a fresh Memory, and verify the index is identical.
//! Covers the full happy path: serialize → atomic-write → fresh-process-equivalent
//! load → query → match.

use std::path::Path;

use blazing_art_mcp::ingest;
use blazing_art_mcp::memory::Memory;

const REPO_ID: &str = "sample-repo";

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-repo")
}

#[test]
fn snapshot_then_load_preserves_full_index() {
    // 1. Ingest the fixture into a fresh Memory.
    let mem1 = Memory::new(1000);
    let stats = ingest::ingest_repo(&mem1, REPO_ID, &fixture_path());
    assert!(stats.errors.is_empty());
    let total_before = mem1.symbol_count();
    assert!(total_before > 0);

    // Capture a known query result *before* snapshotting so we can compare.
    let circle_def_before = mem1.find_symbols("sym\x01class\x01Circle\x01", 10);
    assert_eq!(circle_def_before.len(), 1);
    let sample_before = circle_def_before[0].clone();

    // Add a sample entity + event so all three maps are populated.
    use blazing_art_mcp::memory::{Entity, Event};
    mem1.add_entity(Entity {
        name: "test-entity".into(),
        summary: "for snapshot test".into(),
        born: None,
        tags: vec!["snapshot".into()],
    });
    mem1.add_event(Event {
        id: "test-event".into(),
        timestamp: "2026-05-27T00:00:00Z".into(),
        description: "for snapshot test".into(),
        category: "test".into(),
    });

    // 2. Snapshot to a temp file.
    let tmp = tempfile_for_test("blazing-art-snapshot.bin");
    mem1.snapshot(&tmp).expect("snapshot must succeed");
    assert!(tmp.exists(), "snapshot file must exist after snapshot()");
    let bytes = std::fs::metadata(&tmp).unwrap().len();
    assert!(bytes > 100, "snapshot must contain real payload (>100 bytes); got {bytes}");

    // Sanity: file starts with the BARTS001 magic.
    let head = std::fs::read(&tmp).unwrap();
    assert_eq!(&head[..8], b"BARTS001", "snapshot must start with magic header");

    // 3. Drop mem1, create a fresh empty Memory, load_snapshot.
    drop(mem1);
    let mem2 = Memory::new(1000);
    assert_eq!(mem2.symbol_count(), 0);
    mem2.load_snapshot(&tmp).expect("load_snapshot must succeed");

    // 4. Verify counts and a known query round-trip.
    assert_eq!(mem2.symbol_count(), total_before, "symbol count must match");
    assert_eq!(mem2.entity_count(), 1, "test-entity must round-trip");
    assert_eq!(mem2.event_count(), 1, "test-event must round-trip");

    let circle_def_after = mem2.find_symbols("sym\x01class\x01Circle\x01", 10);
    assert_eq!(circle_def_after.len(), 1);
    let sample_after = &circle_def_after[0];
    assert_eq!(sample_after.repo, sample_before.repo);
    assert_eq!(sample_after.path, sample_before.path);
    assert_eq!(sample_after.line, sample_before.line);
    assert_eq!(sample_after.name, sample_before.name);

    // Entity round-trip.
    let ent = mem2.lookup_entity("test-entity").expect("entity must round-trip");
    assert_eq!(ent.summary, "for snapshot test");

    // Cleanup.
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn load_snapshot_rejects_files_without_magic() {
    let tmp = tempfile_for_test("not-a-snapshot.bin");
    std::fs::write(&tmp, b"junk junk junk").unwrap();
    let mem = Memory::new(100);
    let err = mem.load_snapshot(&tmp).expect_err("must reject non-snapshot file");
    let msg = err.to_string();
    assert!(
        msg.contains("not a blazing-art snapshot"),
        "wrong error message: {msg}"
    );
    let _ = std::fs::remove_file(&tmp);
}

fn tempfile_for_test(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("blazing-art-test-{}-{name}", std::process::id()));
    p
}
