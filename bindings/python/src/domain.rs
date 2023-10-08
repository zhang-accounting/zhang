use pyo3::prelude::*;
use pyo3::types::PyDateTime;

#[pyclass]
pub struct AccountDomain(pub zhang_core::domains::schemas::AccountDomain);



#[pymethods]
impl AccountDomain {

    #[getter]
    pub fn datetime(&self) -> i64 {
        self.0.date.timestamp()
    }
    #[getter]
    pub fn r#type(&self) -> String {
        self.0.r#type.to_string()
    }

    #[getter]
    pub fn name(&self) -> String {
        self.0.name.clone()
    }
    #[getter]
    pub fn status(&self) -> &str {
        self.0.status.as_ref()
    }

    #[getter]
    pub fn alias(&self) -> Option<String> {
        self.0.alias.clone()
    }

}