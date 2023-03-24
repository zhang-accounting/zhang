use crate::create_folder_if_not_exist;
use itertools::Itertools;
use std::fs::OpenOptions;
use std::io::Write;
use text_exporter::TextExporter;
use zhang_ast::{Directive, Include, ZhangString};
use zhang_core::ledger::Ledger;

pub(crate) fn append_directives(
    ledger: &Ledger, directives: Vec<Directive>, target_endpoint: impl Into<Option<String>>,
) {
    let (entry, endpoint) = &ledger.entry;
    let endpoint = entry.join(target_endpoint.into().unwrap_or_else(|| endpoint.clone()));

    create_folder_if_not_exist(&endpoint);

    if !ledger.visited_files.contains(&endpoint) {
        let path = match endpoint.strip_prefix(entry) {
            Ok(relative_path) => relative_path.to_str().unwrap(),
            Err(_) => endpoint.to_str().unwrap(),
        };
        append_directives(
            &ledger,
            vec![Directive::Include(Include {
                file: ZhangString::QuoteString(path.to_string()),
            })],
            None,
        );
    }
    // todo(refact): should use the exporter to export directive
    let directive_content = format!("\n{}\n", directives.into_iter().map(|it| it.to_target()).join("\n"));
    let mut ledger_base_file = OpenOptions::new().append(true).create(true).open(&endpoint).unwrap();
    ledger_base_file.write_all(directive_content.as_bytes()).unwrap();
}
