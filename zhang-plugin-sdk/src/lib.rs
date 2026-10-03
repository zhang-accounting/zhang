//! Write [zhang](https://github.com/zhang-accounting/zhang) WASM plugins in Rust.
//!
//! A zhang plugin is a WebAssembly module built with [Extism](https://extism.org/) for
//! `wasm32-unknown-unknown`. This crate wraps zhang's plugin ABI v1 so a plugin is a few plain functions:
//!
//! - [`plugin!`] exports `name`, `version`, `supported_type` and the handlers, doing the JSON for you;
//! - [`config`] reads the plugin's config: flat keys, its directive's arguments and meta, ledger options, and
//!   [`Config::resolve`](config::Config::resolve) for settings that can come from several places;
//! - [`custom`] reads config written as dated `custom "<plugin>" "<key>" …` directives;
//! - [`clock`] gives the date of the load and deterministic randomness;
//! - [`fs`] reads the ledger files the plugin's `allowed_paths` grant;
//! - [`errors`] reports problems in the ledger's error list;
//! - [`router`] serves HTTP requests and queries the ledger.
//!
//! ```no_run
//! use zhang_plugin_sdk::{plugin, Directive, Error, Stream};
//!
//! plugin! {
//!     name: "drop-comments",
//!     version: env!("CARGO_PKG_VERSION"),
//!     processor: process,
//! }
//!
//! fn process(stream: Stream) -> Result<Stream, Error> {
//!     Ok(stream.into_iter().filter(|it| !matches!(it.data, Directive::Comment(_))).collect())
//! }
//! ```
//!
//! # Building
//!
//! A plugin crate is a `cdylib` built for `wasm32-unknown-unknown`:
//!
//! ```toml
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [dependencies]
//! zhang-plugin-sdk = { git = "https://github.com/zhang-accounting/zhang" }
//! ```
//!
//! ```sh
//! rustup target add wasm32-unknown-unknown
//! cargo build --release --target wasm32-unknown-unknown
//! ```
//!
//! The SDK is versioned with zhang and not published on crates.io yet. The directive JSON a plugin exchanges with
//! zhang is `zhang-ast`'s serde representation, so pin the git dependency to the zhang release you run (`tag =
//! "v…"`): a plugin built against an older `zhang-ast` fails to read a directive kind a newer zhang added.
//!
//! On any target other than `wasm32`, the crate still compiles: [`plugin!`] exports nothing, config reads find
//! nothing and host functions answer [`HostErrorKind::Unavailable`]. That lets you unit test plugin logic with a
//! plain `cargo test`, building a [`Config`](config::Config) with [`Config::from_map`](config::Config::from_map).
//!
//! # Minimum zhang version
//!
//! A plugin only imports the host functions it calls, and a plugin importing one fails to load on a zhang that
//! does not have it. Everything here needs a zhang with plugin ABI v1 (the release after 0.2.0) except
//! [`plugin!`] with a processor or mapper and the flat config keys ([`Config::get`](config::Config::get)),
//! which work since 0.2.0.

mod abi;
pub mod clock;
pub mod config;
pub mod custom;
mod error;
pub mod errors;
pub mod fs;
pub mod router;

pub use bigdecimal;
pub use chrono;
/// The Extism plug-in development kit the SDK builds on, e.g. for HTTP requests to the hosts a plugin's
/// `allowed_hosts` grant (`extism_pdk::http::request`). Only on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub use extism_pdk;
pub use serde_json;
pub use zhang_ast as ast;
pub use zhang_ast::{Directive, Meta, PluginType, SpanInfo, Spanned};

pub use crate::error::{Error, HostError, HostErrorKind};

/// the directive stream a processor receives and returns
pub type Stream = Vec<Spanned<Directive>>;

/// the plugin ABI version this SDK speaks; see [`Config::abi`](config::Config::abi)
pub const ABI_VERSION: u32 = 1;

/// The common imports of a plugin.
pub mod prelude {
    pub use crate::config::{Config, Source, Values};
    pub use crate::router::{Request, Response};
    pub use crate::{clock, custom, errors, fs, plugin, router, Directive, Error, HostError, HostErrorKind, Meta, SpanInfo, Spanned, Stream};
}

