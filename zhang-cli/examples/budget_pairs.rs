//! Run the budget baseline (`examples/budget_baseline.rs`) in pairs, the native budget implementation against a
//! candidate (the budget plugin), and judge them with `budget_compare` (`examples/budget_compare.rs`): the fast
//! procedure of P4 on the "Budget Baseline" page of the developer docs
//! (`docs/src/content/docs/developers/budget-baseline.md`, "The fast procedure of P4").
//!
//! ```sh
//! budget_pairs --out <dir> --native <budget_baseline> [--env VAR=value]... [--native-env VAR=value]... \
//!     [--candidate <budget_baseline>] [--candidate-env VAR=value]... \
//!     [--ledgers 10000tx-30b-60m,100000tx-30b-60m] [--modes e2e,requests] [--pairs 10] [--most-pairs 15] \
//!     [--requests-pairs 60] [--rounds 200] [--cpus 16-63] [--cpus-per-side 8] [--max-invalid <n>] \
//!     [--compare <budget_compare>] [-- <options of budget_compare, such as --expect-analyzer zhang:budget>]
//! ```
//!
//! - **A pair's two runs run at the same time**, each pinned to a CPU set of its own (`ZHANG_BUDGET_BASELINE_CPUS`, with
//!   as many worker threads as CPUs), so that both see the same state of the machine, its memory's slow periods
//!   included. The CPUs of `--cpus` are cut into sets of `--cpus-per-side` and the sets into slots of two, one pair a
//!   slot: 16-63 by 8 is three pairs at once. The two sides swap the sets of their slot from pair to pair.
//! - **Each mode and ledger gets its valid pairs**: `--pairs` for the `e2e` (and full) mode, `--requests-pairs` for the
//!   `requests` mode, whose runs take `--rounds` rounds of warm requests. A pair with a run that is invalid (a build
//!   ran, the cgroup was throttled or its tasks waited for a CPU, a timer kept fewer than its clean rounds, the run
//!   failed) is moved to `<out>/invalid/` and run again, up to `--max-invalid` times a mode and ledger (as many as its
//!   pairs by default).
//! - **Then `budget_compare` judges them**, and while a verdict of the `e2e` mode is inconclusive and its ledger has
//!   fewer than `--most-pairs` valid pairs, more pairs are run (at least as many as there are slots) and they are
//!   judged again. The `requests` mode gets no more: its pairs are all run before the judgement.
//!
//! The binaries are run as they are: one binary for both sides with an environment each (`--native-env
//! ZHANG_BUDGET_IMPL=native --candidate-env ZHANG_BUDGET_IMPL=plugin`), or two; `--env` sets a variable on both sides
//! (`ZHANG_BUDGET_BASELINE_COMMIT`). The reports go to `<out>/<side>/<mode>/<ledger>/<pair>/<ledger>.json`, with the
//! run's stdout and stderr, which is budget_compare's layout; `<out>/runs.jsonl` records every pair run,
//! `<out>/compare.txt` and `<out>/summary.json` the last judgement. Run again with the same `--out`, it keeps the valid
//! pairs it finds and runs what is missing. A run's timings only mean something in a bench window, with no build
//! running in the machine: on the build pod, run this under `kbuild --bench`. It exits with budget_compare's status, or
//! a failure of its own.

use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// the timers of an `e2e` report, each of which must have kept its clean rounds
const TIMERS: [&str; 6] = [
    "write_then_budgets",
    "write_then_errors",
    "reload_then_budgets",
    "reload_then_errors",
    "start_then_budgets",
    "start_then_errors",
];
/// the modes of the bench a pair can run, by what they cost: the longest are started first
const MODES: [&str; 3] = ["full", "e2e", "requests"];
/// how often the runs are looked at
const POLL: Duration = Duration::from_millis(200);
/// the pairs of the `requests` mode: `budget_compare` judges the named-query p95 on 60 pairs
const REQUESTS_PAIRS: usize = 60;
/// the rounds of warm requests of a `requests` run (`ZHANG_BUDGET_BASELINE_ROUNDS`)
const REQUESTS_ROUNDS: usize = 200;

