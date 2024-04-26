use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use bytes::Bytes;
use http2::{Request, Response, StatusCode};
use log::info;
use opendal::raw::oio::WriteBuf;
use opendal::raw::{
    new_request_build_error, oio, parse_content_length, parse_content_range, parse_etag, parse_into_metadata, Accessor, AccessorInfo, AsyncBody, HttpClient,
    IncomingAsyncBody, OpCreateDir, OpRead, OpStat, OpWrite, RpCreateDir, RpRead, RpStat, RpWrite,
};
use opendal::{Builder, Capability, Scheme};
use serde::Serialize;

#[derive(Debug, Default)]
pub struct GithubBuilder {
    pub user: String,
    pub repo: String,
    pub token: String,
}

impl Builder for GithubBuilder {
    const SCHEME: Scheme = Scheme::Custom("github");
    type Accessor = GithubAccessor;

    fn from_map(_map: HashMap<String, String>) -> Self {
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
            token: format!("Bearer {token}"),
        }
    }
}

#[derive(Debug)]
pub struct GithubAccessor {
    pub core: Arc<GithubCore>,
}

#[derive(Serialize)]
pub struct CreateOrUpdateFileRequest {
    message: String,
    content: String,
    sha: Option<String>,
}
#[async_trait::async_trait]
impl Accessor for GithubAccessor {
    type Reader = IncomingAsyncBody;
    type Writer = oio::OneShotWriter<GithubWriter>;
    type Lister = ();
    type BlockingReader = ();
    type BlockingWriter = ();
    type BlockingLister = ();

    fn info(&self) -> AccessorInfo {
        let mut ma = AccessorInfo::default();
        ma.set_scheme(Scheme::Custom("github")).set_native_capability(Capability {
            stat: true,

            read: true,

            write: true,

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

    async fn create_dir(&self, _path: &str, _args: OpCreateDir) -> opendal::Result<RpCreateDir> {
        Ok(RpCreateDir {})
    }
    async fn stat(&self, path: &str, args: OpStat) -> opendal::Result<RpStat> {
        let (_, _) = (path, args);

        let path = urlencoding::encode(path);
        let req = Request::get(format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            &self.core.user, &self.core.repo, &path
        ))
        .header("accept", "application/vnd.github+json")
        .header("Authorization", &self.core.token)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "zhang-server")
        .body(AsyncBody::Empty)
        .map_err(new_request_build_error)?;

        let resp = self.core.client.send(req).await?;
        let status = resp.status();
        match status {
            StatusCode::OK => parse_into_metadata(&path, resp.headers()).map(RpStat::new),
            StatusCode::NOT_FOUND => Err(opendal::Error::new(opendal::ErrorKind::NotFound, "not found")),
            StatusCode::FORBIDDEN => Err(opendal::Error::new(opendal::ErrorKind::PermissionDenied, "Forbidden")),
            _ => Err(opendal::Error::new(opendal::ErrorKind::Unexpected, "Unexpected")),
        }
    }

    async fn read(&self, path: &str, _args: OpRead) -> opendal::Result<(RpRead, Self::Reader)> {
        info!("read");
        let path = urlencoding::encode(path);
        let req = Request::get(format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            &self.core.user, &self.core.repo, &path
        ))
        .header("accept", "application/vnd.github.raw")
        .header("Authorization", &self.core.token)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "zhang-server")
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
}

pub struct GithubWriter {
    core: Arc<GithubCore>,
    path: String,
}

impl GithubWriter {
    pub fn new(core: Arc<GithubCore>, _op: OpWrite, path: String) -> Self {
        GithubWriter { core, path }
    }
}

#[async_trait]
impl oio::OneShotWrite for GithubWriter {
    async fn write_once(&self, bs: &dyn WriteBuf) -> opendal::Result<()> {
        let bytes = bs.bytes(bs.remaining());
        let encoded_content = general_purpose::STANDARD.encode(bytes);
        let sha = self.core.get_file_sha(&self.path).await?;
        let request_body = CreateOrUpdateFileRequest {
            message: "updated by zhang-server".to_string(),
            content: encoded_content,
            sha,
        };

        let path = urlencoding::encode(&self.path);
        let req = Request::put(format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            &self.core.user, &self.core.repo, &path
        ))
        .header("accept", "application/vnd.github+json")
        .header("authorization", &self.core.token)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "zhang-server")
        .body(AsyncBody::Bytes(Bytes::from(serde_json::to_vec(&request_body).unwrap())))
        .map_err(new_request_build_error)?;

        let resp = self.core.client.send(req).await?;

        let status = resp.status();

        match status {
            StatusCode::CREATED | StatusCode::OK | StatusCode::NO_CONTENT => {
                resp.into_body().consume().await?;
                Ok(())
            }
            _ => {
                let x = resp.into_body().bytes().await?;
                let cow = String::from_utf8_lossy(&x);
                Err(opendal::Error::new(opendal::ErrorKind::Unexpected, cow.as_ref()))
            }
        }
    }
}

impl GithubCore {
    pub async fn stat(&self, path: &str) -> opendal::Result<Response<IncomingAsyncBody>> {
        let path = urlencoding::encode(path);
        let req = Request::get(format!("https://api.github.com/repos/{}/{}/contents/{}", &self.user, &self.repo, &path))
            .header("accept", "application/vnd.github.raw")
            .header("Authorization", &self.token)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "zhang-server")
            .body(AsyncBody::Empty)
            .map_err(new_request_build_error)?;

        let resp = self.client.send(req).await?;
        Ok(resp)
    }
    pub async fn get_file_sha(&self, path: &str) -> Result<Option<String>, opendal::Error> {
        let resp = self.stat(path).await?;

        match resp.status() {
            StatusCode::OK => {
                let headers = resp.headers();

                let sha = parse_etag(headers)?;

                let Some(sha) = sha else {
                    return Err(opendal::Error::new(opendal::ErrorKind::Unexpected, "No ETag found in response headers"));
                };

                Ok(Some(sha.trim_matches('"').to_string()))
            }
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(opendal::Error::new(opendal::ErrorKind::Unexpected, "Unexpected")),
        }
    }
}
