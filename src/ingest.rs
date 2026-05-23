//! Tree-sitter AST ingestion. Parses files in a repo into ASTs, walks them,
//! and writes one primary key + one inverted key per declaration-kind node
//! into the shared `Memory` symbol index.
//!
//! Schema (locked decision, see memory/feedback_blazing_art_decisions.md):
//!
//!   primary  = "pri\x01<repo>\x01<rel_path>\x01<line5>:<col3>:<kind>\x01<name>"
//!   inverted = "sym\x01<kind>\x01<name>\x01<repo>\x01<rel_path>:<line>"
//!
//! Segment separator is `\x01` (SOH), NOT `\x00`, because `CString::new`
//! rejects interior NULs while still adding the trailing NUL that
//! `blart::TreeMap` requires for `NoPrefixesBytes`.
//!
//! Symbol filter: only declaration kinds are stored, not every identifier.
//! A 10k-line file has ~50k AST nodes total but ~200 declarations; storing
//! all 50k would OOM at scale. The `is_declaration` table per language
//! captures the relevant subset.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tree_sitter::{Language, Node, Parser};
use walkdir::WalkDir;

use crate::memory::Memory;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AstSymbol {
    pub repo: String,
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub kind: String,
    pub name: String,
}

#[derive(Default, Serialize, Debug)]
pub struct IngestStats {
    pub files_parsed: usize,
    pub files_skipped: usize,
    pub symbols_indexed: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
enum Lang {
    Rust,
    Python,
    Ts,
    Tsx,
}

impl Lang {
    fn for_extension(ext: &str) -> Option<Lang> {
        match ext {
            "rs" => Some(Lang::Rust),
            "py" => Some(Lang::Python),
            "ts" | "mts" | "cts" => Some(Lang::Ts),
            "tsx" => Some(Lang::Tsx),
            _ => None,
        }
    }

    fn ts_language(self) -> Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    /// Is this AST node a "declaration" — something an agent would want to look up by name?
    fn is_declaration(self, kind: &str) -> bool {
        match self {
            Lang::Rust => matches!(
                kind,
                "function_item"
                    | "struct_item"
                    | "enum_item"
                    | "trait_item"
                    | "impl_item"
                    | "mod_item"
                    | "type_item"
                    | "const_item"
                    | "static_item"
                    | "union_item"
                    | "macro_definition"
            ),
            Lang::Python => matches!(kind, "function_definition" | "class_definition"),
            Lang::Ts | Lang::Tsx => matches!(
                kind,
                "function_declaration"
                    | "class_declaration"
                    | "interface_declaration"
                    | "type_alias_declaration"
                    | "enum_declaration"
                    | "method_definition"
                    | "abstract_class_declaration"
            ),
        }
    }

    /// Tree-sitter exposes the symbol's identifier as a child node with kind
    /// `identifier`, `type_identifier`, or `property_identifier` (per language).
    /// Walk children of a declaration to find the first one that's a name.
    fn extract_name(self, decl: Node, source: &[u8]) -> Option<String> {
        // Most languages name a declaration via a `name` field on the node.
        if let Some(n) = decl.child_by_field_name("name") {
            return n.utf8_text(source).ok().map(str::to_string);
        }
        // Fallback: scan children for an identifier-shaped kind.
        let mut walker = decl.walk();
        for child in decl.children(&mut walker) {
            let k = child.kind();
            if matches!(
                k,
                "identifier" | "type_identifier" | "property_identifier" | "field_identifier"
            ) {
                if let Ok(t) = child.utf8_text(source) {
                    return Some(t.to_string());
                }
            }
        }
        None
    }
}

/// Encode a primary key. Returns None if any segment contains the SOH separator,
/// which would break the schema's prefix-scan invariants.
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

/// Walk a tree-sitter AST and emit one (key, AstSymbol) pair per declaration node,
/// for both the primary and inverted key namespaces.
fn collect_symbols(
    node: Node,
    source: &[u8],
    lang: Lang,
    repo: &str,
    path: &str,
    out: &mut Vec<(String, AstSymbol)>,
) {
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        let kind = n.kind();
        if lang.is_declaration(kind) {
            if let Some(name) = lang.extract_name(n, source) {
                let pos = n.start_position();
                let sym = AstSymbol {
                    repo: repo.to_string(),
                    path: path.to_string(),
                    line: pos.row as u32 + 1, // 1-based for human readability
                    col: pos.column as u32 + 1,
                    kind: kind.to_string(),
                    name: name.clone(),
                };
                if let Some(pk) = primary_key(repo, path, sym.line, sym.col, kind, &name) {
                    out.push((pk, sym.clone()));
                }
                if let Some(ik) = inverted_key(repo, path, sym.line, kind, &name) {
                    out.push((ik, sym));
                }
            }
        }
        // Push children so we descend into nested decls (e.g., methods inside impls).
        for child in n.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Parse a single file, return its (key, symbol) pairs.
/// Returns Err if the file can't be read; Ok(empty) if the file kind isn't supported.
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

    let mut parser = Parser::new();
    parser
        .set_language(&lang.ts_language())
        .map_err(|e| anyhow::anyhow!("set_language failed for {ext}: {e}"))?;

    let Some(tree) = parser.parse(&source, None) else {
        anyhow::bail!("parse returned None for {}", path.display());
    };

    let mut out = Vec::new();
    collect_symbols(tree.root_node(), &source, lang, repo, &rel, &mut out);
    Ok(out)
}

/// Public entry point. Walks `repo_path` recursively, parses every .rs/.py/.ts/.tsx,
/// and inserts the resulting symbol entries into `memory`.
pub fn ingest_repo(memory: &Memory, repo_id: &str, repo_path: &Path) -> IngestStats {
    let mut stats = IngestStats::default();

    if !repo_path.exists() {
        stats.errors.push(format!("repo path does not exist: {}", repo_path.display()));
        return stats;
    }

    // Reject SOH (\x01) in the repo id since it would corrupt the key schema.
    if repo_id.as_bytes().contains(&0x01) {
        stats.errors.push("repo_id contains SOH separator byte (\\x01)".to_string());
        return stats;
    }

    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            // Skip noisy directories that bloat ingest with no payoff.
            !matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | ".venv" | "__pycache__" | "dist" | "build"
            )
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        match ingest_file(entry.path(), repo_id, repo_path) {
            Ok(pairs) => {
                if pairs.is_empty() {
                    continue;
                }
                stats.files_parsed += 1;
                for (key, sym) in pairs {
                    if memory.add_symbol(&key, sym) {
                        stats.symbols_indexed += 1;
                    }
                }
            }
            Err(e) => {
                stats.files_skipped += 1;
                stats.errors.push(format!("{}: {e}", entry.path().display()));
            }
        }
    }

    stats
}
