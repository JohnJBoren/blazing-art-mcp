//! v0.2 Task 6: gold-set generator.
//!
//! Walks a target repo's git log, extracts identifier-like tokens from each
//! commit subject, and emits a JSONL record whenever exactly ONE declaration
//! in the repo's symbol index matches the token. The result is a synthetic
//! `(query, expected)` retrieval gold set that we can score the ART RAG
//! against in CI (Task 8).
//!
//! Heuristics:
//!   * Skip merges, skip subjects shorter than 6 words.
//!   * Skip subjects starting with `bump`, `fmt`, `chore`, `wip`, `merge`,
//!     `revert`, `typo`, `test:`, `ci:`, `docs:` (case-insensitive).
//!   * For each remaining subject, scan for tokens of 3+ chars that look like
//!     identifiers (`[A-Za-z_][A-Za-z0-9_]{2,30}`) and filter out common stop
//!     words.
//!   * For each token, look up `sym\x01<kind>\x01<token>\x01` for each
//!     declaration kind we care about. If exactly one matches, emit a record.
//!   * Dedupe final output by `(path, name)`.
//!
//! This is high-precision, modest-recall by design — the eval runner needs
//! reliably correct ground-truth labels far more than it needs volume.
//!
//! Usage:
//! ```bash
//! cargo run --release --bin build_goldset -- \
//!     --repo /path/to/repo \
//!     --out  eval/goldset/<id>.jsonl \
//!     [--repo-id <id>] [--max-records 200] [--lookback-commits 5000]
//! ```

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use blazing_art_mcp::{ingest, memory::Memory};
use regex::Regex;
use serde::Serialize;

const DECL_KINDS: &[&str] = &["function", "class", "method", "type", "interface", "module", "macro", "constant"];

/// Subjects starting with these tokens are project-meta noise, not feature work.
const BOILERPLATE_PREFIXES: &str = r"(?i)^\s*(bump|fmt|chore|wip|merge|revert|typo|test:|ci:|docs:|deps?:|lint|format|cleanup|nit:|rfc:)";

/// Stop words that match the identifier regex but are unlikely to be useful queries.
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "into", "this", "that", "when", "use", "fix", "add",
    "make", "new", "now", "let", "set", "get", "all", "any", "out", "via", "off", "but",
    "can", "not", "are", "has", "have", "was", "were", "you", "your", "our", "than",
    "test", "tests", "spec", "specs", "TODO", "FIXME",
];

#[derive(Serialize, Debug)]
struct GoldRecord {
    /// Natural-language query: the commit subject.
    query: String,
    repo: String,
    path: String,
    kind: String,
    name: String,
    /// "definition" for now; future tasks may emit "reference" records.
    role: &'static str,
    /// Provenance for debuggability.
    source_commit: String,
}

#[derive(Default)]
struct Args {
    repo: Option<PathBuf>,
    out: Option<PathBuf>,
    repo_id: Option<String>,
    max_records: usize,
    lookback_commits: usize,
    emit_per_commit: usize,
    verbose: bool,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        max_records: 200,
        lookback_commits: 5_000,
        emit_per_commit: 1,
        ..Args::default()
    };
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--repo" => { a.repo = Some(PathBuf::from(&argv[i + 1])); i += 2; }
            "--out" => { a.out = Some(PathBuf::from(&argv[i + 1])); i += 2; }
            "--repo-id" => { a.repo_id = Some(argv[i + 1].clone()); i += 2; }
            "--max-records" => { a.max_records = argv[i + 1].parse()?; i += 2; }
            "--lookback-commits" => { a.lookback_commits = argv[i + 1].parse()?; i += 2; }
            "--emit-per-commit" => { a.emit_per_commit = argv[i + 1].parse()?; i += 2; }
            "--verbose" | "-v" => { a.verbose = true; i += 1; }
            "--help" | "-h" => {
                eprintln!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown flag: {other}\n{USAGE}")),
        }
    }
    Ok(a)
}

const USAGE: &str = r#"build_goldset --repo <path> --out <jsonl> [options]

  --repo <path>             Path to a git repository (REQUIRED)
  --out  <path>             Output JSONL file (REQUIRED)
  --repo-id <id>            Identifier for keys; default = basename of --repo
  --max-records <n>         Cap output size (default: 200)
  --lookback-commits <n>    Recent commits to consider (default: 5000)
  --emit-per-commit <n>     Max records per commit (default: 1; emit top-N
                            by token specificity if subject mentions multiple
                            uniquely-matching identifiers)
  --verbose, -v             Print rejected subjects + match traces
"#;

