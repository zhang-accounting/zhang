//! The file path of `/api/files/{file_path}` and `/api/documents/{file_path}`, through the router
//! the server runs: the frontend encodes it with standard base64, whose alphabet has `/`, and a path
//! with one in its encoding (about one in ten Chinese file names) must reach its route, sent as is
//! or as `%2F`, instead of the SPA fallback. An empty or invalid path is a 400.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use gotcha::{GotchaApp, GotchaContext};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower::ServiceExt;
use zhang_core::data_source::{DataSource, LoadResult, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::ZhangResult;
use zhang_server::broadcast::Broadcaster;
use zhang_server::util::sha256_hex;
use zhang_server::{create_server_app, ReloadSender, ServeConfig};

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

/// A local file system data source resolving relative paths against the ledger root, like the
/// opendal one the CLI uses: a missing file reads as empty, saving creates the parent folders.
struct RootedFileSystem {
    root: PathBuf,
    inner: LocalFileSystemDataSource,
}

impl RootedFileSystem {
    fn resolve(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        }
    }
}

impl DataSource for RootedFileSystem {
    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        match std::fs::read(self.resolve(&path)) {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        self.inner.load(entry, endpoint)
    }

    fn save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        let path = self.resolve(&path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

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

async fn server(dir: &Path) -> Router {
    let source = Arc::new(RootedFileSystem {
        root: dir.to_path_buf(),
        inner: LocalFileSystemDataSource::new(ZhangDataType {}),
    });
    let ledger = Ledger::load(dir.to_path_buf(), "main.zhang".to_owned(), source.clone()).unwrap_or_else(|error| panic!("ledger should load: {error}"));
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    let app = create_server_app(
        ServeConfig {
            path: dir.to_path_buf(),
            endpoint: "main.zhang".to_owned(),
            addr: "127.0.0.1".to_owned(),
            port: 0,
            no_report: true,
            data_source: source,
            auth_credential: None,
            passkey_secret: None,
            passkey_rp_id: None,
            passkey_origin: None,
            session_secret: None,
        },
        Arc::new(RwLock::new(ledger)),
        Broadcaster::create(),
        Arc::new(ReloadSender::new(sender)),
    );
    let config = app.config().await.unwrap();
    let state = app.state(&config).await.unwrap_or_else(|error| panic!("state should build: {error}"));
    app.build_router(GotchaContext { config, state }).await.unwrap()
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

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("expect JSON, got {:?}", String::from_utf8_lossy(&self.body)))
    }
}

async fn call(router: &Router, method: Method, uri: &str, body: Option<Value>) -> Reply {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply { status, headers, body }
}

async fn get(router: &Router, uri: &str) -> Reply {
    call(router, Method::GET, uri, None).await
}

async fn put(router: &Router, uri: &str, body: Value) -> Reply {
    call(router, Method::PUT, uri, Some(body)).await
}

#[tokio::test]
async fn a_document_whose_base64_path_has_a_slash_downloads() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;
    let _cache = CacheFile::of(dir.path(), DOCUMENT);
    let encoded = encode(DOCUMENT);
    assert_eq!(encoded, "YXR0YWNobWVudHMvdTEwL+S/nemZqS5wZGY=");

    let reply = get(&router, &format!("/api/documents/{encoded}")).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", String::from_utf8_lossy(&reply.body));
    assert_eq!(reply.body.as_ref(), b"%PDF-1.4 insurance");
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
    assert_eq!(reply.body.as_ref(), b"%PDF-1.4 insurance");
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
    assert_eq!(reply.body.as_ref(), b"%PDF-1.4 receipt");
    assert_eq!(reply.headers[header::CONTENT_DISPOSITION], "inline; filename=\"receipt.pdf\"");
}

/// The documents page lists the document with the path the download route takes, through the built-in query
/// `journals.documents`, next to the download routes under `/api/documents/`.
#[tokio::test]
async fn the_document_list_names_the_path_the_download_takes() {
    let dir = ledger_dir();
    let router = server(dir.path()).await;

    let reply = call(&router, Method::POST, "/api/query/builtins/journals.documents", Some(json!({ "params": {} }))).await;
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
    assert_eq!(reply.status, StatusCode::OK, "{:?}", String::from_utf8_lossy(&reply.body));
    assert_eq!(
        reply.json()["data"],
        json!({"path": FILE, "content": INCLUDED, "sha256": sha256_hex(INCLUDED.as_bytes())})
    );

    let content = "1970-01-01 open Expenses:Insurance\n1970-01-01 open Expenses:Health\n";
    let reply = put(&router, &format!("/api/files/{encoded}"), json!({ "content": content })).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", String::from_utf8_lossy(&reply.body));
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
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", String::from_utf8_lossy(&reply.body));
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
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "GET {uri}: {:?}", String::from_utf8_lossy(&reply.body));
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
