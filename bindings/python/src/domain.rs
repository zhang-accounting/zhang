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

#[pyclass]
pub struct TransactionHeaderDomain(pub zhang_core::store::TransactionHeaderDomain);

#[pymethods]
impl TransactionHeaderDomain {
    #[getter]
    pub fn id(&self) -> String {
        self.0.id.to_string()
    }

    #[getter]
    pub fn sequence(&self) -> i32 {
        self.0.sequence
    }

    #[getter]
    pub fn datetime(&self) -> i64 {
        self.0.datetime.timestamp()
    }

    #[getter]
    pub fn flag(&self) -> String {
        self.0.flag.to_string()
    }

    #[getter]
    pub fn payee(&self) -> Option<String> {
        self.0.payee.clone()
    }

    #[getter]
    pub fn narration(&self) -> Option<String> {
        self.0.narration.clone()
    }

    #[getter]
    pub fn tags(&self) -> Vec<String> {
        self.0.tags.clone()
    }

    #[getter]
    pub fn links(&self) -> Vec<String> {
        self.0.links.clone()
    }
    // todo SpanInfo,
    // todo postings
}

#[pyclass]
pub struct PostingDomain(pub zhang_core::store::PostingDomain);

#[pymethods]
impl PostingDomain {

    #[getter]
    pub fn id(&self) -> String {
        self.0.id.to_string()
    }
    #[getter]
    pub fn account(&self) -> String {
       self.0.account.name().to_string()
    }

    #[getter]
    pub fn unit(&self) -> Option<Amount> {
        self.0.unit.clone().map(Amount)
    }
    #[getter]
    pub fn cost(&self) -> Option<Amount> {
        self.0.cost.clone().map(Amount)
    }
    #[getter]
    pub fn inferred_amount(&self) ->Amount  {
        Amount(self.0.inferred_amount.clone())
    }
    #[getter]
    pub fn previous_amount(&self) -> Amount  {
        Amount(self.0.previous_amount.clone())
    }
    #[getter]
    pub fn after_amount(&self) -> Amount  {
        Amount(self.0.after_amount.clone())
    }
}

#[pyclass]
pub struct Amount(pub zhang_ast::amount::Amount);

#[pymethods]
impl Amount {
    #[getter]
    pub fn number(&self) -> String {
        self.0.number.to_string()
    }
    #[getter]
    pub fn currency(&self) ->String {
        self.0.currency.to_owned()
    }
}

// todo price
// todo commodity lot
// todo document
// todo errors
