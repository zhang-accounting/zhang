//! Lua analyzers as they register (RFC #698 P1.6): the type `Analyzer`, alone or with `Router`, and the manifest made
//! of fields of the module table. What they analyse and publish is tested through the server's analysis host
//! (`zhang-server/tests/api/lua_analyzers.rs`).

use zhang_shared::plugin_abi::analysis as abi;

use super::*;

const ANALYZER: &[u8] = br#"
-- what the script replaces at its top level, before its manifest is written, changes its own calls only
string.format, table.concat = function() return "F" end, function() return "C" end
json.object, json.array = function() return {} end, function() return {} end
local M = {
  name = "shop", version = "0.1.0", types = { "Analyzer", "Router" },
  analysis_abi = 1,
  namespace = "shop",
  consumes = { "custom:shop" },
  open_meta = {},
  diagnostics = { { kind = "UnknownItem" }, { kind = "Unpriced", phase = "after_fold" } },
  tables = {
    { name = "items", kind = "records", columns = { { name = "item", type = "str" } }, order = { "item" } },
    { name = "stock", kind = "series", partition = {},
      period = { column = "date", unit = "month", derive = {} },
      columns = { { name = "date", type = "date" }, { name = "sold", type = "decimal", fill = "zero" },
                  { name = "left", type = "decimal", fill = { previous = "sold" } } } },
  },
  columns = { { table = "postings", name = "items", predicate = "TRUE", params = {} } },
  -- not part of the manifest: a module table holds its functions and its own fields too
  helper = function() end,
  router = function() return { json = zhang.api_version } end,
}
function M.analyze() return {} end
return M
"#;

#[test]
fn a_lua_analyzer_declares_its_manifest_in_its_module_table() {
    let (_dir, ledger) = script_ledger("shop.lua", ANALYZER, "");
    let ledger = ledger.unwrap();
    assert_eq!(registered(&ledger), vec![("shop".to_owned(), vec![PluginType::Analyzer, PluginType::Router])]);
    let analyzers = ledger.plugins.analyzers().collect::<Vec<_>>();
    assert_eq!(analyzers.len(), 1);
    // the empty maps of the manifest are objects, where Lua would write `{}` as `[]`, and the empty lists lists, even
    // though the script replaced json.object in its own json table
    let expected: abi::Manifest = serde_json::from_value(json!({
        "analysis_abi": 1,
        "namespace": "shop",
        "consumes": ["custom:shop"],
        "open_meta": [],
        "diagnostics": [{"kind": "UnknownItem"}, {"kind": "Unpriced", "phase": "after_fold"}],
        "tables": [
            {"name": "items", "kind": "records", "columns": [{"name": "item", "type": "str"}], "order": ["item"]},
            {"name": "stock", "kind": "series", "partition": [], "period": {"column": "date", "unit": "month", "derive": {}},
             "columns": [{"name": "date", "type": "date"}, {"name": "sold", "type": "decimal", "fill": "zero"},
                         {"name": "left", "type": "decimal", "fill": {"previous": "sold"}}]}
        ],
        "columns": [{"table": "postings", "name": "items", "predicate": "TRUE", "params": {}}]
    }))
    .unwrap();
    assert_eq!(analyzers[0].analyzer(), Some(&Ok(expected)));
    // the same script serves its route
    assert_eq!(json_body(&call(&ledger, "shop", &get("/")).unwrap()), json!(1));
    // a plain zhang-core load has no analysis host: the analysis fails with one error, and the ledger loads
    let failed = errors_of(&ledger, ErrorKind::PluginAnalysisFailed);
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["reason"], "unavailable");
}

#[test]
fn a_lua_analyzer_whose_manifest_does_not_hold_still_loads() {
    // a field of the wrong shape: the manifest is refused when the ledger is analysed, and the ledger loads
    let (_dir, ledger) = script_ledger(
        "bad.lua",
        br#"return { name = "bad", version = "0.1.0", types = { "Analyzer" }, analysis_abi = 1, tables = { { name = "t", kind = "cube", columns = {} } } }"#,
        "",
    );
    let ledger = ledger.unwrap();
    let plugin = ledger.plugins.analyzers().next().unwrap();
    let Some(Err(message)) = plugin.analyzer() else {
        panic!("a manifest that does not hold: {:?}", plugin.analyzer())
    };
    assert!(message.starts_with("its manifest is not one: unknown variant `cube`"), "{message}");
    assert_eq!(errors_of(&ledger, ErrorKind::PluginAnalysisFailed).len(), 1);

    // a value JSON cannot write fails the manifest too, naming the script
    let (_dir, ledger) = script_ledger(
        "cycle.lua",
        b"local t = {}\nt.self = t\nreturn { name = \"cycle\", version = \"0.1.0\", types = { \"Analyzer\" }, tables = t }",
        "",
    );
    let ledger = ledger.unwrap();
    let Some(Err(message)) = ledger.plugins.analyzers().next().unwrap().analyzer() else {
        panic!("no manifest")
    };
    assert!(message.contains("cycle.lua: the analyzer's manifest cannot be written as JSON"), "{message}");
}
