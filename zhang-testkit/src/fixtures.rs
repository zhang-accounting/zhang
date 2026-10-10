//! The fixture ledgers of the repository and how to load them.
//!
//! `integration-tests/<case>/` holds one ledger per case as `main.zhang`, `main.bean` or both, with the
//! `validations.json` the end-to-end run checks; `examples/main.zhang` is the example ledger; the beancount oracle
//! ledgers (`extensions/beancount/tests/balance_assertions/*.bean`) are small `.bean` files with pads, balances and
//! document paths as beancount reads them. [`every_fixture_ledger`] and [`oracle_ledgers`] enumerate them once per
//! process and [`FixtureLedger::ledger`] loads each at most once, so a suite that walks every ledger shares the loads.
//!
//! A ledger is loaded in the format of its entry file ([`Dialect::of`]): the zhang parser for `.zhang`, the beancount
//! parser for `.bean`. The local data source expands wildcard includes (`include "data/*.zhang"`) like the server's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use beancount::Beancount;
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::Dialect;
use zhang_core::ledger::Ledger;
use zhang_core::ZhangResult;

/// The root of the repository (the workspace directory).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zhang-testkit sits in the workspace root")
        .canonicalize()
        .expect("the workspace root exists")
}

/// `integration-tests/`, the fixture corpus.
pub fn integration_tests_dir() -> PathBuf {
    repo_root().join("integration-tests")
}

/// `integration-tests/<name>/`. Panics, naming the directory, when the case does not exist.
pub fn fixture_dir(name: &str) -> PathBuf {
    let dir = integration_tests_dir().join(name);
    assert!(dir.is_dir(), "no fixture directory integration-tests/{name} ({})", dir.display());
    dir
}

/// `examples/`, the directory of the example ledger.
pub fn examples_dir() -> PathBuf {
    repo_root().join("examples")
}

/// The directory of the beancount oracle ledgers: `extensions/beancount/tests/balance_assertions/`, whose `*.bean`
/// files come with `oracle.json`, what beancount makes of them.
pub fn oracle_ledger_dir() -> PathBuf {
    repo_root().join("extensions/beancount/tests/balance_assertions")
}

/// A ledger of the repository: where it is, and its loaded form once something asked for it.
pub struct FixtureLedger {
    /// `<directory>/<entry>`, e.g. `fava-demo-ledger/main.zhang`, `examples/main.zhang`, `beancount-oracle/two_pads.bean`
    pub name: String,
    /// the directory of the ledger
    pub dir: PathBuf,
    /// the entry file within `dir`
    pub entry: String,
    /// the format of the entry file
    pub dialect: Dialect,
    loaded: OnceLock<Arc<Ledger>>,
}

impl std::fmt::Debug for FixtureLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixtureLedger")
            .field("name", &self.name)
            .field("dir", &self.dir)
            .field("entry", &self.entry)
            .finish()
    }
}

impl FixtureLedger {
    /// A ledger at `dir/entry`, named `name`. The format comes from the entry's extension.
    pub fn new(name: impl Into<String>, dir: impl Into<PathBuf>, entry: impl Into<String>) -> FixtureLedger {
        let entry = entry.into();
        let dialect = Dialect::of(&entry).unwrap_or_else(|error| panic!("{entry}: {error}"));
        FixtureLedger {
            name: name.into(),
            dir: dir.into(),
            entry,
            dialect,
            loaded: OnceLock::new(),
        }
    }

    /// the path of the entry file
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.entry)
    }

    /// The ledger, loaded anew. A test that changes the ledger, or needs its own copy, loads here (or copies the
    /// directory with [`crate::ledger::Scratch::copy_of`]).
    pub fn load(&self) -> ZhangResult<Ledger> {
        load_dir(self.dir.clone(), &self.entry)
    }

    /// The ledger, loaded once per process and shared. Panics with the fixture's name when it does not load.
    pub fn ledger(&self) -> Arc<Ledger> {
        self.loaded
            .get_or_init(|| Arc::new(self.load().unwrap_or_else(|error| panic!("{}: cannot load the ledger: {error}", self.name))))
            .clone()
    }
}

/// The data source of a local ledger whose entry file is `entry`: the parser of its format over the local disk.
pub fn data_source_for(entry: &str) -> Arc<dyn DataSource> {
    match Dialect::of(entry).unwrap_or_else(|error| panic!("{entry}: {error}")) {
        Dialect::Zhang => Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
        Dialect::Beancount => Arc::new(LocalFileSystemDataSource::new(Beancount::default())),
    }
}

/// Load the ledger at `dir/entry` in the format of `entry`.
pub fn load_dir(dir: impl Into<PathBuf>, entry: &str) -> ZhangResult<Ledger> {
    Ledger::load(dir.into(), entry.to_owned(), data_source_for(entry))
}

/// Every ledger of `integration-tests/` (each case in each format it has, in name order) and `examples/main.zhang`.
/// Enumerated once; each ledger loads at most once through [`FixtureLedger::ledger`].
pub fn every_fixture_ledger() -> &'static [FixtureLedger] {
    static ALL: OnceLock<Vec<FixtureLedger>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut dirs = std::fs::read_dir(integration_tests_dir())
            .expect("integration-tests/ is readable")
            .map(|it| it.expect("a directory entry").path())
            .filter(|it| it.is_dir())
            .collect::<Vec<_>>();
        dirs.sort();
        dirs.push(examples_dir());
        let mut ledgers = vec![];
        for dir in dirs {
            let case = dir.file_name().expect("a named directory").to_string_lossy().into_owned();
            for entry in ["main.zhang", "main.bean"] {
                if dir.join(entry).is_file() {
                    ledgers.push(FixtureLedger::new(format!("{case}/{entry}"), dir.clone(), entry));
                }
            }
        }
        ledgers
    })
}

/// The fixture ledger `integration-tests/<case>/<entry>` (or `examples/main.zhang`). Panics, naming it, when there
/// is no such ledger.
pub fn fixture_ledger(case: &str, entry: &str) -> &'static FixtureLedger {
    let name = format!("{case}/{entry}");
    every_fixture_ledger()
        .iter()
        .find(|it| it.name == name)
        .unwrap_or_else(|| panic!("no fixture ledger {name}"))
}

/// The beancount oracle ledgers, in name order, named `beancount-oracle/<file>`.
pub fn oracle_ledgers() -> &'static [FixtureLedger] {
    static ALL: OnceLock<Vec<FixtureLedger>> = OnceLock::new();
    ALL.get_or_init(|| {
        let dir = oracle_ledger_dir();
        let mut files = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
            .map(|it| it.expect("a directory entry").path())
            .filter(|it| it.extension().is_some_and(|extension| extension == "bean"))
            .collect::<Vec<_>>();
        files.sort();
        files
            .into_iter()
            .map(|file| {
                let entry = file.file_name().expect("a file name").to_string_lossy().into_owned();
                FixtureLedger::new(format!("beancount-oracle/{entry}"), dir.clone(), entry)
            })
            .collect()
    })
}

/// The fava demo ledger (`integration-tests/fava-demo-ledger/main.zhang`, 2015-2017, the beanquery oracle ledger),
/// loaded once per process. [`crate::ledger::fava_demo_ledger`] loads a fresh copy instead.
pub fn fava_demo() -> Arc<Ledger> {
    fixture_ledger("fava-demo-ledger", "main.zhang").ledger()
}
