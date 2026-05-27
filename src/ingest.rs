//! Tree-sitter AST ingestion. Walks a repo, parses each supported file, runs
//! the language-specific tags query, and writes one primary key + one inverted
//! key per `@definition.*` capture into the shared `Memory` symbol index.
//!
//! ## Schema (locked)
//!
//! ```text
//! primary  = "pri\x01<repo>\x01<rel_path>\x01<line5>:<col3>:<kind>\x01<name>"
//! inverted = "sym\x01<kind>\x01<name>\x01<repo>\x01<rel_path>:<line>"
//! ```
//!
//! Segment separator is `\x01` (SOH), NOT `\x00`, because `CString::new` rejects
//! interior NULs while still adding the trailing NUL that `blart::TreeMap`
//! requires for `NoPrefixesBytes`. Task 2 adds a third `ref\x01...` namespace.
//!
//! ## Why tags.scm and not a hand-coded `is_declaration` table?
//!
//! Tree-sitter ships an official [Code Navigation] subsystem with a
//! standardized capture vocabulary (`@definition.{class,function,interface,
//! method,module,macro}`, `@reference.{call,class,implementation,type}`,
//! plus inner `@name`). It is the same vocabulary GitHub uses for
//! search-based code navigation. By driving extraction from each grammar's
//! upstream `queries/tags.scm` we get one query loop instead of N hand-coded
//! tables, and we inherit grammar-author-blessed coverage for free. The
//! vendored queries live under `queries/<lang>/tags.scm` with their source
//! commit SHAs noted in headers.
//!
//! [Code Navigation]: https://tree-sitter.github.io/tree-sitter/4-code-navigation.html
//!
//! Side effect of the move: the `kind` field stored on each `AstSymbol` now
//! uses tags-vocabulary names (`function`, `class`, `method`, ...) instead of
//! raw tree-sitter node kinds (`function_item`, `struct_item`). See CLAUDE.md.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};
use walkdir::WalkDir;

use crate::memory::Memory;

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum SymbolRole {
    #[default]
    Definition,
    Reference,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AstSymbol {
    pub repo: String,
    pub path: String,
    pub line: u32,
    pub col: u32,
    /// Tags-vocabulary kind: one of `function`, `method`, `class`, `interface`,
    /// `module`, `macro`, `constant` (and whatever else the vendored tags
    /// queries emit). Capture names of the form `definition.X` or `reference.X`
    /// map to `kind=X`.
    pub kind: String,
    pub name: String,
    /// Whether this symbol entry was emitted by a `@definition.*` capture or a
    /// `@reference.*` capture. Defaults to `Definition` so JSON written before
    /// v0.2 Task 2 still deserializes cleanly.
    #[serde(default)]
    pub role: SymbolRole,
}

#[derive(Default, Serialize, Debug)]
pub struct IngestStats {
    pub files_parsed: usize,
    pub files_skipped: usize,
    pub symbols_indexed: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Lang {
    Rust,
    Python,
    Ts,
    Tsx,
    Go,
    Java,
    C,
    Cpp,
    Js,
}

impl Lang {
    fn for_extension(ext: &str) -> Option<Lang> {
        match ext {
            "rs" => Some(Lang::Rust),
            "py" => Some(Lang::Python),
            "ts" | "mts" | "cts" => Some(Lang::Ts),
            "tsx" => Some(Lang::Tsx),
            "go" => Some(Lang::Go),
            "java" => Some(Lang::Java),
            "c" | "h" => Some(Lang::C),
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(Lang::Cpp),
            // tree-sitter-javascript handles JSX, so .jsx is mapped here. If a
            // future workload needs TSX-style JSX semantics, change to Tsx.
            "js" | "mjs" | "cjs" | "jsx" => Some(Lang::Js),
            _ => None,
        }
    }

    fn ts_language(self) -> Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::C => tree_sitter_c::LANGUAGE.into(),
            Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Lang::Js => tree_sitter_javascript::LANGUAGE.into(),
        }
    }

    fn tags_query_source(self) -> &'static str {
        match self {
            Lang::Rust => include_str!("../queries/rust/tags.scm"),
            Lang::Python => include_str!("../queries/python/tags.scm"),
            Lang::Ts => include_str!("../queries/typescript/tags.scm"),
            Lang::Tsx => include_str!("../queries/tsx/tags.scm"),
            Lang::Go => include_str!("../queries/go/tags.scm"),
            Lang::Java => include_str!("../queries/java/tags.scm"),
            Lang::C => include_str!("../queries/c/tags.scm"),
            Lang::Cpp => include_str!("../queries/cpp/tags.scm"),
            Lang::Js => include_str!("../queries/javascript/tags.scm"),
        }
    }
}

