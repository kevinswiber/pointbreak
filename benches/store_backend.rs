//! Baseline measurement harness for the durable event store's file backend.
//!
//! Run with `cargo bench --features bench`. It measures the three metrics a
//! future log-structured backend would be compared against: whole-log read
//! latency, single-append latency, and on-disk amplification. The synthetic
//! groups are generated in-process and need nothing external — anyone can run
//! them. An optional real-world read-all sample runs only when
//! `SHORE_BENCH_FIXTURE` points at an existing store directory; it is skipped
//! otherwise, so the harness has no baked-in paths. No alternative backend is
//! built here; this only establishes the file backend's numbers.
//!
//! See `docs/benchmarking.md` for `SHORE_BENCH_FIXTURE`, the expected store-dir
//! layout, and the schema-currency requirement.

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput};
use shoreline::bench_support::StoreBenchHarness;
use shoreline::model::RevisionId;
use shoreline::session::{
    EventVerificationPolicy, ReviewHistoryOptions, RevisionListOptions, RevisionOverviewsOptions,
    RevisionShowOptions, event_log_head_marker, list_revisions, review_history, show_revision,
    show_revision_overviews,
};

/// Synthetic store sizes. The largest is where a many-small-files layout starts
/// to slow whole-log reads.
const SIZES: &[usize] = &[100, 1_000, 10_000];

/// Whole-log read latency — the dominant access pattern, since projection,
/// inventory, and bundle all read the full event log. Measured over synthetic
/// stores and, when present, the developer-local captured fixture.
fn read_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_all");
    for &n in SIZES {
        let dir = tempfile::tempdir().expect("a temp dir");
        let harness = StoreBenchHarness::open(dir.path().join(".shore/data"));
        harness.populate(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("synthetic", n), &harness, |b, h| {
            b.iter(|| black_box(h.read_all()));
        });
    }

    match fixture_store_dir() {
        Some(store_dir) => {
            let harness = StoreBenchHarness::open(&store_dir);
            match harness.try_read_all() {
                Ok(count) => {
                    group.throughput(Throughput::Elements(count as u64));
                    group.bench_with_input(BenchmarkId::new("fixture", count), &harness, |b, h| {
                        b.iter(|| black_box(h.read_all()));
                    });
                }
                Err(error) => eprintln!(
                    "skipping SHORE_BENCH_FIXTURE read-all ({}): {error}",
                    store_dir.display()
                ),
            }
        }
        None => eprintln!(
            "skipping fixture read-all: set SHORE_BENCH_FIXTURE to a store directory \
             for a real-world sample"
        ),
    }

    group.finish();
}

/// Single-append latency. Append cost is independent of warm-store size (one
/// exclusive create), so the warm store only ensures the events directory
/// already exists; each iteration writes a fresh key so it is a genuine create,
/// never an idempotent no-op.
fn append(c: &mut Criterion) {
    let mut group = c.benchmark_group("append");
    for &n in SIZES {
        let dir = tempfile::tempdir().expect("a temp dir");
        let harness = StoreBenchHarness::open(dir.path().join(".shore/data"));
        harness.populate(n);
        group.bench_with_input(BenchmarkId::new("warm", n), &harness, |b, h| {
            b.iter(|| h.append_one());
        });
    }
    group.finish();
}

/// On-disk amplification is a one-shot measurement, not a timing — report it
/// before the timed groups so a run always surfaces it.
fn report_disk_amplification() {
    eprintln!("disk amplification — file backend, synthetic events (on-disk / logical bytes):");
    for &n in SIZES {
        let dir = tempfile::tempdir().expect("a temp dir");
        let harness = StoreBenchHarness::open(dir.path().join(".shore/data"));
        harness.populate(n);
        let usage = harness.byte_usage();
        let ratio = if usage.logical == 0 {
            0.0
        } else {
            usage.physical as f64 / usage.logical as f64
        };
        eprintln!(
            "  n={n:>6}: logical={:>10} B  on-disk={:>10} B  amplification={ratio:.2}x",
            usage.logical, usage.physical
        );
    }
}

