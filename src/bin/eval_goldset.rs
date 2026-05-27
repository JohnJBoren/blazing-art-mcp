//! v0.2 Task 8: gold-set retrieval eval runner.
//!
//! Loads every `*.jsonl` under `eval/goldset/`, resolves each `repo_id` via
//! `eval/repos.json`, ingests each referenced repo into a fresh `Memory`, runs
//! the appropriate query for each record (definition vs reference), and
//! scores the index against the gold-set.
//!
//! Metrics reported per run:
//!   * Recall@{1, 5, 20} — fraction of records where the expected hit appears
//!     in the top-K (judged by `(repo, path, name)` match).
//!   * MRR (Mean Reciprocal Rank) over the top-20.
//!   * p50 / p99 query latency.
//!   * Per-category breakdowns (ambiguous, cross_language, long_tail, rename, negative).
//!
//! Outputs both a JSON file (machine-readable, consumed by
//! `scripts/check_eval_threshold.sh`) and a Markdown summary table for human
//! inspection. Files land at `eval/results/<git-sha>-<unix-ts>.{json,md}`.
//!
//! Usage:
//! ```bash
//! cargo run --release --bin eval_goldset
//! ```
//!
//! Flags (all optional):
//!   --goldset-dir <path>     default: eval/goldset/
//!   --repos      <path>      default: eval/repos.json
//!   --out-dir    <path>      default: eval/results/
//!   --top-k      <n>         default: 20
//!   --quiet                   suppress per-query trace

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use blazing_art_mcp::ingest::{self, AstSymbol};
use blazing_art_mcp::memory::Memory;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug, Clone)]
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

#[derive(Deserialize, Debug)]
struct ReposManifest {
    repos: HashMap<String, String>,
}

#[derive(Serialize, Debug, Default, Clone)]
struct CategoryMetrics {
    n: usize,
    recall_at_1: f64,
    recall_at_5: f64,
    recall_at_20: f64,
    mrr: f64,
    /// Only meaningful for the "negative" category: fraction that correctly returned 0.
    empty_correctness: f64,
}

#[derive(Serialize, Debug)]
struct EvalReport {
    git_sha: String,
    timestamp_unix: u64,
    n_records: usize,
    n_repos: usize,
    n_negative: usize,
    overall: CategoryMetrics,
    by_category: HashMap<String, CategoryMetrics>,
    latency_p50_us: u64,
    latency_p99_us: u64,
}

#[derive(Default)]
struct Args {
    goldset_dir: Option<PathBuf>,
    repos: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    top_k: usize,
    quiet: bool,
}

fn parse_args() -> Result<Args> {
    let mut a = Args { top_k: 20, ..Args::default() };
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--goldset-dir" => { a.goldset_dir = Some(PathBuf::from(&argv[i + 1])); i += 2; }
            "--repos" => { a.repos = Some(PathBuf::from(&argv[i + 1])); i += 2; }
            "--out-dir" => { a.out_dir = Some(PathBuf::from(&argv[i + 1])); i += 2; }
            "--top-k" => { a.top_k = argv[i + 1].parse()?; i += 2; }
            "--quiet" => { a.quiet = true; i += 1; }
            "--help" | "-h" => {
                eprintln!("eval_goldset [--goldset-dir DIR] [--repos JSON] [--out-dir DIR] [--top-k N] [--quiet]");
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown flag: {other}")),
        }
    }
    Ok(a)
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_records(dir: &Path) -> Result<Vec<GoldRecord>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let p = entry?.path();
        if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
            continue;
        }
        let text = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let r: GoldRecord = serde_json::from_str(line)
                .with_context(|| format!("{}:{}: parse error", p.display(), i + 1))?;
            out.push(r);
        }
    }
    Ok(out)
}

fn load_repos(p: &Path) -> Result<ReposManifest> {
    let text = std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
    serde_json::from_str(&text).context("parse repos.json")
}

