use std::path::Path;

use axum::extract::State;
use gotcha::api;
use zhang_core::data_source::{slashed, FileText};
use zhang_core::utils::BOM;

use crate::error::ServerError;
use crate::request::FileUpdateRequest;
use crate::response::{Created, FileDetailEntity, ResponseWrapper};
use crate::routes::Base64Path;
use crate::state::{wrote, SharedLedger, SharedReloadSender};
use crate::util::sha256_hex;
use crate::{ApiResult, ServerResult};

#[api(group = "file")]
pub async fn get_files(ledger: State<SharedLedger>) -> ApiResult<Vec<Option<String>>> {
    let ledger = ledger.read().await;
    // by their paths within the ledger, which the editor reads and writes them by
    let files = ledger
        .visited_files
        .iter()
        .filter_map(|path| ledger.path_in_ledger(path))
        .map(|path| Some(slashed(&path)));
    ResponseWrapper::json(files.collect())
}

/// The fingerprint of a file as the editor is shown it: the SHA-256, in hex, of its content after the byte order mark
/// it may start with, which the editor is not shown ([`FileText`]). Computed from the bytes on disk both when the file
/// is shown and when it is saved over, so the two are hashed alike.
fn fingerprint(file: &[u8]) -> String {
    sha256_hex(file.strip_prefix(BOM.as_bytes()).unwrap_or(file))
}

#[api(group = "file")]
pub async fn get_file_content(ledger: State<SharedLedger>, Base64Path(filename): Base64Path) -> ApiResult<FileDetailEntity> {
    let ledger = ledger.read().await;

    let content = match ledger.data_source.get(filename.to_owned()) {
        Ok(content) => content,
        Err(error) if error.is_file_not_found() => {
            let (root, main) = &ledger.entry;
            // the main file of a ledger started without one (`zhang serve` on a new folder) is the empty ledger served,
            // which the editor writes; any other file that is not there is not shown as an empty one
            let main = ledger.path_in_ledger(&root.join(main));
            if main.is_none() || ledger.path_in_ledger(Path::new(&filename)) != main {
                return Err(ServerError::NoSuchFile(filename));
            }
            vec![]
        }
        Err(error) => return Err(error.into()),
    };
    // the fingerprint of the file as shown: a save sends it back, so a file changed since is not overwritten (#506)
    let sha256 = fingerprint(&content);
    // without the byte order mark the file may start with, as the parsers read it (#505). A file that is not UTF-8
    // text, as an image in the ledger's directory, is not shown: the answer names it
    let content = FileText::decode(content, &filename)?.text;

    ResponseWrapper::json(FileDetailEntity {
        path: filename,
        content,
        sha256,
    })
}

#[api(group = "file")]
pub async fn update_file_content(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, Base64Path(filename): Base64Path,
    axum::extract::Json(payload): axum::extract::Json<FileUpdateRequest>,
) -> ServerResult<Created> {
    // the whole file: it edits no place the ledger loaded, so it needs the ledger reloaded first no more than a
    // ledger that loads. It is saved even when the files cannot be loaded, to fix them
    let mut ledger = ledger.write().await;

    // the file as it is now, read once: for the fingerprint the editor loaded it with, and for the byte order mark it
    // may start with. A file that cannot be read is written as sent: the save, which may fix it, is never held up by
    // the read (#505), unless the save is to be checked against the fingerprint, which nothing stands in for
    let existing = match ledger.data_source.get_existing(filename.clone()) {
        Ok(existing) => existing,
        Err(error) if payload.expected_sha256.is_some() => return Err(error.into()),
        Err(_) => None,
    };

    // a file that changed since the editor loaded it (a transaction or a balance check recorded in the UI, a document
    // uploaded, an edit outside) is not overwritten, which would undo the change (#506). The ledger is held
    // exclusively from the read to the save, as for an edit in place, so no write comes between. A file that is gone
    // has nothing to undo, and is written. A save without the fingerprint, from an older client, overwrites the file
    // as before
    if let (Some(expected), Some(existing)) = (&payload.expected_sha256, &existing) {
        if !fingerprint(existing).eq_ignore_ascii_case(expected) {
            return Err(ServerError::Conflict(format!(
                "the file {filename} changed since it was opened in the editor, so nothing was written: reload it to see the change, and make your edit again"
            )));
        }
    }

    // the editor shows the file without the byte order mark it may start with: a file that has one keeps it (#505)
    let had_bom = existing.is_some_and(|existing| existing.starts_with(BOM.as_bytes()));
    let mut content = FileText::new(payload.content);
    content.bom |= had_bom;
    let saved = ledger.data_source.save(&ledger, filename, &content.into_bytes());
    wrote(&mut ledger, &reload_sender, saved.map_err(ServerError::from))?;
    Ok(Created)
}

