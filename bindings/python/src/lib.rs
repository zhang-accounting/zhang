use std::collections::HashMap;
use std::env::temp_dir;
use std::path::PathBuf;
use std::str::FromStr;

use pyo3::prelude::*;
use zhang_ast::Spanned;
use zhang_core::text::transformer::TextTransformer;

use crate::domain::CommodityDomain;

pub mod ast;
pub mod domain;
#[pyclass]
pub struct Directive(Spanned<zhang_ast::Directive>);

#[pyclass]
pub struct Ledger(zhang_core::ledger::Ledger);

#[pymethods]
impl Ledger {
    #[new]
    pub fn new(path: &str, endpoint: &str) -> PyResult<Self> {
        let pathbuf = PathBuf::from_str(path)?;
        Ok(Ledger(
            zhang_core::ledger::Ledger::load::<TextTransformer>(pathbuf, endpoint.to_owned()).unwrap(),
        ))
    }

    #[staticmethod]
    pub fn from_string(content: &str) -> PyResult<Self> {
        let t_dir = temp_dir();
        let endpoint = t_dir.join("main.zhang");
        std::fs::write(endpoint, content)?;
        Ok(Ledger(
            zhang_core::ledger::Ledger::load::<TextTransformer>(t_dir, "main.zhang".to_owned()).unwrap(),
        ))
    }

    #[getter]
    pub fn options(&self) -> PyResult<HashMap<String, String>> {
        let store = self.0.store.read().unwrap();
        Ok(store.options.clone())
    }

    #[getter]
    pub fn accounts(&self) -> PyResult<HashMap<ast::Account, domain::AccountDomain>> {
        let store = self.0.store.read().unwrap();
        Ok(store
            .accounts
            .clone()
            .into_iter()
            .map(|(key, value)| (ast::Account(key), domain::AccountDomain(value)))
            .collect())
    }

    #[getter]
    pub fn commodities(&self) -> PyResult<HashMap<String, CommodityDomain>> {
        let store = self.0.store.read().unwrap();
        Ok(store
            .commodities
            .clone()
            .into_iter()
            .map(|(key, value)| (key, domain::CommodityDomain(value)))
            .collect())
    }
}

/// A Python module implemented in Rust.
#[pymodule]
fn zhang(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<Ledger>()?;
    m.add_class::<ast::Account>()?;
    m.add_class::<domain::AccountDomain>()?;
    m.add_class::<domain::CommodityDomain>()?;
    m.add_class::<domain::TransactionHeaderDomain>()?;
    m.add_class::<domain::PostingDomain>()?;
    m.add_class::<domain::Amount>()?;
    Ok(())
}
