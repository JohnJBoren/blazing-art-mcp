//! ART-backed memory store.
//!
//! `Memory` holds two `blart::TreeMap<CString, T>` instances behind a `parking_lot::RwLock`.
//! Keys are `CString` because `blart::TreeMap::insert` requires `NoPrefixesBytes`,
//! which `String` does not satisfy (one string can be a byte-prefix of another).
//! The trailing NUL of `CString` makes any inserted key prefix-free by construction.
//!
//! Prefix scans use `TreeMap::prefix(&[u8])` — O(k + m) where k is prefix length
//! and m is result count.

use std::{ffi::CString, fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use blart::TreeMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::ingest::AstSymbol;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Entity {
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub born: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Event {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    pub category: String,
}

/// Build a `CString` key from a borrowed `&str`. Returns `None` if the input
/// contains an interior NUL byte (in which case the value is rejected at the
/// tool boundary rather than panicking).
fn key(s: &str) -> Option<CString> {
    CString::new(s).ok()
}

pub struct Memory {
    entities: Arc<RwLock<TreeMap<CString, Entity>>>,
    events: Arc<RwLock<TreeMap<CString, Event>>>,
    /// AST symbol index. Holds BOTH primary keys (`pri\x01...`) and inverted
    /// keys (`sym\x01...`) — the leading namespace is part of the key, so
    /// prefix scans naturally select one or the other.
    symbols: Arc<RwLock<TreeMap<CString, AstSymbol>>>,
    event_limit: usize,
}

impl Memory {
    pub fn new(event_limit: usize) -> Self {
        Self {
            entities: Arc::new(RwLock::new(TreeMap::new())),
            events: Arc::new(RwLock::new(TreeMap::new())),
            symbols: Arc::new(RwLock::new(TreeMap::new())),
            event_limit,
        }
    }

    pub fn lookup_entity(&self, name: &str) -> Option<Entity> {
        let k = key(name)?;
        self.entities.read().get(&k).cloned()
    }

    pub fn add_entity(&self, entity: Entity) -> bool {
        let Some(k) = key(&entity.name) else { return false };
        self.entities.write().insert(k, entity);
        true
    }

    pub fn find_events(&self, prefix: &str) -> Vec<Event> {
        self.events
            .read()
            .prefix(prefix.as_bytes())
            .take(self.event_limit)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn add_event(&self, event: Event) -> bool {
        let Some(k) = key(&event.id) else { return false };
        self.events.write().insert(k, event);
        true
    }

    pub fn entity_count(&self) -> usize {
        self.entities.read().len()
    }

    pub fn event_count(&self) -> usize {
        self.events.read().len()
    }

    pub fn symbol_count(&self) -> usize {
        self.symbols.read().len()
    }

    /// Insert a single AST symbol under the given pre-encoded key. Returns false
    /// if the key contains an interior NUL (the SOH-separated key schema cannot
    /// produce one, but defensive callers still get a graceful failure).
    pub fn add_symbol(&self, key: &str, sym: AstSymbol) -> bool {
        let Ok(k) = CString::new(key.as_bytes()) else {
            return false;
        };
        self.symbols.write().insert(k, sym);
        true
    }

    /// Find AST symbols whose key starts with the given prefix. The caller chooses
    /// the namespace by writing the appropriate prefix:
    /// - `"pri\x01<repo>\x01<path>\x01"` — every symbol in one file (sorted by line)
    /// - `"pri\x01<repo>\x01"`           — every symbol in one repo
    /// - `"sym\x01fn\x01parse_request\x01"` — every `parse_request` function across all repos
    pub fn find_symbols(&self, prefix: &str, limit: usize) -> Vec<AstSymbol> {
        self.symbols
            .read()
            .prefix(prefix.as_bytes())
            .take(limit)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// Delete every symbol whose primary or inverted key references the given repo_id.
    /// Returns the number of entries removed (counts both primary and inverted).
    pub fn delete_repo_symbols(&self, repo_id: &str) -> usize {
        // Scan the whole index once; collect keys to delete, then remove them.
        // A single full-scan is correct here because we don't have a back-pointer
        // index, and `repo_id` may appear at different segment positions in
        // primary vs inverted keys.
        let mut to_remove: Vec<CString> = Vec::new();
        {
            let guard = self.symbols.read();
            for (k, v) in guard.iter() {
                if v.repo == repo_id {
                    to_remove.push(k.clone());
                }
            }
        }
        let mut guard = self.symbols.write();
        let mut removed = 0;
        for k in to_remove {
            if guard.remove(&k).is_some() {
                removed += 1;
            }
        }
        removed
    }

    pub fn load_entities(&self, path: &PathBuf) -> Result<()> {
        let text = fs::read_to_string(path)?;
        let list: Vec<Entity> = serde_json::from_str(&text)?;

        let mut entities = self.entities.write();
        let mut skipped = 0usize;
        for e in list {
            match key(&e.name) {
                Some(k) => {
                    entities.insert(k, e);
                }
                None => skipped += 1,
            }
        }

        eprintln!(
            "Loaded {} entities ({skipped} skipped due to NUL in name)",
            entities.len()
        );
        Ok(())
    }

    pub fn load_events(&self, path: &PathBuf) -> Result<()> {
        let text = fs::read_to_string(path)?;
        let list: Vec<Event> = serde_json::from_str(&text)?;

        let mut events = self.events.write();
        let mut skipped = 0usize;
        for ev in list {
            match key(&ev.id) {
                Some(k) => {
                    events.insert(k, ev);
                }
                None => skipped += 1,
            }
        }

        eprintln!(
            "Loaded {} events ({skipped} skipped due to NUL in id)",
            events.len()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(name: &str) -> Entity {
        Entity {
            name: name.to_string(),
            summary: format!("summary for {name}"),
            born: None,
            tags: vec!["test".to_string()],
        }
    }

    fn evt(id: &str) -> Event {
        Event {
            id: id.to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            description: format!("description for {id}"),
            category: "test".to_string(),
        }
    }

    #[test]
    fn lookup_hit_returns_entity() {
        let mem = Memory::new(100);
        assert!(mem.add_entity(ent("Alan Turing")));
        let got = mem.lookup_entity("Alan Turing").expect("should be present");
        assert_eq!(got.name, "Alan Turing");
        assert_eq!(got.summary, "summary for Alan Turing");
    }

    #[test]
    fn lookup_miss_returns_none() {
        let mem = Memory::new(100);
        assert!(mem.add_entity(ent("Alan Turing")));
        assert!(mem.lookup_entity("Ada Lovelace").is_none());
    }

    #[test]
    fn prefix_scan_returns_only_matching_keys_in_order() {
        let mem = Memory::new(100);
        for id in ["2025-12-01:foo", "2025-01-15:bar", "2025-01-02:baz", "2024-06-01:old", "2026-03-08:new"] {
            assert!(mem.add_event(evt(id)));
        }
        let hits = mem.find_events("2025-01");
        let ids: Vec<_> = hits.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["2025-01-02:baz", "2025-01-15:bar"]);
    }

    #[test]
    fn prefix_scan_respects_event_limit() {
        let mem = Memory::new(3);
        for i in 0..10 {
            assert!(mem.add_event(evt(&format!("2025-{i:02}-01:item"))));
        }
        let hits = mem.find_events("2025-");
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn key_with_interior_nul_is_rejected() {
        let mem = Memory::new(100);
        let bad = Entity {
            name: "name\0with\0nul".to_string(),
            summary: "nope".to_string(),
            born: None,
            tags: vec![],
        };
        assert!(!mem.add_entity(bad));
        assert_eq!(mem.entity_count(), 0);
    }

    #[test]
    fn empty_prefix_returns_all_events_up_to_limit() {
        let mem = Memory::new(100);
        for id in ["a-evt", "b-evt", "c-evt"] {
            assert!(mem.add_event(evt(id)));
        }
        let hits = mem.find_events("");
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn event_count_tracks_inserts() {
        let mem = Memory::new(100);
        assert_eq!(mem.event_count(), 0);
        mem.add_event(evt("a"));
        mem.add_event(evt("b"));
        assert_eq!(mem.event_count(), 2);
    }
}