#[cfg(test)]
mod save_test {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;
    use tokio::sync::{mpsc, RwLock};
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::ledger::Ledger;

    use super::{get_file_content, get_files, update_file_content};
    use crate::request::{CreateTransactionRequest, FileUpdateRequest, JournalRequest};
    use crate::routes::common::get_errors;
    use crate::routes::transaction::create_new_transaction;
    use crate::routes::{Base64Path, Query};
    use crate::state::{SharedLedger, SharedReloadSender};
    use crate::util::sha256_hex;
    use crate::ReloadSender;

    const LEDGER: &str = "option \"operating_currency\" \"CNY\"\n2020-01-01 commodity CNY\n2024-01-01 open Assets:A\n2024-01-01 open Income:X\n";

    /// A ledger loaded from `ledger` as its main file, `main.bean`: its directory, the main file, and the states the
    /// routes take.
    async fn opened(ledger: &str) -> (PathBuf, PathBuf, State<SharedLedger>, State<SharedReloadSender>) {
        let dir = std::env::temp_dir().join(format!("zhang-file-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = dir.join("main.bean");
        std::fs::write(&main, ledger).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let loaded = Ledger::load(dir.clone(), "main.bean".to_owned(), source).expect("load ledger");
        let state = State(SharedLedger(Arc::new(RwLock::new(loaded))));
        let (sender, _receiver) = mpsc::channel(8);
        let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
        (dir, main, state, reload)
    }

    /// What the editor is served for `main`: its content, and the fingerprint of it.
    async fn shown(state: &State<SharedLedger>, main: &Path) -> (String, String) {
        let response = get_file_content(state.clone(), Base64Path(main.to_string_lossy().into_owned()))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
        let data = &body["data"];
        (data["content"].as_str().unwrap().to_owned(), data["sha256"].as_str().unwrap().to_owned())
    }

    fn new_transaction() -> CreateTransactionRequest {
        serde_json::from_value(serde_json::json!({
            "datetime": "2024-06-01T12:00:00Z",
            "payee": "Shop",
            "flag": "Okay",
            "narration": "seed",
            "postings": [
                {"account": "Assets:A", "unit": {"number": "50", "commodity": "CNY"}},
                {"account": "Income:X", "unit": null}
            ],
            "metas": [],
            "tags": [],
            "links": []
        }))
        .unwrap()
    }

    /// A save carries the fingerprint the file was served with, and a file that changed since is not overwritten
    /// (#506): a transaction recorded in the UI, which appends to the file, or an edit outside. The save is refused
    /// with 409, and the file is left as it is; reloaded, the editor saves again. A save without the fingerprint,
    /// from an older client, overwrites the file as before.
    #[tokio::test]
    async fn a_save_from_an_editor_whose_file_changed_since_is_refused_and_writes_nothing() {
        let (dir, main, state, reload) = opened(LEDGER).await;
        let save = |content: String, expected_sha256: Option<String>| {
            let path = Base64Path(main.to_string_lossy().into_owned());
            update_file_content(state.clone(), reload.clone(), path, Json(FileUpdateRequest { content, expected_sha256 }))
        };
        let on_disk = || std::fs::read(&main).unwrap();

        let (content, fingerprint) = shown(&state, &main).await;
        assert_eq!(content, LEDGER);
        assert_eq!(fingerprint, sha256_hex(LEDGER.as_bytes()));

        // the file as it was served: written
        let edited = format!("{LEDGER}2024-01-01 open Assets:B\n");
        assert_eq!(answer(save(edited.clone(), Some(fingerprint)).await).await.0, StatusCode::CREATED);
        assert_eq!(on_disk(), edited.as_bytes());
        let (_, fingerprint) = shown(&state, &main).await;

        // a transaction recorded in the UI appends to the main file in between (the include of its output file)
        let (status, message) = answer(create_new_transaction(state.clone(), reload.clone(), Json(new_transaction())).await).await;
        assert_eq!(status, StatusCode::OK, "{message}");
        let appended = on_disk();
        assert_ne!(appended, edited.as_bytes(), "the main file changed");
        let stale = format!("{edited}2024-01-01 open Assets:C\n");
        let (status, message) = answer(save(stale.clone(), Some(fingerprint)).await).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(
            message.contains("changed since it was opened in the editor, so nothing was written"),
            "{message}"
        );
        assert!(message.contains("reload it"), "{message}");
        assert_eq!(on_disk(), appended, "the file is left byte for byte as it was");

        // reloaded, the editor saves
        let (content, fingerprint) = shown(&state, &main).await;
        assert_eq!(content.as_bytes(), appended);
        let merged = format!("{content}2024-01-01 open Assets:C\n");
        assert_eq!(answer(save(merged.clone(), Some(fingerprint)).await).await.0, StatusCode::CREATED);
        assert_eq!(on_disk(), merged.as_bytes());

        // an edit outside in between: a save from the editor as it was is refused the same
        let (_, fingerprint) = shown(&state, &main).await;
        let outside = format!("{merged}2024-01-01 open Assets:D\n");
        std::fs::write(&main, &outside).unwrap();
        let (status, message) = answer(save(stale, Some(fingerprint)).await).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert_eq!(on_disk(), outside.as_bytes());

        // a client sending no fingerprint overwrites the file, as before
        assert_eq!(answer(save(merged.clone(), None).await).await.0, StatusCode::CREATED);
        assert_eq!(on_disk(), merged.as_bytes());
        std::fs::remove_dir_all(dir).ok();
    }

    /// The fingerprint is of the file as the editor is shown it, without the byte order mark it may start with
    /// (#505): the fingerprint of the text shown. A save with it is written, and keeps the mark.
    #[tokio::test]
    async fn the_fingerprint_is_of_the_file_as_shown_without_its_byte_order_mark() {
        let (dir, main, state, reload) = opened(LEDGER).await;
        std::fs::write(&main, format!("\u{feff}{LEDGER}")).unwrap();
        let (content, fingerprint) = shown(&state, &main).await;
        assert_eq!(content, LEDGER, "the mark is not shown");
        assert_eq!(fingerprint, sha256_hex(content.as_bytes()), "the fingerprint is of the text shown");

        let edited = format!("{LEDGER}2024-01-01 open Assets:B\n");
        let path = Base64Path(main.to_string_lossy().into_owned());
        let request = FileUpdateRequest {
            content: edited.clone(),
            expected_sha256: Some(fingerprint),
        };
        assert_eq!(
            answer(update_file_content(state.clone(), reload.clone(), path, Json(request)).await).await.0,
            StatusCode::CREATED
        );
        assert_eq!(std::fs::read_to_string(&main).unwrap(), format!("\u{feff}{edited}"), "the mark is kept");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The check does not hold up fixing a file that does not parse (#508): the fix is saved with the fingerprint of
    /// the broken file as shown, the ledger loading or not. A buffer older than the broken file is refused the same,
    /// and leaves the file as it is.
    #[tokio::test]
    async fn a_file_that_does_not_parse_is_fixed_with_the_fingerprint_it_is_shown_with() {
        let (dir, main, state, reload) = opened(LEDGER).await;
        let save = |content: String, expected_sha256: Option<String>| {
            let path = Base64Path(main.to_string_lossy().into_owned());
            update_file_content(state.clone(), reload.clone(), path, Json(FileUpdateRequest { content, expected_sha256 }))
        };
        let create = || create_new_transaction(state.clone(), reload.clone(), Json(new_transaction()));

        let (_, fingerprint) = shown(&state, &main).await;
        let broken = format!("{LEDGER}this is not beancount\n");
        assert_eq!(answer(save(broken.clone(), Some(fingerprint.clone())).await).await.0, StatusCode::CREATED);
        let (status, message) = answer(create().await).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(message.contains("the ledger cannot be loaded from its files as they are now"), "{message}");

        // a buffer from before the break is refused, and the broken file is left for the fix
        let (status, message) = answer(save(LEDGER.to_owned(), Some(fingerprint)).await).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), broken);

        // the fix, from the editor showing the broken file, is written while the ledger cannot be loaded
        let (content, fingerprint) = shown(&state, &main).await;
        assert_eq!(content, broken);
        assert_eq!(answer(save(LEDGER.to_owned(), Some(fingerprint)).await).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), LEDGER);
        let (status, message) = answer(create().await).await;
        assert_eq!(status, StatusCode::OK, "{message}");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The editor is answered 415, with a message naming the file, for a file that is not UTF-8 text: an image in the
    /// ledger's directory, or a file saved in another encoding. The handler panicked, which dropped the connection.
    #[tokio::test]
    async fn the_editor_is_answered_415_for_a_file_that_is_not_text() {
        let (dir, _, state, _) = opened(LEDGER).await;
        std::fs::create_dir_all(dir.join("attachments")).unwrap();
        let image = dir.join("attachments/img.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
        let latin1 = dir.join("latin1.bean");
        std::fs::write(&latin1, b"; first line\n; \xe9t\xe9\n").unwrap();

        for (file, line) in [(image, 1), (latin1, 2)] {
            let path = file.to_string_lossy().into_owned();
            let (status, message) = answer(get_file_content(state.clone(), Base64Path(path.clone())).await).await;
            assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{message}");
            assert!(message.contains(&format!("the file {path} is not UTF-8 text: line {line}")), "{message}");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// The file list names the ledger's files by their paths within it, and the editor reads and saves them by those
    /// names, under the local source the tests load through as under the one `zhang serve` runs: the local source read
    /// and wrote such a name in the working directory. A ledger loaded through a link to its directory, as `/tmp` is
    /// on macOS, too: its file list was empty, and each transaction recorded wrote the `include` of its month's file
    /// into the main file once more.
    #[tokio::test]
    async fn the_editor_reads_and_saves_the_files_it_lists() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap().join("ledger");
        std::fs::create_dir_all(&real).unwrap();
        let link = real.with_file_name("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        std::fs::write(real.join("main.bean"), LEDGER).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let loaded = Ledger::load(link.clone(), "main.bean".to_owned(), source).expect("load ledger");
        let state = State(SharedLedger(Arc::new(RwLock::new(loaded))));
        let (sender, _receiver) = mpsc::channel(8);
        let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));

        for _ in 0..2 {
            let (status, message) = answer(create_new_transaction(state.clone(), reload.clone(), Json(new_transaction())).await).await;
            assert_eq!(status, StatusCode::OK, "{message}");
        }
        let main = std::fs::read_to_string(real.join("main.bean")).unwrap();
        assert_eq!(main.matches("include \"data/2024/06.bean\"").count(), 1, "{main}");

        state.write().await.reload().unwrap();
        let listed = answer_json(get_files(state.clone()).await).await;
        assert_eq!(listed["data"], serde_json::json!(["main.bean", "data/2024/06.bean"]));
        for file in ["main.bean", "data/2024/06.bean"] {
            let (content, _) = shown(&state, Path::new(file)).await;
            assert_eq!(content, std::fs::read_to_string(real.join(file)).unwrap(), "{file}");
        }
        let edited = format!("{main}2024-01-01 open Assets:B\n");
        let request = FileUpdateRequest {
            content: edited.clone(),
            expected_sha256: None,
        };
        let saved = update_file_content(state.clone(), reload.clone(), Base64Path("main.bean".to_owned()), Json(request)).await;
        assert_eq!(answer(saved).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(real.join("main.bean")).unwrap(), edited);
    }

    /// The errors name their files as the file list does: by the path within the ledger, one helper for both
    /// (`path_in_ledger`), so the error dialog's "Open in Raw Editing" opens a file the editor lists (#669, #675). Also
    /// for a ledger loaded through a link to its directory, as `/tmp` is on macOS, and through a relative path.
    #[tokio::test]
    async fn the_errors_name_their_files_as_the_file_list_does() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap().join("ledger");
        std::fs::create_dir_all(real.join("data")).unwrap();
        let link = real.with_file_name("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // the same directory relative to the working directory, through the root
        let cwd = std::env::current_dir().unwrap();
        let relative = cwd
            .components()
            .skip(1)
            .map(|_| "..")
            .collect::<PathBuf>()
            .join(real.strip_prefix("/").unwrap());
        assert!(relative.is_relative());
        std::fs::write(
            real.join("main.bean"),
            format!("{LEDGER}include \"data/2024.bean\"\n2024-01-02 balance Assets:Gone 1 CNY\n"),
        )
        .unwrap();
        std::fs::write(real.join("data/2024.bean"), "2024-01-03 balance Assets:Lost 1 CNY\n").unwrap();

        for root in [real.clone(), link, relative] {
            let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
            let loaded = Ledger::load(root.clone(), "main.bean".to_owned(), source).expect("load ledger");
            let state = State(SharedLedger(Arc::new(RwLock::new(loaded))));
            let listed = answer_json(get_files(state.clone()).await).await;
            assert_eq!(listed["data"], serde_json::json!(["main.bean", "data/2024.bean"]), "{}", root.display());
            let request = JournalRequest {
                page: None,
                size: None,
                keyword: None,
                tags: None,
                links: None,
            };
            let errors = answer_json(get_errors(state.clone(), Query(request)).await).await;
            // an error in each file, named as the file list names it
            let files = errors["data"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| error["span"]["filename"].as_str().unwrap_or("no file").to_owned())
                .collect::<BTreeSet<_>>();
            let listed = listed["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| file.as_str().unwrap().to_owned())
                .collect::<BTreeSet<_>>();
            assert_eq!(files, listed, "{}: {}", root.display(), errors);
        }
    }

    async fn answer_json(response: impl IntoResponse) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_response().into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn answer(response: impl IntoResponse) -> (StatusCode, String) {
        let response = response.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let message = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|it| it["message"].as_str().map(str::to_owned))
            .unwrap_or_default();
        (status, message)
    }

    /// The editor shows a file starting with a UTF-8 byte order mark without the mark, and a save from it keeps the
    /// mark, once (#505); a file without one gets none.
    #[tokio::test]
    async fn the_editor_keeps_a_byte_order_mark_it_does_not_show() {
        let dir = std::env::temp_dir().join(format!("zhang-file-bom-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = dir.join("main.zhang");
        let opens = "1970-01-01 open Assets:Cash\n";
        std::fs::write(&main, format!("\u{feff}{opens}")).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(zhang_core::data_type::text::ZhangDataType {}));
        let loaded = Ledger::load(dir.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        let state = State(SharedLedger(Arc::new(RwLock::new(loaded))));
        let (sender, _receiver) = mpsc::channel(8);
        let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
        let path = || Base64Path(main.to_string_lossy().into_owned());
        let shown = || async {
            let response = get_file_content(state.clone(), path()).await.into_response();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["data"]["content"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let save = |content: String| {
            update_file_content(
                state.clone(),
                reload.clone(),
                path(),
                Json(FileUpdateRequest {
                    content,
                    expected_sha256: None,
                }),
            )
        };

        assert_eq!(shown().await, opens);
        let edited = format!("{opens}1970-01-01 open Assets:Bank\n");
        assert_eq!(answer(save(edited.clone()).await).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), format!("\u{feff}{edited}"));
        assert_eq!(shown().await, edited);

        std::fs::write(&main, opens).unwrap();
        assert_eq!(answer(save(edited.clone()).await).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), edited, "a file without a mark gets none");
        std::fs::remove_dir_all(dir).ok();
    }

    /// A save of a file that does not parse is written, and so is the save fixing it: a save edits no place the
    /// ledger loaded, so it never waits on the ledger loading. A write that edits the ledger as loaded is refused in
    /// between, with what to do, and written once the files are fixed.
    #[tokio::test]
    async fn a_file_that_does_not_parse_can_be_fixed_in_the_editor() {
        let dir = std::env::temp_dir().join(format!("zhang-file-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = dir.join("main.bean");
        let ledger = "option \"operating_currency\" \"CNY\"\n2020-01-01 commodity CNY\n2024-01-01 open Assets:A\n2024-01-01 open Income:X\n";
        std::fs::write(&main, ledger).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let loaded = Ledger::load(dir.clone(), "main.bean".to_owned(), source).expect("load ledger");
        let state = State(SharedLedger(Arc::new(RwLock::new(loaded))));
        let (sender, _receiver) = mpsc::channel(8);
        let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
        let save = |content: String| {
            let path = Base64Path(main.to_string_lossy().into_owned());
            update_file_content(
                state.clone(),
                reload.clone(),
                path,
                Json(FileUpdateRequest {
                    content,
                    expected_sha256: None,
                }),
            )
        };
        let create = || {
            let request: CreateTransactionRequest = serde_json::from_value(serde_json::json!({
                "datetime": "2024-06-01T12:00:00Z",
                "payee": "Shop",
                "flag": "Okay",
                "narration": "seed",
                "postings": [
                    {"account": "Assets:A", "unit": {"number": "50", "commodity": "CNY"}},
                    {"account": "Income:X", "unit": null}
                ],
                "metas": [],
                "tags": [],
                "links": []
            }))
            .unwrap();
            create_new_transaction(state.clone(), reload.clone(), Json(request))
        };

        let broken = format!("{ledger}this is not beancount\n");
        assert_eq!(answer(save(broken.clone()).await).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), broken);

        let (status, message) = answer(create().await).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(
            message.contains("the ledger cannot be loaded from its files as they are now, so nothing was written"),
            "{message}"
        );
        assert!(message.contains("Fix them in the file editor, then try again: cannot parse"), "{message}");
        assert!(message.contains("main.bean"), "{message}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), broken, "nothing is written");

        assert_eq!(answer(save(ledger.to_owned()).await).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), ledger);

        let (status, message) = answer(create().await).await;
        assert_eq!(status, StatusCode::OK, "{message}");
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let reloaded = Ledger::load(dir.clone(), "main.bean".to_owned(), source).expect("load ledger");
        assert!(reloaded.errors.is_empty(), "{:?}", reloaded.errors);
        assert_eq!(reloaded.transactions().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
