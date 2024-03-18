use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, StatusCode};
use opendal::raw::oio::WriteBuf;
use opendal::raw::{
    new_request_build_error, oio, parse_content_length, parse_content_range, Accessor, AccessorInfo, AsyncBody, HttpClient, IncomingAsyncBody, OpCopy,
    OpCreateDir, OpDelete, OpList, OpRead, OpRename, OpStat, OpWrite, RpCopy, RpCreateDir, RpDelete, RpList, RpRead, RpRename, RpStat, RpWrite,
};
use opendal::{Builder, Capability, EntryMode, Metadata, Scheme};

#[derive(Debug, Default)]
pub struct GithubBuilder {
    pub user: String,
    pub repo: String,
    pub token: String,
}

impl Builder for GithubBuilder {
    const SCHEME: Scheme = Scheme::Custom("github");
    type Accessor = GithubAccessor;

    fn from_map(map: HashMap<String, String>) -> Self {
        todo!()
    }

    fn build(&mut self) -> opendal::Result<Self::Accessor> {
        Ok(GithubAccessor {
            core: Arc::new(GithubCore::new(self.user.clone(), self.repo.clone(), self.token.clone())),
        })
    }
}
#[derive(Debug)]
pub struct GithubCore {
    pub client: HttpClient,
    pub user: String,
    pub repo: String,
    pub token: String,
}

impl GithubCore {
    pub fn new(user: String, repo: String, token: String) -> Self {
        Self {
            client: HttpClient::new()
                .map_err(|err| err.with_operation("Builder::build").with_context("service", "github"))
                .unwrap(),
            user,
            repo,
            token,
        }
    }
}

#[derive(Debug)]
pub struct GithubAccessor {
    pub core: Arc<GithubCore>,
}

pub struct CreateOrUpdateFileRequest {
    message: String,
    content: String,
}
#[async_trait::async_trait]
impl Accessor for GithubAccessor {
    type Reader = IncomingAsyncBody;
    type BlockingReader = ();
    type Writer = oio::OneShotWriter<GithubWriter>;
    type BlockingWriter = ();
    type Lister = ();
    type BlockingLister = ();

    fn info(&self) -> AccessorInfo {
        let mut ma = AccessorInfo::default();
        ma.set_scheme(Scheme::Custom("github")).set_native_capability(Capability {
            stat: true,

            read: true,
            read_can_next: false,
            read_with_range: false,

            write: true,
            write_can_empty: true,

            create_dir: true,
            delete: false,

            copy: false,

            rename: false,

            list: true,
            // We already support recursive list but some details still need to polish.
            // list_with_recursive: true,
            ..Default::default()
        });

        ma
    }

    async fn stat(&self, path: &str, args: OpStat) -> opendal::Result<RpStat> {
        let (_, _) = (path, args);

        let path = urlencoding::encode(path);
        let req = Request::get(format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            &self.core.user, &self.core.repo, &path
        ))
        .header("accept", "application/vnd.github+json")
        .header("authorization", &self.core.token)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .body(AsyncBody::Empty)
        .map_err(new_request_build_error)?;

        let resp = self.core.client.send(req).await?;
        let status = resp.status();
        match status {
            StatusCode::OK => {
                let x = resp.into_body().bytes().await?;
                let response_body = String::from_utf8_lossy(&x);
                if response_body.starts_with('[') {
                    Ok(RpStat::new(Metadata::new(EntryMode::DIR)))
                } else {
                    Ok(RpStat::new(Metadata::new(EntryMode::FILE)))
                }
            }
            StatusCode::NOT_FOUND => Err(opendal::Error::new(opendal::ErrorKind::NotFound, "not found")),
            StatusCode::FORBIDDEN => Err(opendal::Error::new(opendal::ErrorKind::PermissionDenied, "Forbidden")),
            _ => Err(opendal::Error::new(opendal::ErrorKind::Unexpected, "Unexpected")),
        }
    }
    async fn create_dir(&self, path: &str, args: OpCreateDir) -> opendal::Result<RpCreateDir> {
        Ok(RpCreateDir {})
    }

    async fn read(&self, path: &str, args: OpRead) -> opendal::Result<(RpRead, Self::Reader)> {
        let path = urlencoding::encode(path);
        let req = Request::get(format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            &self.core.user, &self.core.repo, &path
        ))
        .header("accept", "application/vnd.github.raw")
        .header("authorization", &self.core.token)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .body(AsyncBody::Empty)
        .map_err(new_request_build_error)?;

        let resp = self.core.client.send(req).await?;
        let status = resp.status();
        match status {
            StatusCode::OK => {
                let size = parse_content_length(resp.headers())?;
                let range = parse_content_range(resp.headers())?;
                Ok((RpRead::new().with_size(size).with_range(range), resp.into_body()))
            }
            StatusCode::NOT_FOUND => Err(opendal::Error::new(opendal::ErrorKind::NotFound, "not found")),
            StatusCode::FORBIDDEN => Err(opendal::Error::new(opendal::ErrorKind::PermissionDenied, "Forbidden")),
            _ => Err(opendal::Error::new(opendal::ErrorKind::Unexpected, "Unexpected")),
        }
    }

    async fn write(&self, path: &str, args: OpWrite) -> opendal::Result<(RpWrite, Self::Writer)> {
        Ok((
            RpWrite::default(),
            oio::OneShotWriter::new(GithubWriter::new(self.core.clone(), args, path.to_string())),
        ))
    }

    async fn copy(&self, from: &str, to: &str, args: OpCopy) -> opendal::Result<RpCopy> {
        todo!()
    }

    async fn rename(&self, from: &str, to: &str, args: OpRename) -> opendal::Result<RpRename> {
        todo!()
    }

    async fn delete(&self, path: &str, args: OpDelete) -> opendal::Result<RpDelete> {
        todo!()
    }

    async fn list(&self, path: &str, args: OpList) -> opendal::Result<(RpList, Self::Lister)> {
        todo!()
    }
}

pub struct GithubWriter {
    core: Arc<GithubCore>,

    op: OpWrite,
    path: String,
}

impl GithubWriter {
    pub fn new(core: Arc<GithubCore>, op: OpWrite, path: String) -> Self {
        GithubWriter { core, op, path }
    }
}

#[async_trait]
impl oio::OneShotWrite for GithubWriter {
    async fn write_once(&self, bs: &dyn WriteBuf) -> opendal::Result<()> {
        let bytes = bs.bytes(bs.remaining());
        let encoded_content = base64::encode(bytes);

        let request_body = format!(r#"{{"message":"updated by zhang", "contnt": "{}"}}"#, encoded_content);

        let path = urlencoding::encode(&self.path);
        let req = Request::put(format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            &self.core.user, &self.core.repo, &path
        ))
        .header("accept", "application/vnd.github+json")
        .header("authorization", &self.core.token)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .body(AsyncBody::Bytes(Bytes::from(request_body)))
        .map_err(new_request_build_error)?;

        let resp = self.core.client.send(req).await?;

        let status = resp.status();

        match status {
            StatusCode::CREATED | StatusCode::OK | StatusCode::NO_CONTENT => {
                resp.into_body().consume().await?;
                Ok(())
            }
            _ => Err(opendal::Error::new(opendal::ErrorKind::Unexpected, "Unexpected")),
        }
    }
}