/// Compiled-query registry. One `Arc<Query>` per language, built lazily on first
/// use. `Query` is `Send + Sync` per the upstream type bounds, so sharing across
/// rayon workers (Task 4) is safe.
fn query_for(lang: Lang) -> Arc<Query> {
    static REGISTRY: OnceLock<RwLock<HashMap<Lang, Arc<Query>>>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| RwLock::new(HashMap::new()));

    {
        let r = registry.read().expect("query registry poisoned");
        if let Some(q) = r.get(&lang) {
            return Arc::clone(q);
        }
    }
    // Compile outside the read lock; double-check on the write side.
    let compiled = Query::new(&lang.ts_language(), lang.tags_query_source())
        .unwrap_or_else(|e| panic!("vendored tags.scm for {lang:?} failed to compile: {e}"));
    let arc = Arc::new(compiled);
    let mut w = registry.write().expect("query registry poisoned");
    let entry = w.entry(lang).or_insert_with(|| Arc::clone(&arc));
    Arc::clone(entry)
}

/// Per-thread, per-language parser pool. Tree-sitter `Parser` is `!Sync`, so
/// each rayon worker keeps its own. Lazy: parsers are created on first use per
/// thread per language.
fn with_parser<R>(lang: Lang, f: impl FnOnce(&mut Parser) -> R) -> R {
    thread_local! {
        static POOL: RefCell<HashMap<Lang, Parser>> = RefCell::new(HashMap::new());
    }
    POOL.with(|cell| {
        let mut map = cell.borrow_mut();
        let parser = map.entry(lang).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&lang.ts_language())
                .expect("set_language must succeed for vendored grammar");
            p
        });
        f(parser)
    })
}

/// SOH guard. Schema requires that no segment value contains 0x01.
fn primary_key(repo: &str, path: &str, line: u32, col: u32, kind: &str, name: &str) -> Option<String> {
    if [repo, path, kind, name].iter().any(|s| s.as_bytes().contains(&0x01)) {
        return None;
    }
    Some(format!("pri\x01{repo}\x01{path}\x01{line:05}:{col:03}:{kind}\x01{name}"))
}

fn inverted_key(repo: &str, path: &str, line: u32, kind: &str, name: &str) -> Option<String> {
    if [repo, path, kind, name].iter().any(|s| s.as_bytes().contains(&0x01)) {
        return None;
    }
    Some(format!("sym\x01{kind}\x01{name}\x01{repo}\x01{path}:{line}"))
}

/// Encode a reference (call-site / use) key.
///
/// ```text
/// ref\x01<name>\x01<repo>\x01<rel_path>:<line>
/// ```
///
/// Unlike `inverted_key`, `kind` is NOT part of the prefix path — agents
/// usually want "every use of name X" regardless of whether the use is a
/// `call`, `class` ref, `implementation`, etc. The `kind` is still recorded
/// inside the `AstSymbol` body for server-side filtering.
fn reference_key(repo: &str, path: &str, line: u32, name: &str) -> Option<String> {
    if [repo, path, name].iter().any(|s| s.as_bytes().contains(&0x01)) {
        return None;
    }
    Some(format!("ref\x01{name}\x01{repo}\x01{path}:{line}"))
}

/// Returns `Some(kind)` if `capture_name` is `definition.<kind>`.
fn definition_kind(capture_name: &str) -> Option<&str> {
    capture_name.strip_prefix("definition.")
}

/// Returns `Some(kind)` if `capture_name` is `reference.<kind>` — `call`,
/// `class`, `implementation`, `type`, etc.
fn reference_kind(capture_name: &str) -> Option<&str> {
    capture_name.strip_prefix("reference.")
}

