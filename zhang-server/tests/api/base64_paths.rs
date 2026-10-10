//! The file path of `/api/files/{file_path}` and `/api/documents/{file_path}`, through the router
//! the server runs: the frontend encodes it with standard base64, whose alphabet has `/`, and a path
//! with one in its encoding (about one in ten Chinese file names) must reach its route, sent as is
//! or as `%2F`, instead of the SPA fallback. An empty or invalid path is a 400.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::{header, Method, StatusCode};
use axum::Router;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use zhang_server::util::sha256_hex;
use zhang_testkit::http::{call, router, Reply, RootedFileSystem, Settings};

/// a document whose standard base64, `YXR0YWNobWVudHMvdTEwL+S/nemZqS5wZGY=`, has a `+` and a `/`
const DOCUMENT: &str = "attachments/u10/保险.pdf";
/// a document whose standard base64, `YXR0YWNobWVudHMvcmVjZWlwdC5wZGY=`, has neither
const ASCII_DOCUMENT: &str = "attachments/receipt.pdf";
/// an included ledger file whose standard base64, `ZGF0YS/kv53pmakuemhhbmc=`, has a `/` in the middle
const FILE: &str = "data/保险.zhang";
/// a file whose standard base64, `bm90ZXMv5Li/`, ends with `/`
const TRAILING_SLASH_FILE: &str = "notes/丿";

const MAIN: &str = "include \"data/保险.zhang\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n\
                    2024-01-02 document Assets:Cash \"attachments/u10/保险.pdf\"\n";
const INCLUDED: &str = "1970-01-01 open Expenses:Insurance\n";

/// A ledger with [`MAIN`] including [`FILE`], and the two documents.
fn ledger_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::create_dir_all(dir.path().join("attachments/u10")).unwrap();
    std::fs::write(dir.path().join("main.zhang"), MAIN).unwrap();
    std::fs::write(dir.path().join(FILE), INCLUDED).unwrap();
    std::fs::write(dir.path().join(DOCUMENT), b"%PDF-1.4 insurance").unwrap();
    std::fs::write(dir.path().join(ASCII_DOCUMENT), b"%PDF-1.4 receipt").unwrap();
    dir
}

/// The server on the ledger of `dir`, read through a source without a local root, as the CLI's opendal one.
async fn server(dir: &Path) -> Router {
    router(dir, "main.zhang", Arc::new(RootedFileSystem::new(dir)), &Settings::default()).await
}

/// The standard base64 of `path`, as the frontend sends it.
fn encode(path: &str) -> String {
    BASE64_STANDARD.encode(path)
}

/// The document cache file of `path` in the ledger at `root`, which the download of a document of a source without a
/// local root (as [`RootedFileSystem`]) writes under the working directory: removed when created and dropped, so a
/// run neither reads a stale one nor leaves it behind.
struct CacheFile(PathBuf);

impl CacheFile {
    fn of(root: &Path, path: &str) -> Self {
        let file = PathBuf::from(".cache/documents").join(zhang_server::util::document_cache_key(root, path));
        std::fs::remove_file(&file).ok();
        CacheFile(file)
    }
}

impl Drop for CacheFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

async fn get(router: &Router, uri: &str) -> Reply {
    zhang_testkit::http::get(router, uri, &[]).await
}

async fn put(router: &Router, uri: &str, body: Value) -> Reply {
    zhang_testkit::http::put(router, uri, &[], &body).await
}

#[tokio::test]
async fn a_document_whose_base64_path_has_a_slash_downloads() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;
    let _cache = CacheFile::of(dir.path(), DOCUMENT);
    let encoded = encode(DOCUMENT);
    assert_eq!(encoded, "YXR0YWNobWVudHMvdTEwL+S/nemZqS5wZGY=");

    let reply = get(&router, &format!("/api/documents/{encoded}")).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.text());
    assert_eq!(reply.bytes.as_ref(), b"%PDF-1.4 insurance");
    assert_eq!(
        reply.headers[header::CONTENT_DISPOSITION].as_bytes(),
        "inline; filename=\"保险.pdf\"".as_bytes()
    );
    // the cache file is one flat file, named by the hash of the ledger root and the path, whatever the path holds
    assert!(_cache.0.is_file(), "the download should cache {:?}", _cache.0);
    assert_eq!(_cache.0.parent(), Some(Path::new(".cache/documents")));

    // the frontend's API client percent encodes a path parameter
    let percent_encoded = encoded.replace('+', "%2B").replace('/', "%2F").replace('=', "%3D");
    let reply = get(&router, &format!("/api/documents/{percent_encoded}")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.bytes.as_ref(), b"%PDF-1.4 insurance");
}

#[tokio::test]
async fn a_document_whose_base64_path_has_no_slash_downloads() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;
    let _cache = CacheFile::of(dir.path(), ASCII_DOCUMENT);
    let encoded = encode(ASCII_DOCUMENT);
    assert!(!encoded.contains(['/', '+']), "{encoded}");

    let reply = get(&router, &format!("/api/documents/{encoded}")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.bytes.as_ref(), b"%PDF-1.4 receipt");
    assert_eq!(reply.headers[header::CONTENT_DISPOSITION], "inline; filename=\"receipt.pdf\"");
}

