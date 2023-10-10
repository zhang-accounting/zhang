

use pyo3::prelude::*;

#[pyclass]
#[derive(Hash, Eq, PartialEq)]
pub struct Account(pub zhang_ast::Account);

#[pymethods]
impl Account {

    #[getter]
    pub fn name(&self) -> String {
        self.0.content.clone()
    }

    pub fn __str__(&self) -> &str {
        &self.0.content
    }
    pub fn __repr__(&self) -> String {
        format!("<Account: {}>", &self.0.content)
    }
}