/// Run the language's tags query over a parsed tree and emit one
/// `(primary_key, AstSymbol)` + `(inverted_key, AstSymbol)` pair per matched
/// definition, plus one `(reference_key, AstSymbol)` per matched reference.
///
/// ### Multi-pattern dedup
///
/// Some grammars (notably Rust) have a more-specific pattern for methods
/// (`(declaration_list (function_item ...) @definition.method)`) followed by
/// a more-general pattern for top-level functions (`(function_item ...)
/// @definition.function`). A method node will match BOTH. We dedupe by the
/// node's byte span tagged with role, keeping only the first match — and
/// because the upstream Rust `tags.scm` lists method before function, the
/// first-wins rule gives the desired classification (method, not function).
///
/// Definition spans and reference spans dedupe independently because the
/// same node could be both (rare in practice but possible across grammars).
fn collect_symbols_via_query(
    tree: &tree_sitter::Tree,
    source: &[u8],
    lang: Lang,
    repo: &str,
    path: &str,
    out: &mut Vec<(String, AstSymbol)>,
) {
    let query = query_for(lang);
    let mut cursor = QueryCursor::new();
    let capture_names = query.capture_names();
    let mut seen: HashSet<(SymbolRole, usize, usize)> = HashSet::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        let mut role: Option<SymbolRole> = None;
        let mut role_kind: Option<&str> = None;
        let mut role_node: Option<Node> = None;
        let mut name_text: Option<&str> = None;

        for cap in m.captures {
            let cname = capture_names[cap.index as usize];
            if let Some(kind) = definition_kind(cname) {
                role = Some(SymbolRole::Definition);
                role_kind = Some(kind);
                role_node = Some(cap.node);
            } else if let Some(kind) = reference_kind(cname) {
                role = Some(SymbolRole::Reference);
                role_kind = Some(kind);
                role_node = Some(cap.node);
            } else if cname == "name" {
                if let Ok(t) = cap.node.utf8_text(source) {
                    name_text = Some(t);
                }
            }
        }

        let (Some(role), Some(kind), Some(role_node), Some(name)) =
            (role, role_kind, role_node, name_text)
        else {
            continue;
        };

        let span = (role, role_node.start_byte(), role_node.end_byte());
        if !seen.insert(span) {
            continue;
        }

        let pos = role_node.start_position();
        let line = pos.row as u32 + 1;
        let col = pos.column as u32 + 1;

        let sym = AstSymbol {
            repo: repo.to_string(),
            path: path.to_string(),
            line,
            col,
            kind: kind.to_string(),
            name: name.to_string(),
            role,
        };

        match role {
            SymbolRole::Definition => {
                if let Some(pk) = primary_key(repo, path, line, col, kind, name) {
                    out.push((pk, sym.clone()));
                }
                if let Some(ik) = inverted_key(repo, path, line, kind, name) {
                    out.push((ik, sym));
                }
            }
            SymbolRole::Reference => {
                if let Some(rk) = reference_key(repo, path, line, name) {
                    out.push((rk, sym));
                }
            }
        }
    }
}

/// Parse a single file, return its `(key, symbol)` pairs.
/// `Ok(empty)` if the extension is unsupported (callers treat this as "skip silently").
fn ingest_file(path: &Path, repo: &str, repo_root: &Path) -> Result<Vec<(String, AstSymbol)>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let Some(lang) = Lang::for_extension(ext) else {
        return Ok(Vec::new());
    };

    let source = std::fs::read(path)?;
    let rel = path
        .strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    // The thread-local pool means each rayon worker re-uses one Parser per
    // language; we don't pay the set_language cost per file.
    let tree_opt = with_parser(lang, |parser| parser.parse(&source, None));
    let Some(tree) = tree_opt else {
        anyhow::bail!("parse returned None for {}", path.display());
    };

    let mut out = Vec::new();
    collect_symbols_via_query(&tree, &source, lang, repo, &rel, &mut out);
    Ok(out)
}

