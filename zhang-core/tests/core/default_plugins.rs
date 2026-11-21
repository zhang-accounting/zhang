//! The official default plugin `zhang:budget` under the ledger's options and the operator's switch (RFC #698 §6): every
//! ledger has it, without a `plugin` directive and whatever its `features.plugin` says, so no option of a ledger turns
//! budgets off; `ZHANG_PLUGINS=off` stops it as it stops every plugin, and the ledger then says once that its budgets
//! are not interpreted. With the plugin runtime and without it, as in the playground's build: the native provider of
//! the budget tables and checks runs in both.

use std::path::Path;
use std::sync::Arc;

use zhang_ast::error::ErrorKind;
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::{PluginLevel, PluginPolicy};

/// A ledger with a budget, its activity, and a `budget-add` of a budget never defined. Line 2 is the first directive
/// that uses budgets.
const LEDGER: &str = "1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: \"Food\"

2024-01-01 budget Food CNY
2024-01-01 budget-add Food 100 CNY
2024-01-05 * \"Cafe\" \"lunch\"
  Expenses:Food 30 CNY
  Assets:Bank -30 CNY
2024-01-06 budget-add Nope 1 CNY
";

/// What a ledger may say about plugins before its budgets: `features.plugin` absent, off or on, and a `plugin` directive
/// naming `zhang:budget` or none.
fn headers() -> Vec<String> {
    let mut headers = vec![];
    for features in ["", "option \"features.plugin\" \"false\"\n", "option \"features.plugin\" \"true\"\n"] {
        for directive in ["", "plugin \"zhang:budget\"\n"] {
            headers.push(format!("{features}{directive}"));
        }
    }
    headers
}

fn load(dir: &Path, content: &str, level: PluginLevel) -> Ledger {
    std::fs::write(dir.join("main.zhang"), content).unwrap();
    let policy = Arc::new(PluginPolicy {
        level,
        ..PluginPolicy::default()
    });
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    Ledger::load_with_policy(dir.to_path_buf(), "main.zhang".to_owned(), source, Clock::System, policy).unwrap_or_else(|e| panic!("{content}: {e}"))
}

/// Each error as its kind and the line it is on, with the line of the main file's first line of budgets (after the
/// header) counted from 1.
fn errors(ledger: &Ledger, header: &str) -> Vec<(ErrorKind, usize)> {
    let offset = header.lines().count();
    let mut errors = ledger
        .errors
        .iter()
        .map(|error| (error.error_type.clone(), error.span.as_ref().and_then(|span| span.line).unwrap_or(0) - offset))
        .collect::<Vec<_>>();
    errors.sort_by_key(|(_, line)| *line);
    errors
}

#[test]
fn every_ledger_interprets_its_budgets_whatever_its_options_say() {
    let dir = tempfile::tempdir().unwrap();
    for header in headers() {
        for level in [PluginLevel::Read, PluginLevel::Write] {
            let ledger = load(dir.path(), &format!("{header}{LEDGER}"), level);
            assert_eq!(errors(&ledger, &header), [(ErrorKind::BudgetDoesNotExist, 10)], "{level}: {header}");
            assert_eq!(ledger.extensions.unavailable("zhang:budget"), None);
        }
    }
}

#[test]
fn plugins_off_stop_zhang_budget_and_the_ledger_says_so_once() {
    let dir = tempfile::tempdir().unwrap();
    for header in headers() {
        let ledger = load(dir.path(), &format!("{header}{LEDGER}"), PluginLevel::Off);
        let reason = "plugins are off (ZHANG_PLUGINS=off)";
        assert_eq!(ledger.extensions.unavailable("zhang:budget"), Some(reason));
        // no budget error: the budget directives are not interpreted, and one error says so, on the first of them
        let mut expected = vec![(ErrorKind::ExtensionUnavailable, 2)];
        // a directive naming zhang:budget loads nothing, whatever `features.plugin` says
        if header.contains("plugin \"zhang:budget\"") {
            expected.insert(0, (ErrorKind::PluginsDisabled, 0));
        }
        assert_eq!(errors(&ledger, &header), expected, "{header}");
        let unavailable = ledger.errors.iter().find(|it| it.error_type == ErrorKind::ExtensionUnavailable).unwrap();
        assert_eq!(unavailable.metas.len(), 2);
        assert_eq!(
            (unavailable.metas["plugin"].as_str(), unavailable.metas["reason"].as_str()),
            ("zhang:budget", reason)
        );
    }
}