fn main() -> Result<()> {
    let args = parse_args()?;
    let repo = args.repo.as_ref().context("--repo is required (see --help)")?;
    let out = args.out.as_ref().context("--out is required (see --help)")?;
    let repo_id = args.repo_id.unwrap_or_else(|| {
        repo.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo")
            .to_string()
    });

    if !repo.exists() {
        return Err(anyhow!("--repo path does not exist: {}", repo.display()));
    }

    eprintln!("[goldset] ingesting {} as repo_id={repo_id}", repo.display());
    let mem = Memory::new(1_000_000);
    let stats = ingest::ingest_repo(&mem, &repo_id, repo);
    eprintln!(
        "[goldset]   files={} symbols={} errors={}",
        stats.files_parsed,
        stats.symbols_indexed,
        stats.errors.len()
    );

    eprintln!("[goldset] reading git log (last {} commits)…", args.lookback_commits);
    let log = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("log")
        .arg("--no-merges")
        .arg("--pretty=format:%H %s")
        .arg(format!("-{}", args.lookback_commits))
        .output()
        .context("failed to invoke git (is it installed and is --repo a git repo?)")?;
    if !log.status.success() {
        return Err(anyhow!(
            "git log failed: {}",
            String::from_utf8_lossy(&log.stderr).trim()
        ));
    }
    let log_text = String::from_utf8_lossy(&log.stdout);
    eprintln!("[goldset]   {} commit lines from git", log_text.lines().count());

    let boilerplate = Regex::new(BOILERPLATE_PREFIXES).expect("static regex must compile");
    let ident_re = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]{2,30})\b").expect("static regex must compile");
    let stop: HashSet<&str> = STOP_WORDS.iter().copied().collect();

    let mut records: Vec<GoldRecord> = Vec::new();
    let mut considered = 0usize;
    let mut rejected_short = 0usize;
    let mut rejected_boilerplate = 0usize;
    let mut rejected_no_match = 0usize;

    for line in log_text.lines() {
        if records.len() >= args.max_records {
            break;
        }
        let mut parts = line.splitn(2, ' ');
        let sha = parts.next().unwrap_or("");
        let subject = parts.next().unwrap_or("");
        if sha.len() < 7 || subject.is_empty() {
            continue;
        }
        considered += 1;

        if subject.split_whitespace().count() < 6 {
            rejected_short += 1;
            if args.verbose {
                eprintln!("  SHORT: {subject}");
            }
            continue;
        }
        if boilerplate.is_match(subject) {
            rejected_boilerplate += 1;
            if args.verbose {
                eprintln!("  BOILERPLATE: {subject}");
            }
            continue;
        }

        // Smarter algorithm (v0.3 Task 4): for each candidate token, count
        // matches across ALL declaration kinds. Only emit when EXACTLY one
        // matching symbol exists across the whole index — catches the case
        // where a token matches a function in one place AND a class somewhere
        // else (the old algorithm picked the function silently). Then score
        // surviving candidates by token length and emit the most specific
        // one (or up to args.emit_per_commit, when configured).
        struct Candidate {
            #[allow(dead_code)]
            token: String,
            hit: blazing_art_mcp::ingest::AstSymbol,
            score: f64,
        }
        let mut candidates: Vec<Candidate> = Vec::new();

        for cap in ident_re.captures_iter(subject) {
            let tok = cap.get(1).unwrap().as_str();
            if tok.len() < 4 || stop.contains(tok.to_ascii_lowercase().as_str()) {
                continue;
            }
            if tok.as_bytes().contains(&0x01) {
                continue;
            }

            // Count matches across ALL kinds for this token.
            let mut all_matches: Vec<(&'static str, blazing_art_mcp::ingest::AstSymbol)> =
                Vec::new();
            for kind in DECL_KINDS {
                let prefix = format!("sym\x01{kind}\x01{tok}\x01");
                let hits = mem.find_symbols(&prefix, 5);
                for h in hits {
                    all_matches.push((kind, h));
                }
                // Early exit: if we already have >1 hit, this token is
                // ambiguous and we won't emit it.
                if all_matches.len() > 1 {
                    break;
                }
            }
            if all_matches.len() != 1 {
                continue;
            }
            let (_kind, hit) = all_matches.into_iter().next().unwrap();

            // Score: longer tokens are more specific. Multiplier helps
            // ties go to longer tokens but doesn't dominate over future
            // refinements (e.g., rarity score).
            let score = tok.len() as f64;
            candidates.push(Candidate {
                token: tok.to_string(),
                hit,
                score,
            });
        }

        if candidates.is_empty() {
            rejected_no_match += 1;
            continue;
        }

        // Sort by score descending (most specific first).
        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        let take = args.emit_per_commit.max(1);
        for cand in candidates.into_iter().take(take) {
            records.push(GoldRecord {
                query: subject.to_string(),
                repo: cand.hit.repo.clone(),
                path: cand.hit.path.clone(),
                kind: cand.hit.kind.clone(),
                name: cand.hit.name.clone(),
                role: "definition",
                source_commit: sha.to_string(),
            });
            if records.len() >= args.max_records {
                break;
            }
        }
    }

    // Dedupe by (path, name) — keep the first occurrence.
    let mut seen: HashSet<(String, String)> = HashSet::new();
    records.retain(|r| seen.insert((r.path.clone(), r.name.clone())));

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).context("create output dir")?;
    }
    let mut buf = String::new();
    for r in &records {
        buf.push_str(&serde_json::to_string(r)?);
        buf.push('\n');
    }
    std::fs::write(out, buf).context("write output JSONL")?;

    eprintln!("[goldset] DONE. wrote {} records to {}", records.len(), out.display());
    eprintln!(
        "[goldset]   considered={considered}  short={rejected_short}  \
         boilerplate={rejected_boilerplate}  no_match={rejected_no_match}"
    );
    if args.verbose {
        for r in records.iter().take(10) {
            eprintln!("  SAMPLE: {} -> {}/{}/{}", r.query, r.kind, r.name, r.path);
        }
    }
    Ok(())
}