/// Public entry: walk `repo_path` recursively, parse every supported file in
/// parallel via rayon, and insert the resulting symbols into `memory` under a
/// single bulk-write lock.
pub fn ingest_repo(memory: &Memory, repo_id: &str, repo_path: &Path) -> IngestStats {
    let mut stats = IngestStats::default();

    if !repo_path.exists() {
        stats.errors.push(format!("repo path does not exist: {}", repo_path.display()));
        return stats;
    }

    if repo_id.as_bytes().contains(&0x01) {
        stats.errors.push("repo_id contains SOH separator byte (\\x01)".to_string());
        return stats;
    }

    // Step 1: walk the tree (cheap) and collect every supported file path.
    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | ".venv" | "__pycache__" | "dist" | "build"
            )
        });

    let paths: Vec<PathBuf> = walker
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .and_then(Lang::for_extension)
                .is_some()
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    // Step 2: parse in parallel. Each worker uses its thread-local parser pool;
    // shared `Arc<Query>` per language is fetched lazily via `query_for`.
    // We collect (file_result, pairs) tuples so we can compute stats accurately.
    type FileResult = (PathBuf, Result<Vec<(String, AstSymbol)>>);
    let results: Vec<FileResult> = paths
        .into_par_iter()
        .map(|p| {
            let r = ingest_file(&p, repo_id, repo_path);
            (p, r)
        })
        .collect();

    // Step 3: drain results into a single batch and bulk-insert under one
    // write lock. This is the dominant correctness-plus-perf decision: every
    // alternative we tried (per-file lock, sharded TreeMap) was slower or
    // more complex.
    let mut batch: Vec<(String, AstSymbol)> = Vec::new();
    for (p, r) in results {
        match r {
            Ok(pairs) => {
                if pairs.is_empty() {
                    continue;
                }
                stats.files_parsed += 1;
                batch.extend(pairs);
            }
            Err(e) => {
                stats.files_skipped += 1;
                stats.errors.push(format!("{}: {e}", p.display()));
            }
        }
    }

    let inserted = memory.add_symbols_bulk(batch);
    stats.symbols_indexed = inserted;

    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_kind_strips_prefix() {
        assert_eq!(definition_kind("definition.function"), Some("function"));
        assert_eq!(definition_kind("definition.class"), Some("class"));
        assert_eq!(definition_kind("definition.method"), Some("method"));
        assert_eq!(definition_kind("name"), None);
        assert_eq!(definition_kind("reference.call"), None);
    }

    #[test]
    fn reference_kind_strips_prefix() {
        assert_eq!(reference_kind("reference.call"), Some("call"));
        assert_eq!(reference_kind("reference.class"), Some("class"));
        assert_eq!(reference_kind("reference.implementation"), Some("implementation"));
        assert_eq!(reference_kind("reference.type"), Some("type"));
        assert_eq!(reference_kind("name"), None);
        assert_eq!(reference_kind("definition.function"), None);
    }

    #[test]
    fn primary_key_format_is_stable() {
        let k = primary_key("r", "src/a.rs", 12, 4, "function", "foo").unwrap();
        assert_eq!(k, "pri\x01r\x01src/a.rs\x0100012:004:function\x01foo");
    }

    #[test]
    fn reference_key_format_is_stable() {
        let k = reference_key("r", "src/a.rs", 42, "foo").unwrap();
        assert_eq!(k, "ref\x01foo\x01r\x01src/a.rs:42");
    }

    #[test]
    fn reference_key_rejects_soh_in_segments() {
        assert!(reference_key("re\x01po", "p", 1, "n").is_none());
        assert!(reference_key("repo", "pa\x01th", 1, "n").is_none());
        assert!(reference_key("repo", "p", 1, "na\x01me").is_none());
    }

    #[test]
    fn primary_key_rejects_soh_in_segments() {
        assert!(primary_key("re\x01po", "p", 1, 1, "k", "n").is_none());
        assert!(primary_key("repo", "pa\x01th", 1, 1, "k", "n").is_none());
        assert!(primary_key("repo", "p", 1, 1, "ki\x01nd", "n").is_none());
        assert!(primary_key("repo", "p", 1, 1, "k", "na\x01me").is_none());
    }

    #[test]
    fn symbol_role_serde_round_trip() {
        let sym = AstSymbol {
            repo: "r".into(),
            path: "p".into(),
            line: 1,
            col: 1,
            kind: "function".into(),
            name: "foo".into(),
            role: SymbolRole::Reference,
        };
        let s = serde_json::to_string(&sym).unwrap();
        assert!(s.contains("\"role\":\"reference\""));
        let back: AstSymbol = serde_json::from_str(&s).unwrap();
        assert_eq!(back.role, SymbolRole::Reference);

        // Backward compat: JSON missing the `role` field deserializes to Definition.
        let legacy = r#"{"repo":"r","path":"p","line":1,"col":1,"kind":"function","name":"foo"}"#;
        let parsed: AstSymbol = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.role, SymbolRole::Definition);
    }

    #[test]
    fn queries_compile_for_all_languages() {
        // Touching each variant forces lazy compile and asserts the vendored .scm
        // sources are syntactically valid against the grammar versions in Cargo.toml.
        let _ = query_for(Lang::Rust);
        let _ = query_for(Lang::Python);
        let _ = query_for(Lang::Ts);
        let _ = query_for(Lang::Tsx);
        let _ = query_for(Lang::Go);
        let _ = query_for(Lang::Java);
        let _ = query_for(Lang::C);
        let _ = query_for(Lang::Cpp);
        let _ = query_for(Lang::Js);
    }
}
