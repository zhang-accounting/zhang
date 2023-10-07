use pyo3::prelude::*;
use pyo3::types::PyString;
use zhang_ast::{Spanned};

use zhang_core::text::parser::parse;
/// Formats the sum of two numbers as string.

#[pyclass]
pub struct Directive(Spanned<zhang_ast::Directive>);


#[pyfunction]
fn parse_content(content: &str)-> PyResult<Vec<Directive>> {
    Ok(parse(content, None).unwrap().into_iter().map(Directive).collect())
}

/// A Python module implemented in Rust.
#[pymodule]
fn zhang(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_content, m)?)?;
    Ok(())
}