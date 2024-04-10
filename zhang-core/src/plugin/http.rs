use http::request::Parts;
use http::{HeaderMap, HeaderValue, Response, StatusCode};
use http::{Method, Request, Uri};

pub struct PluginRequest<T> {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap<HeaderValue>,
    body: T,
}

pub struct PluginResponse<T> {
    pub status: u16,
    pub body: T,
}