/// one side of the comparison: its name in the layout, the bench's binary and the environment it runs with
struct Side {
    name: &'static str,
    binary: PathBuf,
    env: Vec<(String, String)>,
}

struct Config {
    out: PathBuf,
    sides: [Side; 2],
    ledgers: Vec<String>,
    modes: Vec<String>,
    pairs: usize,
    most: usize,
    /// the pairs of the `requests` mode, all run before they are judged
    requests_pairs: usize,
    /// the CPU sets of each slot, as `taskset -c` lists
    slots: Vec<[String; 2]>,
    per_side: usize,
    rounds: usize,
    max_invalid: Option<usize>,
    compare: PathBuf,
    compare_args: Vec<String>,
}

impl Config {
    /// the valid pairs `group` gets before it is judged: `--requests-pairs` for the `requests` mode, `--pairs` for the
    /// others
    fn target(&self, group: &Group) -> usize {
        if group.mode == "requests" {
            self.requests_pairs
        } else {
            self.pairs
        }
    }

    /// the valid pairs `group` gets at most, with the pairs added while a verdict is inconclusive: `--most-pairs`, but
    /// for the `requests` mode, whose pairs are all run before the judgement
    fn most(&self, group: &Group) -> usize {
        if group.mode == "requests" {
            self.requests_pairs
        } else {
            self.most
        }
    }
}

/// what a pair measures: a mode of the bench on a ledger
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Group {
    mode: String,
    ledger: String,
}

impl Group {
    fn dir(&self, root: &Path, side: &str) -> PathBuf {
        root.join(side).join(&self.mode).join(&self.ledger)
    }

    /// the longest first: the full mode, then `e2e`, then `requests`, the larger ledger first
    fn cost(&self) -> (usize, std::cmp::Reverse<usize>) {
        let mode = MODES.iter().position(|it| *it == self.mode).unwrap_or(MODES.len());
        let transactions = self.ledger.split("tx").next().and_then(|it| it.parse().ok()).unwrap_or(0);
        (mode, std::cmp::Reverse(transactions))
    }
}

/// a pair in flight: its two runs, started together
struct Running {
    group: Group,
    pair: usize,
    runs: Vec<(Child, PathBuf, String)>,
    started: Instant,
    started_at: String,
}

fn main() -> ExitCode {
    let config = match parse(std::env::args().skip(1).collect()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("budget_pairs: {error}");
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        }
    };
    match run(&config) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("budget_pairs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> &'static str {
    "usage: budget_pairs --out <dir> --native <budget_baseline> [--env VAR=value]... [--native-env VAR=value]... [--candidate <budget_baseline>] \
     [--candidate-env VAR=value]... [--ledgers a,b] [--modes e2e,requests] [--pairs 10] [--most-pairs 15] \
     [--requests-pairs 60] [--rounds 200] [--cpus 16-63] [--cpus-per-side 8] [--max-invalid <n>] \
     [--compare <budget_compare>] [-- <budget_compare options>]"
}