/// Export a plugin: its `name`, its `version`, its `supported_type` (derived from the handlers given) and the
/// handlers, with the JSON (de)serialization done for you.
///
/// ```no_run
/// use zhang_plugin_sdk::router::{Request, Response};
/// use zhang_plugin_sdk::{plugin, Error, Spanned, Directive, Stream};
///
/// plugin! {
///     name: "my-plugin",
///     version: env!("CARGO_PKG_VERSION"),
///     processor: process,
///     mapper: map,
///     router: route,
/// }
///
/// /// the whole stream in, the new stream out
/// fn process(stream: Stream) -> Result<Stream, Error> { Ok(stream) }
///
/// /// one directive in, the directives replacing it out
/// fn map(directive: Spanned<Directive>) -> Result<Stream, Error> { Ok(vec![directive]) }
///
/// /// an HTTP request to `/api/plugins/my-plugin/…` in, the response out
/// fn route(request: Request) -> Result<Response, Error> { Ok(Response::text(request.path)) }
/// ```
///
/// `name` and `version` are any expressions serializing to a JSON string. Each handler is optional, but give at
/// least one and keep the order `processor`, `mapper`, `router`. A handler returning `Err` fails its call: a
/// processor or mapper aborts the whole ledger load with the error's message, a router answers status 500.
///
/// The handler signatures are checked on every target. The exports exist only on `wasm32`, so a native build of
/// the plugin crate (e.g. for `cargo test`) links nothing from zhang.
#[macro_export]
macro_rules! plugin {
    (
        name: $name:expr,
        version: $version:expr
        $(, processor: $processor:path)?
        $(, mapper: $mapper:path)?
        $(, router: $router:path)?
        $(,)?
    ) => {
        $(const _: fn($crate::Stream) -> ::core::result::Result<$crate::Stream, $crate::Error> = $processor;)?
        $(const _: fn($crate::Spanned<$crate::Directive>) -> ::core::result::Result<$crate::Stream, $crate::Error> = $mapper;)?
        $(const _: fn($crate::router::Request) -> ::core::result::Result<$crate::router::Response, $crate::Error> = $router;)?

        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            #[allow(dead_code)]
            fn metadata() {
                let _ = ($name, $version);
            }
        };

        #[cfg(target_arch = "wasm32")]
        const _: () = {
            #[export_name = "name"]
            pub extern "C" fn __zhang_plugin_name() -> i32 {
                $crate::__private::export(|| ::core::result::Result::Ok($name))
            }

            #[export_name = "version"]
            pub extern "C" fn __zhang_plugin_version() -> i32 {
                $crate::__private::export(|| ::core::result::Result::Ok($version))
            }

            #[export_name = "supported_type"]
            pub extern "C" fn __zhang_plugin_supported_type() -> i32 {
                #[allow(unused_mut)]
                let mut types = ::std::vec::Vec::<$crate::PluginType>::new();
                $(let _ = stringify!($processor); types.push($crate::PluginType::Processor);)?
                $(let _ = stringify!($mapper); types.push($crate::PluginType::Mapper);)?
                $(let _ = stringify!($router); types.push($crate::PluginType::Router);)?
                $crate::__private::export(|| ::core::result::Result::Ok(types))
            }

            $(
                #[export_name = "processor"]
                pub extern "C" fn __zhang_plugin_processor() -> i32 {
                    $crate::__private::export_with_input("processor", $processor)
                }
            )?

            $(
                #[export_name = "mapper"]
                pub extern "C" fn __zhang_plugin_mapper() -> i32 {
                    $crate::__private::export_with_input("mapper", $mapper)
                }
            )?

            $(
                #[export_name = "router"]
                pub extern "C" fn __zhang_plugin_router() -> i32 {
                    $crate::__private::export_with_input("router", $router)
                }
            )?
        };
    };
}

/// What [`plugin!`] expands to. Not part of the public API.
#[doc(hidden)]
#[cfg(target_arch = "wasm32")]
pub mod __private {
    use extism_pdk::Memory;
    use serde::de::DeserializeOwned;
    use serde::Serialize;

    use crate::Error;

    /// run an export without input: write its result as JSON output, or fail the call with its error
    pub fn export<O: Serialize>(handler: impl FnOnce() -> Result<O, Error>) -> i32 {
        finish(handler())
    }

    /// run an export taking JSON input
    pub fn export_with_input<I: DeserializeOwned, O: Serialize>(export: &str, handler: impl FnOnce(I) -> Result<O, Error>) -> i32 {
        let input = extism_pdk::input_bytes();
        match serde_json::from_slice::<I>(&input) {
            Ok(input) => finish(handler(input)),
            Err(e) => fail(&format!(
                "the plugin cannot read its `{export}` input: {e}. A plugin built against an older zhang may not know a newer directive; rebuild it against the zhang you run"
            )),
        }
    }

    fn finish<O: Serialize>(result: Result<O, Error>) -> i32 {
        let output = result.and_then(|output| serde_json::to_vec(&output).map_err(Error::from));
        match output.and_then(|output| Memory::from_bytes(output).map_err(|e| Error::msg(format!("cannot write the output: {e}")))) {
            Ok(memory) => {
                memory.set_output();
                0
            }
            Err(e) => fail(e.message()),
        }
    }

    /// fail the call with `message`, which the host reports
    fn fail(message: &str) -> i32 {
        if let Ok(memory) = Memory::from_bytes(message.as_bytes()) {
            // SAFETY: the kernel keeps the block as the call's error
            unsafe { extism_pdk::extism::error_set(memory.offset()) };
        }
        1
    }
}
