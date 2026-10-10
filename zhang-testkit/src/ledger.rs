//! Ledgers built for a test: from text, in a scratch directory, or the fava demo ledger loaded anew.
//!
//! These are the helpers `zhang-query/tests/common/mod.rs` used to hold, with the same behaviour, plus
//! [`Scratch`], a directory the test owns, and [`XorShift`], the reproducible random source of the property tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;
use zhang_ast::{Directive, Spanned};
use zhang_core::clock::Clock;
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::{DataType, Dialect};
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::ZhangResult;

use crate::fixtures::{self, FixtureLedger};

/// Load the ledger at `dir/entry` **with the zhang parser**, whatever the entry's extension; the ledger's dialect
/// still follows the entry name (a `main.bean` entry gives a beancount-dialect ledger parsed as zhang text, which is
/// what some document-path tests rely on). For a ledger parsed in the format of its entry use
/// [`fixtures::load_dir`]. Panics when the ledger does not load.
pub fn load_ledger(dir: PathBuf, entry: &str) -> Ledger {
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir, entry.to_owned(), Arc::new(source)).expect("cannot load ledger")
}

/// Load a ledger from zhang text written as `main.zhang` to a temporary directory. The directory is kept for the
/// life of the process (the ledger's data source reads from it); use [`Scratch`] for a directory that is removed.
pub fn load_text(content: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    load_ledger(dir, "main.zhang")
}

/// Load a ledger from zhang text, with the current time read from `clock` (what a plugin calling `zhang_now`
/// gets). The directory is kept like [`load_text`]'s.
pub fn load_text_at(content: &str, clock: Clock) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    let directives = ZhangDataType {}
        .transform(content.to_owned(), Some("main.zhang".to_owned()))
        .expect("parse ledger");
    Ledger::process(LedgerProcessContext {
        directives,
        entry: (dir, "main.zhang".to_owned()),
        dialect: Dialect::Zhang,
        visited_files: vec![],
        data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
        clock,
    })
    .expect("cannot load ledger")
}

/// Load a ledger from zhang text, with `transform` changing its parsed directives first, as a plugin changes the
/// stream it reads. The directory is kept like [`load_text`]'s.
pub fn load_transformed(content: &str, transform: impl FnOnce(Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>>) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    let source: Arc<dyn DataSource> = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let loaded = source
        .load(dir.to_string_lossy().into_owned(), "main.zhang".to_owned())
        .expect("cannot read ledger");
    Ledger::process(LedgerProcessContext {
        directives: transform(loaded.directives),
        entry: (dir, "main.zhang".to_owned()),
        dialect: Dialect::Zhang,
        visited_files: loaded.visited_files,
        data_source: source,
        clock: Clock::System,
    })
    .expect("cannot process ledger")
}

/// The fava demo ledger, loaded anew. [`fixtures::fava_demo`] shares one load per process instead.
pub fn fava_demo_ledger() -> Ledger {
    load_ledger(fixtures::fixture_dir("fava-demo-ledger"), "main.zhang")
}

/// A directory the test owns, with a main file, removed when dropped. The ledger is read in the format of the main
/// file's extension.
pub struct Scratch {
    dir: TempDir,
    main: String,
}

impl Scratch {
    /// A directory with `main.zhang` holding `content`.
    pub fn zhang(content: &str) -> Scratch {
        Scratch::with_main("main.zhang", content)
    }

    /// A directory with `main.bean` holding `content`.
    pub fn beancount(content: &str) -> Scratch {
        Scratch::with_main("main.bean", content)
    }

    /// A directory with the main file `main` holding `content`.
    pub fn with_main(main: &str, content: &str) -> Scratch {
        let dir = tempfile::Builder::new().prefix("zhang-test-").tempdir().expect("tempdir");
        std::fs::write(dir.path().join(main), content).expect("write the main file");
        Scratch { dir, main: main.to_owned() }
    }

    /// A copy of a fixture ledger's directory, for a test that writes to it.
    pub fn copy_of(fixture: &FixtureLedger) -> Scratch {
        let dir = tempfile::Builder::new().prefix("zhang-test-").tempdir().expect("tempdir");
        copy_dir(&fixture.dir, dir.path());
        Scratch {
            dir,
            main: fixture.entry.clone(),
        }
    }

    /// the directory
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// the name of the main file within the directory
    pub fn main(&self) -> &str {
        &self.main
    }

    /// the path of the main file
    pub fn main_file(&self) -> PathBuf {
        self.dir.path().join(&self.main)
    }

    /// Write `content` to `relative`, creating its parent directories.
    pub fn write(&self, relative: &str, content: &str) {
        let path = self.dir.path().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent directories");
        }
        std::fs::write(&path, content).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }

    /// The text of `relative`.
    pub fn read(&self, relative: &str) -> String {
        let path = self.dir.path().join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// The ledger loaded from the files as they are now, in the format of the main file.
    pub fn ledger(&self) -> ZhangResult<Ledger> {
        fixtures::load_dir(self.dir.path().to_path_buf(), &self.main)
    }

    /// Keep the directory after the test (to look at what was written) and return its path.
    pub fn keep(self) -> PathBuf {
        self.dir.keep()
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap_or_else(|error| panic!("{}: {error}", to.display()));
    for entry in std::fs::read_dir(from).unwrap_or_else(|error| panic!("{}: {error}", from.display())) {
        let entry = entry.expect("a directory entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap_or_else(|error| panic!("{}: {error}", target.display()));
        }
    }
}

/// A small xorshift PRNG, so the property tests need no extra dependency and are reproducible.
pub struct XorShift(pub u64);

impl XorShift {
    /// the next value; named before the type moved here, and no iterator (an iterator would never end)
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    /// true `percent` times in a hundred
    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// A random string drawn mostly from the characters that need care in a quoted string.
    pub fn string(&mut self) -> String {
        const INTERESTING: &[char] = &[
            '"', '\\', '$', '`', '\'', '/', 'u', '{', '}', 'd', 'n', ' ', '\n', '\r', '\t', '\u{0}', '\u{07}', '\u{08}', '\u{1b}', '\u{7f}', '\u{85}',
            '\u{a0}', '\u{200d}', '\u{2028}', '\u{2029}', '\u{3000}', '\u{feff}', '😀', '你', '好', 'é', 'a', ';', '#', ':', '*', '^', '@', '!',
        ];
        let len = self.below(16);
        (0..len)
            .map(|_| {
                if self.below(6) == 0 {
                    loop {
                        if let Some(c) = char::from_u32((self.next() % 0x11_0000) as u32) {
                            break c;
                        }
                    }
                } else {
                    INTERESTING[self.below(INTERESTING.len())]
                }
            })
            .collect()
    }
}