/// The `/api/revisions` overview cost, batch vs the per-revision N+1 it replaced,
/// over a real captured store. The inspector lists the entries once (in
/// `revisions_json`) and hands them to `revision_overviews`; both the old loop and
/// the new batch start from those ids, so the ids are listed once up front, not
/// inside either timed path. `loop_show_revision` reproduces the old path:
/// `show_revision` once per revision, each re-reading and re-folding the whole
/// event log plus the advisory member-readback loop. `batch` is the single-pass
/// `show_revision_overviews`. `list_revisions` is timed alone to expose the shared
/// floor both paths sit on (its own per-revision git-liveness pass). The small
/// sample size keeps the slow cases bounded — a one-off measurement, not a CI gate.
/// Skipped unless a repo is supplied.
fn revision_overviews(c: &mut Criterion) {
    let Some(repo) = fixture_repo_dir() else {
        eprintln!(
            "skipping revision_overviews A/B: set SHORE_BENCH_REPO to a repo root (or \
             SHORE_BENCH_FIXTURE to its <repo>/.git/shore store) for the live sample"
        );
        return;
    };
    let ids: Vec<RevisionId> =
        match list_revisions(RevisionListOptions::new(&repo).with_read_for_display(true)) {
            Ok(result) => result.entries.into_iter().map(|e| e.revision_id).collect(),
            Err(error) => {
                eprintln!(
                    "skipping revision_overviews A/B ({}): {error}",
                    repo.display()
                );
                return;
            }
        };
    let n = ids.len();

    let mut group = c.benchmark_group("revision_overviews");
    group.sample_size(10);
    group.throughput(Throughput::Elements(n as u64));
    group.bench_function(BenchmarkId::new("list_revisions", n), |b| {
        b.iter(|| {
            black_box(
                list_revisions(RevisionListOptions::new(&repo).with_read_for_display(true))
                    .expect("list revisions"),
            )
        });
    });
    group.bench_function(BenchmarkId::new("loop_show_revision", n), |b| {
        // The helper black-boxes each `show_revision` result internally; it returns
        // unit, so it is not wrapped again here.
        b.iter(|| overviews_via_per_revision_loop(&repo, &ids));
    });
    group.bench_function(BenchmarkId::new("batch", n), |b| {
        b.iter(|| {
            black_box(
                show_revision_overviews(
                    RevisionOverviewsOptions::new(&repo)
                        .with_revisions(ids.clone())
                        .with_read_for_display(true),
                )
                .expect("batch overviews"),
            )
        });
    });
    group.finish();
}

/// Reproduce the retired per-revision overview path over already-listed ids: one
/// `show_revision` per revision (the N+1), each with the advisory verification
/// policy the old overview path set.
fn overviews_via_per_revision_loop(repo: &Path, ids: &[RevisionId]) {
    for id in ids {
        black_box(
            show_revision(
                RevisionShowOptions::new(repo)
                    .with_revision_id(id.clone())
                    .with_exact(true)
                    .with_read_for_display(true)
                    .with_verification_policy(EventVerificationPolicy::advisory()),
            )
            .expect("show revision"),
        );
    }
}

/// The `/api/freshness` poll cost, the cheap head marker vs the old per-poll full
/// read. Case (a) is `review_history(include_body=false)` — the full fold + event
/// set hash the old probe paid every tick. Case (b) is `event_log_head_marker` — a
/// dirent count, no event bytes read. Skipped unless a repo is supplied.
fn freshness(c: &mut Criterion) {
    let Some(repo) = fixture_repo_dir() else {
        return; // the skip notice is already printed by revision_overviews
    };

    let mut group = c.benchmark_group("freshness");
    group.bench_function("review_history_full_read", |b| {
        b.iter(|| {
            black_box(
                review_history(
                    ReviewHistoryOptions::new(&repo)
                        .with_include_body(false)
                        .with_read_for_display(true),
                )
                .expect("review history"),
            )
        });
    });
    group.bench_function("event_log_head_marker", |b| {
        b.iter(|| black_box(event_log_head_marker(&repo).expect("head marker")));
    });
    group.finish();
}

/// An optional real-world store to sample read-all over, supplied entirely by the
/// caller through `SHORE_BENCH_FIXTURE` — no path is baked into the source, so the
/// harness stays runnable by anyone. Returns the store directory only when the env
/// var is set and its event log is present; an unset or absent fixture is skipped
/// rather than failing the run.
fn fixture_store_dir() -> Option<PathBuf> {
    let store_dir = PathBuf::from(std::env::var_os("SHORE_BENCH_FIXTURE")?);
    store_dir.join("events").is_dir().then_some(store_dir)
}

/// The repo root for the API-level benches (`show_revision*`, freshness), which
/// resolve their store from a repo path rather than a store dir. Prefers an
/// explicit `SHORE_BENCH_REPO`; otherwise derives it from a `<repo>/.git/shore`
/// `SHORE_BENCH_FIXTURE`. Returns `None` (skip) when neither yields a git repo.
fn fixture_repo_dir() -> Option<PathBuf> {
    if let Some(repo) = std::env::var_os("SHORE_BENCH_REPO") {
        let repo = PathBuf::from(repo);
        return repo.join(".git").exists().then_some(repo);
    }
    let store = PathBuf::from(std::env::var_os("SHORE_BENCH_FIXTURE")?);
    // <repo>/.git/shore -> <repo>
    if store.file_name()?.to_str()? == "shore" {
        let git = store.parent()?;
        if git.file_name()?.to_str()? == ".git" {
            return git.parent().map(Path::to_path_buf);
        }
    }
    None
}

fn main() {
    report_disk_amplification();
    let mut criterion = Criterion::default().configure_from_args();
    read_all(&mut criterion);
    append(&mut criterion);
    revision_overviews(&mut criterion);
    freshness(&mut criterion);
    criterion.final_summary();
}
