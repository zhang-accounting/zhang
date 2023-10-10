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

    pub fn __repr__(&self) -> String {
        format!("<AccountDomain: {}>", &self.0.name)
    }
}


#[pyclass]
pub struct CommodityDomain(pub zhang_core::domains::schemas::CommodityDomain);

#[pymethods]
impl CommodityDomain {

    #[getter]
    pub fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    pub fn precision(&self) -> i32 {
        self.0.precision
    }

    #[getter]
    pub fn prefix(&self) -> Option<String> {
        self.0.prefix.clone()
    }
    #[getter]
    pub fn suffix(&self) -> Option<String> {
        self.0.suffix.clone()
    }
    #[getter]
    pub fn rounding(&self) -> Option<String> {
        self.0.rounding.clone()
    }

    pub fn __repr__(&self) -> String {
        format!("<CommodityDomain: {}>", &self.0.name)
    }
}