fn parse(args: Vec<String>) -> Result<Config, String> {
    let mut args = args.into_iter();
    let (mut out, mut native, mut candidate, mut native_env, mut candidate_env) = (None, None, None, vec![], vec![]);
    // the environment of both sides, before each side's own
    let mut env = vec![];
    let mut ledgers = "10000tx-30b-60m,100000tx-30b-60m".to_owned();
    let mut modes = "e2e,requests".to_owned();
    let (mut pairs, mut most, mut cpus, mut per_side) = (10, 15, "16-63".to_owned(), 8);
    let (mut rounds, mut max_invalid, mut compare, mut compare_args) = (REQUESTS_ROUNDS, None, None, vec![]);
    let mut requests_pairs = REQUESTS_PAIRS;
    let number = |flag: &str, value: Option<String>| -> Result<usize, String> {
        value
            .as_deref()
            .and_then(|it| it.parse::<usize>().ok())
            .filter(|it| *it > 0)
            .ok_or_else(|| format!("{flag} needs a number above 0"))
    };
    let assignment = |flag: &str, value: Option<String>| -> Result<(String, String), String> {
        let value = value.ok_or_else(|| format!("{flag} needs VAR=value"))?;
        let (name, value) = value.split_once('=').ok_or_else(|| format!("{flag} {value}: expect VAR=value"))?;
        Ok((name.to_owned(), value.to_owned()))
    };
    while let Some(arg) = args.next() {
        let missing = || format!("{arg} needs a value");
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(args.next().ok_or_else(missing)?)),
            "--native" => native = Some(PathBuf::from(args.next().ok_or_else(missing)?)),
            "--candidate" => candidate = Some(PathBuf::from(args.next().ok_or_else(missing)?)),
            "--env" => env.push(assignment(&arg, args.next())?),
            "--native-env" => native_env.push(assignment(&arg, args.next())?),
            "--candidate-env" => candidate_env.push(assignment(&arg, args.next())?),
            "--ledgers" => ledgers = args.next().ok_or_else(missing)?,
            "--modes" => modes = args.next().ok_or_else(missing)?,
            "--pairs" => pairs = number(&arg, args.next())?,
            "--most-pairs" => most = number(&arg, args.next())?,
            "--cpus" => cpus = args.next().ok_or_else(missing)?,
            "--cpus-per-side" => per_side = number(&arg, args.next())?,
            "--rounds" => rounds = number(&arg, args.next())?,
            "--requests-pairs" => requests_pairs = number(&arg, args.next())?,
            "--max-invalid" => max_invalid = Some(number(&arg, args.next())?),
            "--compare" => compare = Some(PathBuf::from(args.next().ok_or_else(missing)?)),
            "--" => compare_args.extend(args.by_ref()),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    let out = out.ok_or("--out is required")?;
    let native = native.ok_or("--native is required")?;
    let candidate = candidate.unwrap_or_else(|| native.clone());
    for binary in [&native, &candidate] {
        if !binary.is_file() {
            return Err(format!("{}: no such binary", binary.display()));
        }
    }
    let split = |list: &str| {
        list.split(',')
            .map(str::trim)
            .filter(|it| !it.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (ledgers, modes) = (split(&ledgers), split(&modes));
    if let Some(mode) = modes.iter().find(|mode| !MODES.contains(&mode.as_str())) {
        return Err(format!("--modes: {mode} is not one of {}", MODES.join(", ")));
    }
    if ledgers.is_empty() || modes.is_empty() {
        return Err("--ledgers and --modes need at least one each".to_owned());
    }
    if most < pairs {
        return Err(format!("--most-pairs {most} is below --pairs {pairs}"));
    }
    let cpus = cpu_list(&cpus).ok_or_else(|| format!("--cpus {cpus}: expect a list such as 16-63 or 16-31,48-63"))?;
    let sets = cpus.chunks_exact(per_side).map(cpu_ranges).collect::<Vec<_>>();
    let slots = sets.as_chunks::<2>().0.to_vec();
    if slots.is_empty() {
        return Err(format!("{} CPUs make no slot of two sets of {per_side}", cpus.len()));
    }
    let compare = match compare {
        Some(compare) => compare,
        None => std::env::current_exe()
            .map_err(|error| format!("the runner's path: {error}"))?
            .with_file_name("budget_compare"),
    };
    if !compare.is_file() {
        return Err(format!("{}: no budget_compare (build the examples, or give --compare)", compare.display()));
    }
    Ok(Config {
        out,
        sides: [
            Side {
                name: "native",
                binary: native,
                env: env.iter().cloned().chain(native_env).collect(),
            },
            Side {
                name: "candidate",
                binary: candidate,
                env: env.iter().cloned().chain(candidate_env).collect(),
            },
        ],
        ledgers,
        modes,
        pairs,
        most,
        slots,
        per_side,
        requests_pairs,
        rounds,
        max_invalid,
        compare,
        compare_args,
    })
}

/// the CPUs of a list such as `16-63` or `16-31,48-63`, in order
fn cpu_list(list: &str) -> Option<Vec<usize>> {
    let mut cpus = vec![];
    for part in list.split(',').map(str::trim).filter(|it| !it.is_empty()) {
        let (low, high) = part.split_once('-').unwrap_or((part, part));
        let (low, high) = (low.parse::<usize>().ok()?, high.parse::<usize>().ok()?);
        if high < low {
            return None;
        }
        cpus.extend(low..=high);
    }
    (!cpus.is_empty()).then_some(cpus)
}

/// CPUs as a `taskset -c` list: `16-23`, or `16-19,24-27`
fn cpu_ranges(cpus: &[usize]) -> String {
    let mut ranges: Vec<(usize, usize)> = vec![];
    for &cpu in cpus {
        match ranges.last_mut() {
            Some((_, high)) if *high + 1 == cpu => *high = cpu,
            _ => ranges.push((cpu, cpu)),
        }
    }
    let range = |(low, high): &(usize, usize)| if low == high { low.to_string() } else { format!("{low}-{high}") };
    ranges.iter().map(range).collect::<Vec<_>>().join(",")
}

fn now() -> String {
    chrono::Utc::now().format("%H:%M:%S").to_string()
}

/// The pairs of every mode and ledger, the judgement, the top-ups, the judgement again: budget_compare's status.
fn run(config: &Config) -> Result<ExitCode, String> {
    let started = Instant::now();
    std::fs::create_dir_all(&config.out).map_err(|error| format!("{}: {error}", config.out.display()))?;
    let mut groups = config
        .modes
        .iter()
        .flat_map(|mode| {
            config.ledgers.iter().map(move |ledger| Group {
                mode: mode.clone(),
                ledger: ledger.clone(),
            })
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(Group::cost);
    eprintln!(
        "[{}] budget_pairs: {} slots of 2 x {} CPUs ({}); valid pairs {}",
        now(),
        config.slots.len(),
        config.per_side,
        config.slots.iter().map(|it| it.join(" | ")).collect::<Vec<_>>().join(", "),
        groups
            .iter()
            .map(|it| format!("{} {} {} (at most {})", it.mode, it.ledger, config.target(it), config.most(it)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // the pairs already there, from an earlier run into the same directory
    let mut valid = BTreeMap::new();
    let mut invalid = BTreeMap::<Group, usize>::new();
    for group in &groups {
        valid.insert(group.clone(), resume(config, group)?);
    }
    let mut wanted = groups
        .iter()
        .map(|group| (group.clone(), config.target(group).saturating_sub(valid[group])))
        .collect::<Vec<_>>();
    let status = loop {
        let mut queue = VecDeque::new();
        for (group, count) in &wanted {
            queue.extend(std::iter::repeat_n(group.clone(), *count));
        }
        schedule(config, queue, &mut valid, &mut invalid)?;
        let (status, verdicts) = judge(config)?;
        // top-ups: the modes and ledgers with an inconclusive verdict that more pairs would decide
        wanted = vec![];
        for group in &groups {
            let needs = verdicts
                .iter()
                .filter(|it| it["ledger"] == group.ledger.as_str() && it["verdict"] == "INCONCLUSIVE")
                .filter(|it| reads(&group.mode, it["section"].as_str().unwrap_or_default()))
                .filter_map(|it| it["needs"].as_u64())
                .max()
                .unwrap_or(0) as usize;
            let room = config.most(group).saturating_sub(valid[group]);
            if needs > 0 && room > 0 {
                let more = needs.max(config.slots.len()).min(room);
                eprintln!("[{}] {} {}: inconclusive, {more} more pairs", now(), group.mode, group.ledger);
                wanted.push((group.clone(), more));
            }
        }
        if wanted.is_empty() {
            break status;
        }
    };
    let elapsed = started.elapsed().as_secs();
    eprintln!(
        "[{}] budget_pairs: done in {}:{:02}:{:02}; valid pairs {}; pairs run again {}",
        now(),
        elapsed / 3600,
        elapsed / 60 % 60,
        elapsed % 60,
        groups
            .iter()
            .map(|it| format!("{} {} {}", it.mode, it.ledger, valid[it]))
            .collect::<Vec<_>>()
            .join(", "),
        groups
            .iter()
            .map(|it| format!("{} {} {}", it.mode, it.ledger, invalid.get(it).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(status)
}

/// whether the reports of `mode` hold the `section` a verdict read
fn reads(mode: &str, section: &str) -> bool {
    matches!((mode, section), ("full", _) | ("e2e", "end_to_end") | ("requests", "requests"))
}

/// The pairs of `group` already in the output directory: the valid ones stay, the others (invalid, or with a run
/// missing, from an interrupted run) are moved to `<out>/invalid/`. The number of valid pairs.
fn resume(config: &Config, group: &Group) -> Result<usize, String> {
    let mut pairs = std::collections::BTreeSet::new();
    for side in &config.sides {
        if let Ok(entries) = std::fs::read_dir(group.dir(&config.out, side.name)) {
            pairs.extend(entries.filter_map(Result::ok).filter_map(|it| it.file_name().to_str()?.parse::<usize>().ok()));
        }
    }
    let mut count = 0;
    for pair in pairs {
        match check_pair(config, group, pair) {
            Ok(()) => count += 1,
            Err(why) => {
                eprintln!("[{}] {} {} pair {pair:02} found invalid ({why}): moved aside", now(), group.mode, group.ledger);
                move_aside(config, group, pair)?;
            }
        }
    }
    if count > 0 {
        eprintln!("[{}] {} {}: {count} valid pairs already there", now(), group.mode, group.ledger);
    }
    Ok(count)
}

/// the next free pair number of `group`, past those in the layout and those moved aside
fn next_pair(config: &Config, group: &Group) -> usize {
    let mut dirs = config.sides.iter().map(|side| group.dir(&config.out, side.name)).collect::<Vec<_>>();
    dirs.push(group.dir(&config.out, "invalid"));
    dirs.iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|it| it.file_name().to_str()?.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

/// Run the pairs of `queue`, as many at once as there are slots, until each is done and valid; a pair found invalid
/// is moved aside and queued again, up to `--max-invalid` times a mode and ledger.
fn schedule(config: &Config, mut queue: VecDeque<Group>, valid: &mut BTreeMap<Group, usize>, invalid: &mut BTreeMap<Group, usize>) -> Result<(), String> {
    let mut slots: Vec<Option<Running>> = config.slots.iter().map(|_| None).collect();
    let mut failure = None;
    loop {
        if failure.is_none() {
            for (index, slot) in slots.iter_mut().enumerate() {
                if slot.is_none() {
                    if let Some(group) = queue.pop_front() {
                        *slot = Some(start(config, index, group)?);
                    }
                }
            }
        }
        if slots.iter().all(Option::is_none) {
            break;
        }
        std::thread::sleep(POLL);
        for slot in slots.iter_mut() {
            let Some(running) = slot else { continue };
            let mut done = true;
            for (child, _, _) in running.runs.iter_mut() {
                done &= child.try_wait().map_err(|error| format!("a run: {error}"))?.is_some();
            }
            if !done {
                continue;
            }
            let running = slot.take().expect("a pair");
            let group = running.group.clone();
            if finish(config, running)? {
                *valid.get_mut(&group).expect("a group") += 1;
            } else {
                let count = invalid.entry(group.clone()).or_default();
                *count += 1;
                let most = config.max_invalid.unwrap_or_else(|| config.target(&group));
                if *count > most {
                    failure.get_or_insert(format!(
                        "{} {}: {} invalid pairs, more than --max-invalid {}; see {}",
                        group.mode,
                        group.ledger,
                        count,
                        most,
                        config.out.join("runs.jsonl").display()
                    ));
                } else {
                    queue.push_front(group);
                }
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Start the two runs of the next pair of `group` in slot `slot`, at the same time, each on a set of the slot's CPUs;
/// the sides swap the sets from pair to pair.
fn start(config: &Config, slot: usize, group: Group) -> Result<Running, String> {
    let pair = next_pair(config, &group);
    let sets = &config.slots[slot];
    let mut runs = vec![];
    for (index, side) in config.sides.iter().enumerate() {
        let cpus = sets[(index + pair) % 2].clone();
        let dir = group.dir(&config.out, side.name).join(format!("{pair:02}"));
        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let file = |name: &str| File::create(dir.join(name)).map_err(|error| format!("{}: {error}", dir.join(name).display()));
        let mut command = Command::new(&side.binary);
        command
            .arg(&group.ledger)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(file("stdout.txt")?)
            .stderr(file("stderr.txt")?)
            // pinned by the bench itself, on the CPUs of its side
            .env_remove("ZHANG_BUDGET_BASELINE_PINNED")
            .env("ZHANG_BUDGET_BASELINE_ONLY", if group.mode == "full" { "" } else { group.mode.as_str() })
            .env("ZHANG_BUDGET_BASELINE_OUT", &dir)
            .env("ZHANG_BUDGET_BASELINE_CPUS", &cpus)
            .env("ZHANG_BUDGET_BASELINE_WORKERS", config.per_side.to_string());
        if group.mode == "requests" {
            command.env("ZHANG_BUDGET_BASELINE_ROUNDS", config.rounds.to_string());
        }
        command.envs(side.env.iter().map(|(name, value)| (name, value)));
        let child = command.spawn().map_err(|error| format!("{}: {error}", side.binary.display()))?;
        runs.push((child, dir, cpus));
    }
    eprintln!(
        "[{}] {} {} pair {pair:02}: started, {}",
        now(),
        group.mode,
        group.ledger,
        config
            .sides
            .iter()
            .zip(&runs)
            .map(|(side, (_, _, cpus))| format!("{} on {cpus}", side.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(Running {
        group,
        pair,
        runs,
        started: Instant::now(),
        started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })
}

/// The pair is done: whether it is valid; an invalid one is moved aside. Either way it is recorded in `runs.jsonl`.
fn finish(config: &Config, mut running: Running) -> Result<bool, String> {
    let wall = running.started.elapsed().as_secs_f64();
    let mut sides = vec![];
    for (side, (child, dir, cpus)) in config.sides.iter().zip(running.runs.iter_mut()) {
        let status = child.wait().map_err(|error| format!("a run: {error}"))?;
        let check = check_run(&running.group, &dir.join(format!("{}.json", running.group.ledger)), status.success());
        sides.push(json!({
            "side": side.name,
            "binary": side.binary,
            "env": side.env.iter().map(|(name, value)| format!("{name}={value}")).collect::<Vec<_>>(),
            "cpus": cpus,
            "exit": status.code(),
            "valid": check.is_ok(),
            "reasons": check.err(),
        }));
    }
    let valid = sides.iter().all(|it| it["valid"] == true);
    let reasons = sides
        .iter()
        .filter_map(|it| Some(format!("{}: {}", it["side"].as_str()?, it["reasons"].as_str()?)))
        .collect::<Vec<_>>();
    eprintln!(
        "[{}] {} {} pair {:02}: {} in {wall:.0} s{}",
        now(),
        running.group.mode,
        running.group.ledger,
        running.pair,
        if valid { "valid" } else { "INVALID, run again" },
        if reasons.is_empty() {
            String::new()
        } else {
            format!(" ({})", reasons.join("; "))
        }
    );
    let record = json!({
        "mode": running.group.mode,
        "ledger": running.group.ledger,
        "pair": running.pair,
        "started": running.started_at,
        "wall_s": (wall * 10.0).round() / 10.0,
        "valid": valid,
        "sides": sides,
    });
    let path = config.out.join("runs.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    writeln!(file, "{record}").map_err(|error| format!("{}: {error}", path.display()))?;
    if !valid {
        move_aside(config, &running.group, running.pair)?;
    }
    Ok(valid)
}

/// whether both runs of a pair in the layout are there and valid
fn check_pair(config: &Config, group: &Group, pair: usize) -> Result<(), String> {
    let reasons = config
        .sides
        .iter()
        .filter_map(|side| {
            let path = group
                .dir(&config.out, side.name)
                .join(format!("{pair:02}"))
                .join(format!("{}.json", group.ledger));
            check_run(group, &path, true).err().map(|why| format!("{}: {why}", side.name))
        })
        .collect::<Vec<_>>();
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(reasons.join("; "))
    }
}

/// Whether a run's report says it is valid for `group`: the run ended well, wrote its report, in the mode asked, found
/// its conditions good (no build, no throttling, little CPU pressure), and each end-to-end timer kept its clean rounds.
fn check_run(group: &Group, report: &Path, exited_well: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(report).map_err(|_| format!("no report{}", if exited_well { "" } else { ", the run failed" }))?;
    let report: Value = serde_json::from_str(&text).map_err(|error| format!("the report: {error}"))?;
    if !exited_well {
        return Err("the run failed".to_owned());
    }
    let mode = group.mode.as_str();
    if report["mode"] != mode {
        return Err(format!("a report of the mode {}, not {mode}", report["mode"]));
    }
    if report["valid"] != true {
        let reasons = report["validity"]["reasons"]
            .as_array()
            .map(|it| it.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "));
        return Err(reasons.unwrap_or_else(|| "marked invalid".to_owned()));
    }
    let sections: &[&str] = match mode {
        "e2e" => &["end_to_end"],
        "requests" => &["requests"],
        _ => &["end_to_end", "requests"],
    };
    for section in sections {
        if !report[*section].is_object() {
            return Err(format!("no {section} in the report: a ledger without budgets?"));
        }
    }
    if report["end_to_end"].is_object() {
        let short = TIMERS
            .iter()
            .filter(|timer| report["end_to_end"][**timer]["valid"] != true)
            .map(|timer| format!("{timer} kept {} clean rounds", report["end_to_end"][*timer]["clean_rounds"]))
            .collect::<Vec<_>>();
        if !short.is_empty() {
            return Err(short.join(", "));
        }
    }
    Ok(())
}

/// Move both runs of a pair out of the layout budget_compare reads, to `<out>/invalid/<mode>/<ledger>/<pair>/<side>`.
fn move_aside(config: &Config, group: &Group, pair: usize) -> Result<(), String> {
    let aside = group.dir(&config.out, "invalid").join(format!("{pair:02}"));
    for side in &config.sides {
        let from = group.dir(&config.out, side.name).join(format!("{pair:02}"));
        if from.exists() {
            std::fs::create_dir_all(&aside).map_err(|error| format!("{}: {error}", aside.display()))?;
            let to = aside.join(side.name);
            let _ = std::fs::remove_dir_all(&to);
            std::fs::rename(&from, &to).map_err(|error| format!("{} to {}: {error}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// budget_compare on the layout: its output, printed and kept in `<out>/compare.txt`, its status, and its verdicts.
fn judge(config: &Config) -> Result<(ExitCode, Vec<Value>), String> {
    let summary = config.out.join("summary.json");
    let _ = std::fs::remove_file(&summary);
    let output = Command::new(&config.compare)
        .arg(config.out.join(config.sides[0].name))
        .arg(config.out.join(config.sides[1].name))
        .arg("--summary")
        .arg(&summary)
        .args(&config.compare_args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{}: {error}", config.compare.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    print!("{text}");
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    let path = config.out.join("compare.txt");
    std::fs::write(&path, text.as_bytes()).map_err(|error| format!("{}: {error}", path.display()))?;
    let verdicts = std::fs::read_to_string(&summary)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|summary| summary["verdicts"].as_array().cloned())
        .ok_or_else(|| format!("budget_compare wrote no summary ({})", output.status))?;
    Ok((if output.status.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE }, verdicts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpus_are_cut_into_sets() {
        let cpus = cpu_list("16-63").expect("a list");
        let sets = cpus.chunks_exact(8).map(cpu_ranges).collect::<Vec<_>>();
        assert_eq!(sets, ["16-23", "24-31", "32-39", "40-47", "48-55", "56-63"]);
        assert_eq!(cpu_ranges(&cpu_list("0,2,4-6").expect("a list")), "0,2,4-6");
        assert_eq!(cpu_list("9-8"), None);
    }

    #[test]
    fn the_longest_pairs_start_first() {
        let group = |mode: &str, ledger: &str| Group {
            mode: mode.to_owned(),
            ledger: ledger.to_owned(),
        };
        let mut groups = vec![
            group("requests", "10000tx-30b-60m"),
            group("e2e", "10000tx-30b-60m"),
            group("requests", "100000tx-30b-60m"),
            group("e2e", "100000tx-30b-60m"),
        ];
        groups.sort_by_key(Group::cost);
        let order = groups.iter().map(|it| format!("{} {}", it.mode, it.ledger)).collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                "e2e 100000tx-30b-60m",
                "e2e 10000tx-30b-60m",
                "requests 100000tx-30b-60m",
                "requests 10000tx-30b-60m"
            ]
        );
    }
}
