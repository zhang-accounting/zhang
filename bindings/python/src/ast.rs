

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
}