fn current_git_sha(dir: &Path) -> String {
    Command::new("git")
        .arg("-C").arg(dir)
        .arg("rev-parse").arg("--short").arg("HEAD")
        .output()
        .ok()
        .and_then(|o| if o.status.success() { String::from_utf8(o.stdout).ok() } else { None })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Run a query for `record` against `mem`. Returns the top-K hits (ordered by
/// the index's natural prefix-scan order) and the query wall-clock latency.
fn run_query(mem: &Memory, record: &GoldRecord, top_k: usize) -> (Vec<AstSymbol>, std::time::Duration) {
    let prefix = match record.role.as_str() {
        "definition" => {
            // For definitions: prefer the inverted prefix sym\x01<kind>\x01<name>\x01.
            // Negative cases use a name that's expected to NOT appear at all,
            // and we still query under sym\x01function\x01<name>\x01 since that's
            // a valid empty prefix.
            if record.must_be_empty {
                format!("sym\x01function\x01{}\x01", record.name)
            } else {
                format!("sym\x01{}\x01{}\x01", record.kind, record.name)
            }
        }
        "reference" => format!("ref\x01{}\x01", record.name),
        other => panic!("unknown role: {other}"),
    };
    let t0 = Instant::now();
    let hits = mem.find_symbols(&prefix, top_k);
    let elapsed = t0.elapsed();
    (hits, elapsed)
}

/// Score one record. Returns:
///   * `rank`:   1-based position of the first matching hit in the top-K, or 0 if no match.
///   * `is_correct_empty`: only set for negative records — true if hits was empty.
fn score(record: &GoldRecord, hits: &[AstSymbol]) -> (usize, bool) {
    if record.must_be_empty {
        return (0, hits.is_empty());
    }
    for (i, h) in hits.iter().enumerate() {
        if h.repo == record.repo && h.path == record.path && h.name == record.name {
            return (i + 1, false);
        }
    }
    (0, false)
}

fn percentile(samples_us: &mut [u64], p: f64) -> u64 {
    if samples_us.is_empty() {
        return 0;
    }
    samples_us.sort_unstable();
    let idx = ((samples_us.len() as f64 - 1.0) * p).round() as usize;
    samples_us[idx]
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let manifest = manifest_dir();
    let goldset_dir = args.goldset_dir.unwrap_or_else(|| manifest.join("eval/goldset"));
    let repos_path = args.repos.unwrap_or_else(|| manifest.join("eval/repos.json"));
    let out_dir = args.out_dir.unwrap_or_else(|| manifest.join("eval/results"));
    let top_k = args.top_k;

    eprintln!("[eval] loading records from {}", goldset_dir.display());
    let records = load_records(&goldset_dir)?;
    eprintln!("[eval]   {} records loaded", records.len());

    eprintln!("[eval] loading repo map from {}", repos_path.display());
    let manifest_repos = load_repos(&repos_path)?;

    // Ingest each unique repo once.
    let mut indices: HashMap<String, Memory> = HashMap::new();
    let unique_repos: std::collections::HashSet<&String> = records.iter().map(|r| &r.repo).collect();
    for repo_id in &unique_repos {
        let rel = manifest_repos.repos.get(*repo_id).cloned().unwrap_or_else(|| {
            panic!("repos.json missing entry for repo_id '{repo_id}' (used in goldset)");
        });
        let path = manifest.join(&rel);
        eprintln!("[eval] ingesting repo_id={repo_id} from {}", path.display());
        let mem = Memory::new(1_000_000);
        let stats = ingest::ingest_repo(&mem, repo_id, &path);
        eprintln!("[eval]   files={}  symbols={}  errors={}", stats.files_parsed, stats.symbols_indexed, stats.errors.len());
        indices.insert((*repo_id).clone(), mem);
    }

    // Score every record.
    let mut latencies_us: Vec<u64> = Vec::with_capacity(records.len());
    let mut category_records: HashMap<String, Vec<(usize, bool)>> = HashMap::new();
    let mut all_records: Vec<(usize, bool)> = Vec::with_capacity(records.len());
    let mut n_negative = 0usize;

    for r in &records {
        let mem = indices.get(&r.repo).expect("repo ingested");
        let (hits, elapsed) = run_query(mem, r, top_k);
        let (rank, is_correct_empty) = score(r, &hits);
        latencies_us.push(elapsed.as_micros() as u64);
        all_records.push((rank, is_correct_empty));
        let cat = r.category.clone().unwrap_or_else(|| "uncategorized".to_string());
        category_records.entry(cat).or_default().push((rank, is_correct_empty));
        if r.must_be_empty {
            n_negative += 1;
        }
        if !args.quiet {
            let status = if r.must_be_empty {
                if is_correct_empty { "EMPTY-OK" } else { "EMPTY-FAIL" }
            } else if rank == 0 {
                "MISS"
            } else if rank == 1 {
                "RANK1"
            } else if rank <= 5 {
                "TOP5"
            } else {
                "TOP20"
            };
            eprintln!(
                "  {status:11} rank={rank:>2} lat={}μs  {} -> {}/{}",
                elapsed.as_micros(),
                r.query,
                r.kind,
                r.name
            );
        }
    }

    fn metrics_for(items: &[(usize, bool)], _top_k: usize) -> CategoryMetrics {
        let n = items.len();
        if n == 0 {
            return CategoryMetrics::default();
        }
        // An item is a "positive" gold record if its rank is set (>= 1) OR
        // (rank == 0 AND ok == false) — i.e., a positive that missed.
        // Negatives have rank == 0 AND we judge correctness by `ok`.
        let n_correct_empty = items.iter().filter(|(_, ok)| *ok).count() as f64;
        // Positive denominator: anything that isn't a "must_be_empty" record.
        // A must_be_empty record will have rank == 0; we tell it apart from a
        // missed positive by `ok == true`. So positives = items where !ok OR
        // rank > 0. Equivalently, items where rank > 0 || !ok.
        let positives: Vec<&(usize, bool)> =
            items.iter().filter(|(rank, ok)| *rank > 0 || !*ok).collect();
        let denom = if !positives.is_empty() {
            positives.len() as f64
        } else {
            // All records were negatives; recall metrics aren't meaningful, but
            // we keep the struct populated to avoid divide-by-zero.
            1.0
        };
        let r_at_1 = positives.iter().filter(|(r, _)| (1..=1).contains(r)).count() as f64;
        let r_at_5 = positives.iter().filter(|(r, _)| (1..=5).contains(r)).count() as f64;
        let r_at_20 = positives.iter().filter(|(r, _)| (1..=20).contains(r)).count() as f64;
        let mrr_sum: f64 = positives
            .iter()
            .filter(|(r, _)| *r > 0)
            .map(|(r, _)| 1.0 / (*r as f64))
            .sum();
        CategoryMetrics {
            n,
            recall_at_1: r_at_1 / denom,
            recall_at_5: r_at_5 / denom,
            recall_at_20: r_at_20 / denom,
            mrr: mrr_sum / denom,
            empty_correctness: n_correct_empty / (n as f64),
        }
    }

    let overall = metrics_for(&all_records, top_k);
    let mut by_category: HashMap<String, CategoryMetrics> = HashMap::new();
    for (cat, items) in &category_records {
        by_category.insert(cat.clone(), metrics_for(items, top_k));
    }

    let p50 = percentile(&mut latencies_us.clone(), 0.50);
    let p99 = percentile(&mut latencies_us.clone(), 0.99);

    let report = EvalReport {
        git_sha: current_git_sha(&manifest),
        timestamp_unix: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        n_records: records.len(),
        n_repos: unique_repos.len(),
        n_negative,
        overall,
        by_category,
        latency_p50_us: p50,
        latency_p99_us: p99,
    };

    std::fs::create_dir_all(&out_dir)?;
    let stem = format!("{}-{}", report.git_sha, report.timestamp_unix);
    let json_path = out_dir.join(format!("{stem}.json"));
    let md_path = out_dir.join(format!("{stem}.md"));

    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;

    let md = render_markdown(&report);
    std::fs::write(&md_path, md)?;

    eprintln!("\n[eval] wrote {}", json_path.display());
    eprintln!("[eval] wrote {}", md_path.display());
    eprintln!(
        "\n  Recall@1 / @5 / @20  =  {:.2} / {:.2} / {:.2}",
        report.overall.recall_at_1, report.overall.recall_at_5, report.overall.recall_at_20
    );
    eprintln!("  MRR                  =  {:.3}", report.overall.mrr);
    eprintln!("  Latency p50 / p99    =  {} / {} μs", report.latency_p50_us, report.latency_p99_us);
    Ok(())
}

fn render_markdown(r: &EvalReport) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Eval results — {} @ {}\n\n", r.git_sha, r.timestamp_unix));
    s.push_str(&format!("- Records: **{}**\n", r.n_records));
    s.push_str(&format!("- Repos ingested: **{}**\n", r.n_repos));
    s.push_str(&format!("- Negative cases: **{}**\n\n", r.n_negative));
    s.push_str("## Overall\n\n");
    s.push_str("| Metric | Value |\n|---|---|\n");
    s.push_str(&format!("| Recall@1 | {:.3} |\n", r.overall.recall_at_1));
    s.push_str(&format!("| Recall@5 | {:.3} |\n", r.overall.recall_at_5));
    s.push_str(&format!("| Recall@20 | {:.3} |\n", r.overall.recall_at_20));
    s.push_str(&format!("| MRR | {:.3} |\n", r.overall.mrr));
    s.push_str(&format!("| Latency p50 | {} μs |\n", r.latency_p50_us));
    s.push_str(&format!("| Latency p99 | {} μs |\n", r.latency_p99_us));
    s.push_str("\n## By category\n\n");
    s.push_str("| Category | n | Recall@1 | Recall@5 | Recall@20 | MRR | Empty-OK |\n|---|---|---|---|---|---|---|\n");
    let mut keys: Vec<_> = r.by_category.keys().collect();
    keys.sort();
    for k in keys {
        let m = &r.by_category[k];
        s.push_str(&format!(
            "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
            k, m.n, m.recall_at_1, m.recall_at_5, m.recall_at_20, m.mrr, m.empty_correctness
        ));
    }
    s
}
