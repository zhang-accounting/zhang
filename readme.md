<div align="center">
    <h1>账 Zhang</h1>
    <p>a plain text double-accounting tool which is compatible with beancount but more powerful</p>
    <img alt="GitHub Workflow Status" src="https://img.shields.io/github/workflow/status/kilerd/zhang/Develop%20Build"> <a href="https://crates.io/crates/zhang"><img alt="Crates.io" src="https://img.shields.io/crates/v/zhang"></a> <img src='https://coveralls.io/repos/github/Kilerd/zhang/badge.svg?branch=main' alt='Coverage Status' /> <img alt="Crates.io (recent)" src="https://img.shields.io/crates/dr/zhang"> <img alt="Crates.io" src="https://img.shields.io/crates/l/zhang">
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
- [ ] multiple file support
- [ ] parser span info
- [ ] error msg
- [ ] inline comment
- [ ] profit calculation logic
- [ ] embedded api server
  - [ ] file listener
  - [ ] convert `.zhang` file into sqlite
  - [ ] export sqlite change to `.zhang` file
- [ ] frontend ui
- [ ] wasm plugin system