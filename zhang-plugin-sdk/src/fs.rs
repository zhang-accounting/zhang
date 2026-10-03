//! Read-only access to the ledger's files, granted by the `allowed_paths` meta of the plugin's directive.
//!
//! ```zhang
//! plugin "receipts.wasm"
//!   allowed_paths: "documents"
//! ```
//!
//! Paths are relative to the ledger root and written with `/` (`"."` is the root itself). A path outside every
//! grant, absolute, or holding `..` is denied; below a grant, a hidden name (starting with `.`) is readable only
//! when a grant names it. Only a processor or mapper reads files: while the plugin registers or handles a request
//! as a router, every path is denied. Every failure is an ordinary [`HostError`], so a missing file never stops
//! the load. Each file or directory a plugin reads is recorded, and a server reloads the ledger when it changes.
//!
//! The host functions behind this module, `zhang_read_file` and `zhang_list_dir`, work on every ledger source
//! (local disk, S3, WebDAV, GitHub), since zhang reads the files, not the plugin.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Deserialize;

use crate::abi;
use crate::error::{host_result, HostError, HostErrorKind};

/// an entry of a listed directory
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DirEntry {
    /// the entry's name, without its directory
    pub name: String,
    pub kind: EntryKind,
}

/// what a listed entry is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum EntryKind {
    File,
    Dir,
    /// a kind this SDK does not know, from a newer zhang
    #[serde(other)]
    Other,
}

/// how the host encodes a file's content in JSON
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Encoding {
    #[default]
    Utf8,
    Base64,
}

/// what `zhang_read_file` answers inside `Ok`
#[derive(Debug, Deserialize)]
struct FileContent {
    content: String,
    #[serde(default)]
    encoding: Encoding,
}

/// what `zhang_list_dir` answers inside `Ok`
#[derive(Debug, Deserialize)]
struct DirListing {
    entries: Vec<DirEntry>,
}

impl FileContent {
    fn into_bytes(self) -> Result<Vec<u8>, HostError> {
        match self.encoding {
            Encoding::Utf8 => Ok(self.content.into_bytes()),
            Encoding::Base64 => BASE64
                .decode(self.content)
                .map_err(|e| HostError::new(HostErrorKind::Other, format!("zhang_read_file answered content that is not base64: {e}"))),
        }
    }
}

/// the bytes of the file at `path`, relative to the ledger root
pub fn read_file(path: &str) -> Result<Vec<u8>, HostError> {
    file_bytes(&abi::read_file(path)?)
}

/// the file at `path`, relative to the ledger root, as text; [`HostErrorKind::Invalid`] when it is not UTF-8
pub fn read_to_string(path: &str) -> Result<String, HostError> {
    let bytes = read_file(path)?;
    String::from_utf8(bytes).map_err(|_| HostError::new(HostErrorKind::Invalid, format!("{path}: the file is not UTF-8 text")))
}

/// the entries of the directory at `path`, relative to the ledger root, sorted by name. Entries a plugin may not
/// read, such as hidden ones below a grant, are left out
pub fn list_dir(path: &str) -> Result<Vec<DirEntry>, HostError> {
    let listing: DirListing = host_result(abi::LIST_DIR, &abi::list_dir(path)?)?;
    Ok(listing.entries)
}

fn file_bytes(answer: &[u8]) -> Result<Vec<u8>, HostError> {
    host_result::<FileContent>(abi::READ_FILE, answer)?.into_bytes()
}

#[cfg(test)]
mod test {
    use super::{file_bytes, DirEntry, DirListing, EntryKind};
    use crate::error::{host_result, HostErrorKind};

    #[test]
    fn should_decode_text_and_base64_files() {
        assert_eq!(file_bytes(br#"{"Ok": {"content": "a,b\n", "encoding": "utf8"}}"#).unwrap(), b"a,b\n");
        assert_eq!(
            file_bytes(br#"{"Ok": {"content": "/wD+", "encoding": "base64"}}"#).unwrap(),
            vec![0xff, 0x00, 0xfe]
        );
        assert_eq!(
            file_bytes(br#"{"Ok": {"content": "%%", "encoding": "base64"}}"#).unwrap_err().kind,
            HostErrorKind::Other
        );
        let denied = file_bytes(br#"{"Err": {"kind": "denied", "message": "secret.txt: not granted"}}"#).unwrap_err();
        assert_eq!((denied.kind, denied.message.as_str()), (HostErrorKind::Denied, "secret.txt: not granted"));
    }

    #[test]
    fn should_read_a_listing() {
        let listing: DirListing = host_result(
            "zhang_list_dir",
            br#"{"Ok": {"entries": [{"name": "2024", "kind": "dir"}, {"name": "a.pdf", "kind": "file"}, {"name": "x", "kind": "socket"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            listing.entries,
            vec![
                DirEntry {
                    name: "2024".to_owned(),
                    kind: EntryKind::Dir
                },
                DirEntry {
                    name: "a.pdf".to_owned(),
                    kind: EntryKind::File
                },
                DirEntry {
                    name: "x".to_owned(),
                    kind: EntryKind::Other
                },
            ]
        );
        assert_eq!(super::list_dir(".").unwrap_err().kind, HostErrorKind::Unavailable);
    }
}
