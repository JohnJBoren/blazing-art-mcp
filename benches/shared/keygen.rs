//! Shared synthetic-key generator + index builders for the bench binaries.
//!
//! Both `art_benchmarks.rs` and `memory_rss.rs` `#[path]`-include this file.
//! It is NOT a regular module under `src/`; the `_shared` subdirectory keeps
//! Cargo from auto-registering it as a `[[bench]]` target.

use std::collections::{BTreeMap, HashMap};
use std::ffi::CString;

use blart::TreeMap as ArtMap;

/// Synthetic value type. Fields exist to give the benchmark a realistic clone cost
/// (a single `usize` value would not faithfully model insert/lookup work).
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct AstSymbol {
    pub line: u32,
    pub col: u32,
    pub kind: &'static str,
    pub name: String,
}

/// Tiny xorshift PRNG — deterministic, no external dependency on `rand`.
pub struct XorShift(u64);
impl XorShift {
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

pub const KINDS: &[&str] = &["fn", "struct", "impl", "trait", "mod", "enum", "const", "type"];
pub const NAME_VOCAB: &[&str] = &[
    "parse", "build", "render", "load", "save", "encode", "decode", "validate",
    "handle", "process", "send", "receive", "open", "close", "read", "write",
    "init", "shutdown", "scan", "match", "find", "lookup", "insert", "remove",
    "create", "update", "delete", "query", "fetch", "publish", "subscribe", "execute",
];

/// Parameterized key generator. Produces `repos × files_per_repo × syms_per_file`
/// keys with realistic prefix sharing — the workload ART exploits.
///
/// Key shape: `<repo>\x01<path>\x01<line5>:<col3>:<kind>\x01<name>` (matches
/// the production Phase 4 schema; `\x01` is the segment separator because
/// `CString::new` rejects interior NULs and `blart::TreeMap::insert` requires
/// `NoPrefixesBytes`).
pub fn generate_keys_n(
    repos: usize,
    files_per_repo: usize,
    syms_per_file: usize,
) -> Vec<(String, AstSymbol)> {
    let total = repos * files_per_repo * syms_per_file;
    let mut rng = XorShift::new(0xCAFEBABEDEADBEEF);
    let mut out = Vec::with_capacity(total);
    for repo in 0..repos {
        for file in 0..files_per_repo {
            for sym in 0..syms_per_file {
                let kind = KINDS[(rng.next() as usize) % KINDS.len()];
                let name_root = NAME_VOCAB[(rng.next() as usize) % NAME_VOCAB.len()];
                let name = format!("{name_root}_{sym}");
                let line = (sym as u32 * 7 + 1) % 99999;
                let col = ((rng.next() as u32) % 80) + 1;
                let key = format!(
                    "repo_{repo:02}\x01src/mod_{file:03}/file_{file:03}.rs\x01{line:05}:{col:03}:{kind}\x01{name}"
                );
                out.push((key, AstSymbol { line, col, kind, name }));
            }
        }
    }
    out
}

#[allow(dead_code)]
pub fn build_art(keys: &[(String, AstSymbol)]) -> ArtMap<CString, AstSymbol> {
    let mut t = ArtMap::new();
    for (k, v) in keys {
        let ck = CString::new(k.as_bytes()).expect("no interior NULs in synthetic keys");
        t.insert(ck, v.clone());
    }
    t
}

#[allow(dead_code)]
pub fn build_btree(keys: &[(String, AstSymbol)]) -> BTreeMap<String, AstSymbol> {
    let mut t = BTreeMap::new();
    for (k, v) in keys {
        t.insert(k.clone(), v.clone());
    }
    t
}

#[allow(dead_code)]
pub fn build_hash(keys: &[(String, AstSymbol)]) -> HashMap<String, AstSymbol> {
    let mut t = HashMap::with_capacity(keys.len());
    for (k, v) in keys {
        t.insert(k.clone(), v.clone());
    }
    t
}
