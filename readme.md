<div align="center">
    <h1>账 Zhang</h1>
    <p>a plain text double-accounting tool which is compatible with beancount but more powerful</p>
    <img alt="GitHub Workflow Status" src="https://img.shields.io/github/workflow/status/kilerd/zhang/Develop%20Build">
    <a href="https://crates.io/crates/zhang"><img alt="Crates.io" src="https://img.shields.io/crates/v/zhang"></a>
    <a href='https://coveralls.io/github/Kilerd/zhang?branch=main'><img src='https://coveralls.io/repos/github/Kilerd/zhang/badge.svg?branch=main' alt='Coverage Status' /></a>
    <img alt="Crates.io (recent)" src="https://img.shields.io/crates/dr/zhang">
    <a href="docs.rs/zhang"><img alt="docs.rs" src="https://img.shields.io/docsrs/zhang"></a>
    <img alt="Crates.io" src="https://img.shields.io/crates/l/zhang">
</div>

## Tasking
- [x] `.zhang` file parser
- [x] ast
- [ ] importer
  - [x] wechat
  - [ ] alipay
- [ ] exporter
  - [x] beancount
  - [ ] sql
- [x] multiple file support
- [ ] parser span info
- [ ] error msg
- [ ] inline comment
- [x] profit calculation logic
- [x] embedded api server
  - [x] file listener
  - [x] graphql api
  - [ ] convert `.zhang` file into sqlite
  - [ ] export sqlite change to `.zhang` file
- [x] frontend ui
  - [ ] design system
- [ ] wasm plugin system