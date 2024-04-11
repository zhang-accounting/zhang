use http::{HeaderMap, HeaderValue, Method, Uri};

pub struct PluginRequest<T> {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap<HeaderValue>,
    pub body: T,
}

pub struct PluginResponse<T> {
    pub status: u16,
    pub body: T,
}
