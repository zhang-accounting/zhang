//! Analyzers: plugins that read the ledger once it is loaded and publish diagnostics and the rows of tables that
//! queries read like zhang's own, without changing the ledger.
//!
//! An analyzer is two functions given to [`plugin!`](crate::plugin): its [`Manifest`], which zhang reads once while the
//! plugin registers, and `analyze`, which gets the records of the kinds the manifest consumes and answers an
//! [`Output`]. While it analyses, it reads the ledger with the query functions of [`crate::router`]
//! ([`query`](crate::router::query), [`query_with_params`](crate::router::query_with_params),
//! [`schema`](crate::router::schema), [`ledger_info`](crate::router::ledger_info)) and converts amounts with
//! [`convert`]: aggregate and convert in zhang, where it is exact, and add up what is left with [`crate::decimal`].
//!
//! ```no_run
//! use zhang_plugin_sdk::analysis::{Diagnostic, Input, Manifest, Output, Target};
//! use zhang_plugin_sdk::{plugin, serde_json, Error};
//!
//! plugin! {
//!     name: "shop",
//!     version: env!("CARGO_PKG_VERSION"),
//!     analyzer: analyze,
//!     manifest: manifest,
//! }
//!
//! fn manifest() -> Manifest {
//!     serde_json::from_value(serde_json::json!({
//!         "analysis_abi": 1,
//!         "consumes": ["custom:shop"],
//!         "diagnostics": [{"kind": "UnknownItem"}],
//!         "tables": [{"name": "items", "kind": "records", "columns": [{"name": "item", "type": "str"}]}]
//!     }))
//!     .expect("a manifest")
//! }
//!
//! fn analyze(input: Input) -> Result<Output, Error> {
//!     let mut output = Output::default();
//!     for record in &input.records {
//!         let item = record.fields["values"][0].as_str().unwrap_or_default();
//!         output.tables.entry("items".to_owned()).or_default().push(serde_json::Map::from_iter([("item".to_owned(), item.into())]));
//!         if item.is_empty() {
//!             output.diagnostics.push(Diagnostic {
//!                 kind: "UnknownItem".to_owned(),
//!                 target: Target { seq: Some(record.seq), ..Target::default() },
//!                 ..Diagnostic::default()
//!             });
//!         }
//!     }
//!     Ok(output)
//! }
//! ```

pub use zhang_shared::plugin_abi::analysis::{
    Cause, ColumnManifest, Conversion, ConvertedAmount, Diagnostic, DiagnosticDecl, Fill, Input, LabelManifest, LabelOutput, Manifest, Output, PartitionOutput,
    PeriodManifest, Phase, PublishedOrder, Record, RecordOrder, SeriesOutput, TableKind, TableManifest, Target, ANALYSIS_ABI, MAX_CONVERSIONS, ROW_META,
};

use crate::error::host_result;
use crate::{abi, HostError, HostErrorKind};

/// `conversions` converted with the ledger's prices, as a query's `convert(position, currency, date)` converts: for
/// each, the amount in its target currency, exact, or `None` when no price converts it. At most [`MAX_CONVERSIONS`] a
/// call. Only while the plugin analyses the ledger or handles a request; on a native build, [`HostErrorKind::Unavailable`].
pub fn convert(conversions: &[Conversion]) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
    let input = serde_json::to_vec(conversions).map_err(|e| HostError::new(HostErrorKind::Invalid, format!("cannot encode the conversions: {e}")))?;
    host_result(abi::CONVERT, &abi::convert(&input)?)
}

#[cfg(test)]
mod test {
    use super::{convert, Conversion, ConvertedAmount};
    use crate::HostErrorKind;

    #[test]
    fn should_answer_unavailable_outside_zhang() {
        let conversion = Conversion {
            amount: ConvertedAmount {
                number: "1".to_owned(),
                currency: "USD".to_owned(),
            },
            target: "CNY".to_owned(),
            date: "2024-01-01".to_owned(),
            via: None,
        };
        assert_eq!(convert(&[conversion]).unwrap_err().kind, HostErrorKind::Unavailable);
    }
}