/// The documents page lists the document with the path the download route takes, through the built-in query
/// `journals.documents`, next to the download routes under `/api/documents/`.
#[tokio::test]
async fn the_document_list_names_the_path_the_download_takes() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;

    let reply = call(
        &router,
        Method::POST,
        "/api/query/builtins/journals.documents",
        &[],
        Some(&json!({ "params": {} })),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let result = reply.json()["data"].clone();
    let path = result["columns"].as_array().unwrap().iter().position(|it| it["name"] == "path").unwrap();
    let documents = result["rows"].as_array().cloned().unwrap();
    assert_eq!(documents.len(), 1, "{documents:?}");
    assert_eq!(documents[0][path], DOCUMENT);
}

#[tokio::test]
async fn a_file_whose_base64_path_has_a_slash_reads_and_saves() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;
    let encoded = encode(FILE);
    assert_eq!(encoded, "ZGF0YS/kv53pmakuemhhbmc=");

    let reply = get(&router, &format!("/api/files/{encoded}")).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.text());
    assert_eq!(
        reply.json()["data"],
        json!({"path": FILE, "content": INCLUDED, "sha256": sha256_hex(INCLUDED.as_bytes())})
    );

    let content = "1970-01-01 open Expenses:Insurance\n1970-01-01 open Expenses:Health\n";
    let reply = put(&router, &format!("/api/files/{encoded}"), json!({ "content": content })).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.text());
    assert_eq!(std::fs::read_to_string(dir.path().join(FILE)).unwrap(), content);

    // and percent encoded, as the frontend's API client sends it
    let reply = get(&router, &format!("/api/files/{}", encoded.replace('/', "%2F").replace('=', "%3D"))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["data"]["content"], content);
}

#[tokio::test]
async fn a_file_whose_base64_path_ends_with_a_slash_reads_and_saves() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;
    let encoded = encode(TRAILING_SLASH_FILE);
    assert_eq!(encoded, "bm90ZXMv5Li/");

    let reply = put(&router, &format!("/api/files/{encoded}"), json!({ "content": "a note\n" })).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.text());
    assert_eq!(std::fs::read_to_string(dir.path().join(TRAILING_SLASH_FILE)).unwrap(), "a note\n");

    let reply = get(&router, &format!("/api/files/{encoded}")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json()["data"],
        json!({"path": TRAILING_SLASH_FILE, "content": "a note\n", "sha256": sha256_hex(b"a note\n")})
    );
}

#[tokio::test]
async fn a_file_whose_base64_path_has_no_slash_reads_and_saves() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;
    let encoded = encode("main.zhang");
    assert!(!encoded.contains(['/', '+']), "{encoded}");

    let reply = get(&router, &format!("/api/files/{encoded}")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json()["data"],
        json!({"path": "main.zhang", "content": MAIN, "sha256": sha256_hex(MAIN.as_bytes())})
    );

    let content = format!("{MAIN}1970-01-01 open Assets:Bank\n");
    let reply = put(&router, &format!("/api/files/{encoded}"), json!({ "content": content })).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(std::fs::read_to_string(dir.path().join("main.zhang")).unwrap(), content);
}

#[tokio::test]
async fn an_empty_or_invalid_path_is_a_bad_request() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;

    for uri in [
        "/api/documents/",
        "/api/files/",
        "/api/documents/not%20base64",
        "/api/files/not%20base64",
        "/api/files/%FF",
    ] {
        let reply = get(&router, uri).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "GET {uri}: {:?}", reply.text());
    }
    // base64, but not of UTF-8 text
    let reply = get(&router, &format!("/api/files/{}", BASE64_STANDARD.encode([0xff, 0xfe]))).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    for uri in ["/api/files/", "/api/files/not%20base64"] {
        let reply = put(&router, uri, json!({ "content": "" })).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "PUT {uri}");
    }
}

#[tokio::test]
async fn the_openapi_document_keeps_the_file_path_parameter() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;

    let spec = get(&router, "/openapi.json").await.json();
    let paths = spec["paths"].as_object().unwrap();
    let mut routes = paths
        .keys()
        .filter(|path| path.starts_with("/api/files") || path.starts_with("/api/documents"))
        .cloned()
        .collect::<Vec<_>>();
    routes.sort();
    // the download routes under `/api/documents/` take a catch-all the document cannot describe
    assert_eq!(routes, ["/api/files", "/api/files/{file_path}"]);

    let file = &paths["/api/files/{file_path}"];
    for method in ["get", "put"] {
        let parameters = file[method]["parameters"].as_array().unwrap();
        assert_eq!(
            parameters,
            &[json!({"name": "file_path", "in": "path", "required": true, "schema": {"type": "string"}})],
            "{method}"
        );
    }
}
