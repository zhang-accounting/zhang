//! The process tests of the `zhang` binary as one test binary: every module under `tests/cli/` spawns the binary
//! cargo built (`CARGO_BIN_EXE_zhang`) and reads its exit code and output. One suite runs with
//! `cargo nextest run -p zhang -E 'binary(cli) & test(log_filter::)'`.

mod load_error_output;
mod log_filter;
mod serve_exit_code;
mod version;