#[test]
fn a_ledger_without_budgets_has_nothing_to_say_with_plugins_off() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = load(
        dir.path(),
        "1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n2024-01-05 * \"Cafe\"\n  Expenses:Food 30 CNY\n  Assets:Bank -30 CNY\n",
        PluginLevel::Off,
    );
    assert_eq!(ledger.errors.len(), 0, "{:?}", ledger.errors);
    // a budget named only in an open's metadata is a use of budgets
    let ledger = load(dir.path(), "1970-01-01 open Expenses:Food\n  budget: \"Food\"\n", PluginLevel::Off);
    assert_eq!(errors(&ledger, ""), [(ErrorKind::ExtensionUnavailable, 1)]);
}

/// A `plugin` directive naming a default plugin the embedder registers (as the server will register the bundled
/// `zhang:budget`) configures it: one plugin, registered first, whatever `features.plugin` says, with the directive's
/// metas, and reporting on the directive.
#[cfg(feature = "plugin_runtime")]
#[test]
fn a_directive_naming_a_default_plugin_configures_it() {
    use std::time::Duration;

    use zhang_core::analysis::{DefaultPlugin, LoadOptions};

    let dir = tempfile::tempdir().unwrap();
    let options = |level: PluginLevel| LoadOptions {
        plugin_policy: Arc::new(PluginPolicy {
            level,
            ..PluginPolicy::default()
        }),
        analysis: None,
        default_plugins: vec![DefaultPlugin {
            module: "zhang:configured".to_owned(),
            file: "router_now.wat".to_owned(),
            content: Arc::from(&include_bytes!("../plugins/router_now.wat")[..]),
        }],
    };
    let load = |content: &str, level: PluginLevel| {
        std::fs::write(dir.path().join("main.zhang"), content).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        Ledger::load_with_options(dir.path().to_path_buf(), "main.zhang".to_owned(), source, Clock::System, options(level)).unwrap()
    };
    for features in ["", "option \"features.plugin\" \"false\"\n", "option \"features.plugin\" \"true\"\n"] {
        let content = format!("{features}plugin \"zhang:configured\"\n  timeout: \"5s\"\n1970-01-01 open Assets:Cash\n");
        let ledger = load(&content, PluginLevel::Read);
        assert_eq!(ledger.plugins.ordered.len(), 1, "one plugin: {features}");
        let (plugin, _) = &ledger.plugins.ordered[0];
        assert!(plugin.is_default());
        assert_eq!(plugin.capabilities().timeout, Duration::from_secs(5), "the directive's timeout: {features}");
        assert_eq!(plugin.span().line, Some(features.lines().count() + 1), "the directive's span");
        assert!(ledger.errors.is_empty(), "{features}: {:?}", ledger.errors);
        // plugins off: nothing registers, and the directive says so whatever `features.plugin` says
        let ledger = load(&content, PluginLevel::Off);
        assert!(ledger.plugins.ordered.is_empty());
        let errors = ledger.errors.iter().map(|error| error.error_type.clone()).collect::<Vec<_>>();
        assert_eq!(errors, [ErrorKind::PluginsDisabled], "{features}");
    }
    // without a directive, it is registered all the same, with no span
    let ledger = load("1970-01-01 open Assets:Cash\n", PluginLevel::Read);
    assert_eq!(ledger.plugins.ordered.len(), 1);
    assert_eq!(ledger.plugins.ordered[0].0.span().line, None);
}
