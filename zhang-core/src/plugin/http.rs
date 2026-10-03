//! The HTTP request a router plugin handles and the response it returns, as they cross the WASM
//! boundary. Both are JSON and part of the plugin ABI: a field is never removed or made required,
//! and a plugin ignores fields it does not know. See [`crate::plugin::router`] for how requests
//! reach a plugin.

use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use http::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING};
use http::{HeaderName, HeaderValue, StatusCode};
use serde::{Deserialize, Deserializer, Serialize};

/// request headers carrying the user's credentials for zhang itself, or for a proxy in front of it.
/// A plugin sits behind zhang's authentication and never needs them, and one allowed to reach the
/// network could leak them, so they are never passed on.
pub const WITHHELD_REQUEST_HEADERS: [&str; 3] = ["authorization", "proxy-authorization", "cookie"];

/// the content type of a router response that does not set one
pub const DEFAULT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// response headers describing how the body is framed, which the host sets itself
const FRAMING_RESPONSE_HEADERS: [HeaderName; 3] = [CONTENT_LENGTH, TRANSFER_ENCODING, CONNECTION];

/// how a body travels in a JSON string
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyEncoding {
    /// the body is the string itself
    #[default]
    Utf8,
    /// the body is the standard base64 (with padding) of the string, for bytes that are not UTF-8
    Base64,
}

/// the request a router plugin's `router` export receives, as JSON:
///
/// ```json
/// {
///   "method": "GET",
///   "path": "/sub/path",
///   "query": {"k": ["v1", "v2"]},
///   "headers": {"accept": "text/html"},
///   "body": "",
///   "body_encoding": "utf8"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRequest {
    /// the HTTP method, upper case, e.g. `GET` or `POST`
    pub method: String,
    /// the path below the plugin's route, starting with `/`, with its percent-encoding as sent:
    /// `/api/plugins/{name}/by-month` gives `/by-month`, and `/api/plugins/{name}` gives `/`
    pub path: String,
    /// the decoded query string: every key with its values, in the order they were sent
    pub query: BTreeMap<String, Vec<String>>,
    /// the request headers, names lower case; the values of a repeated header are joined with `, `.
    /// The credential headers [`WITHHELD_REQUEST_HEADERS`] are left out
    pub headers: BTreeMap<String, String>,
    /// the request body, encoded as `body_encoding` says; empty when there is none
    pub body: String,
    /// `utf8` when the body is valid UTF-8, else `base64`
    pub body_encoding: BodyEncoding,
}

impl PluginRequest {
    /// build a request: query pairs are grouped by key, header names lower-cased, credential headers
    /// left out, and the body carried as UTF-8 when it is valid UTF-8 and as base64 otherwise
    pub fn new(
        method: impl Into<String>, path: impl Into<String>, query: impl IntoIterator<Item = (String, String)>,
        headers: impl IntoIterator<Item = (String, String)>, body: Vec<u8>,
    ) -> PluginRequest {
        let mut grouped_query: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, value) in query {
            grouped_query.entry(key).or_default().push(value);
        }
        let mut grouped_headers: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in headers {
            let name = name.to_ascii_lowercase();
            if WITHHELD_REQUEST_HEADERS.contains(&name.as_str()) {
                continue;
            }
            grouped_headers
                .entry(name)
                .and_modify(|joined| {
                    joined.push_str(", ");
                    joined.push_str(&value);
                })
                .or_insert(value);
        }
        let (body, body_encoding) = match String::from_utf8(body) {
            Ok(text) => (text, BodyEncoding::Utf8),
            Err(e) => (BASE64.encode(e.into_bytes()), BodyEncoding::Base64),
        };
        PluginRequest {
            method: method.into(),
            path: path.into(),
            query: grouped_query,
            headers: grouped_headers,
            body,
            body_encoding,
        }
    }
}

/// what a router plugin's `router` export returns, as JSON. Every field is optional, and `null`
/// counts as absent:
///
/// ```json
/// {
///   "status": 200,
///   "headers": {"content-type": "text/html; charset=utf-8"},
///   "body": "<h1>Report</h1>",
///   "body_encoding": "utf8"
/// }
/// ```
///
/// Any content type works, so a router can answer JSON for a script or a whole HTML page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginResponse {
    /// the HTTP status, 200 when absent
    #[serde(default = "default_status", deserialize_with = "null_as_status")]
    pub status: u16,
    /// response headers, one value per name. Without a `content-type` the response is
    /// `text/plain; charset=utf-8`. The host frames the body itself, so `content-length`,
    /// `transfer-encoding` and `connection` are ignored
    #[serde(default, deserialize_with = "null_as_default")]
    pub headers: BTreeMap<String, String>,
    /// the response body, encoded as `body_encoding` says; empty when absent
    #[serde(default, deserialize_with = "null_as_default")]
    pub body: String,
    /// `utf8` (when absent) or `base64`, for a binary body such as an image
    #[serde(default, deserialize_with = "null_as_default")]
    pub body_encoding: BodyEncoding,
}

impl Default for PluginResponse {
    fn default() -> Self {
        PluginResponse {
            status: default_status(),
            headers: BTreeMap::new(),
            body: String::new(),
            body_encoding: BodyEncoding::Utf8,
        }
    }
}

impl PluginResponse {
    /// read the JSON a plugin returned; an error says why it is not a response. Unlike plain serde,
    /// this accepts only a JSON object
    pub fn from_json(output: &[u8]) -> Result<PluginResponse, String> {
        let value: serde_json::Value = serde_json::from_slice(output).map_err(|e| e.to_string())?;
        if !value.is_object() {
            return Err("the response is not a JSON object".to_owned());
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    /// the body as bytes; an error when it claims to be base64 but is not
    pub fn body_bytes(&self) -> Result<Vec<u8>, base64::DecodeError> {
        match self.body_encoding {
            BodyEncoding::Utf8 => Ok(self.body.clone().into_bytes()),
            BodyEncoding::Base64 => BASE64.decode(&self.body),
        }
    }

    /// the HTTP response to send: [`DEFAULT_CONTENT_TYPE`] when the plugin sets no content type,
    /// and without the framing headers the host sets itself. An error says why the plugin's
    /// response cannot be sent: a status out of range, a header that is not valid HTTP, or a body
    /// that is not the base64 it claims to be.
    pub fn into_http(self) -> Result<http::Response<Vec<u8>>, String> {
        let status = StatusCode::from_u16(self.status).map_err(|_| format!("{} is not an HTTP status", self.status))?;
        let body = self.body_bytes().map_err(|e| format!("the body is not valid base64: {e}"))?;
        let mut response = http::Response::new(body);
        *response.status_mut() = status;
        for (name, value) in &self.headers {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| format!("{name:?} is not a valid header name"))?;
            if FRAMING_RESPONSE_HEADERS.contains(&name) {
                continue;
            }
            let value = HeaderValue::from_str(value).map_err(|_| format!("the value of the header {name} is not a valid header value"))?;
            response.headers_mut().insert(name, value);
        }
        response
            .headers_mut()
            .entry(CONTENT_TYPE)
            .or_insert(HeaderValue::from_static(DEFAULT_CONTENT_TYPE));
        Ok(response)
    }
}

fn default_status() -> u16 {
    200
}

fn null_as_status<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
    Ok(Option::<u16>::deserialize(deserializer)?.unwrap_or_else(default_status))
}

fn null_as_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(deserializer: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use http::header::CONTENT_TYPE;
    use http::StatusCode;
    use serde_json::json;

    use super::{BodyEncoding, PluginRequest, PluginResponse};

    fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn should_group_the_query_and_lower_case_the_headers() {
        let request = PluginRequest::new(
            "POST",
            "/sub/path",
            pairs(&[("k", "v1"), ("other", ""), ("k", "v2")]),
            pairs(&[
                ("Accept", "text/html"),
                ("X-Tag", "a"),
                ("Authorization", "Basic dXNlcjpwYXNz"),
                ("x-tag", "b"),
                ("Cookie", "session=secret"),
                ("proxy-authorization", "Bearer secret"),
            ]),
            b"{\"a\": 1}".to_vec(),
        );

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "method": "POST",
                "path": "/sub/path",
                "query": {"k": ["v1", "v2"], "other": [""]},
                "headers": {"accept": "text/html", "x-tag": "a, b"},
                "body": "{\"a\": 1}",
                "body_encoding": "utf8",
            })
        );
    }

    #[test]
    fn should_carry_a_body_that_is_not_utf8_as_base64() {
        let request = PluginRequest::new("PUT", "/", vec![], vec![], vec![0xff, 0x00, 0x41]);

        assert_eq!(request.body, "/wBB");
        assert_eq!(request.body_encoding, BodyEncoding::Base64);
    }

    #[test]
    fn should_default_every_response_field() {
        for empty in [json!({}), json!({"status": null, "headers": null, "body": null, "body_encoding": null})] {
            assert_eq!(serde_json::from_value::<PluginResponse>(empty).unwrap(), PluginResponse::default());
        }
        assert_eq!(PluginResponse::default().status, 200);
    }

    #[test]
    fn should_decode_a_base64_response_body() {
        let response: PluginResponse = serde_json::from_value(json!({
            "status": 201,
            "headers": {"content-type": "image/png"},
            "body": "/wBB",
            "body_encoding": "base64",
            "a field from a newer plugin": true,
        }))
        .unwrap();

        assert_eq!(response.status, 201);
        assert_eq!(response.headers, BTreeMap::from([("content-type".to_owned(), "image/png".to_owned())]));
        assert_eq!(response.body_bytes().unwrap(), vec![0xff, 0x00, 0x41]);

        let broken = PluginResponse {
            body: "not base64!".to_owned(),
            body_encoding: BodyEncoding::Base64,
            ..PluginResponse::default()
        };
        assert!(broken.body_bytes().is_err());
    }

    #[test]
    fn should_reject_a_response_that_is_not_the_shape() {
        let cases = [
            ("not json", "expected ident at line 1 column 2"),
            ("[]", "the response is not a JSON object"),
            ("null", "the response is not a JSON object"),
            (r#"{"status": "200"}"#, "invalid type: string \"200\", expected u16"),
            (r#"{"status": 70000}"#, "invalid value: integer `70000`, expected u16"),
            (r#"{"body_encoding": "hex"}"#, "unknown variant `hex`, expected `utf8` or `base64`"),
        ];
        for (output, message) in cases {
            assert_eq!(PluginResponse::from_json(output.as_bytes()).unwrap_err(), message, "{output}");
        }
        assert_eq!(PluginResponse::from_json(b"{}").unwrap(), PluginResponse::default());
    }

    #[test]
    fn should_send_the_response_with_a_default_content_type() {
        let response = PluginResponse {
            status: 404,
            body: "missing".to_owned(),
            ..PluginResponse::default()
        }
        .into_http()
        .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/plain; charset=utf-8");
        assert_eq!(response.body(), b"missing");
    }

    #[test]
    fn should_send_the_plugin_headers_but_not_the_framing_ones() {
        let response: PluginResponse = serde_json::from_value(json!({
            "headers": {"Content-Type": "text/html", "x-report": "monthly", "content-length": "999", "transfer-encoding": "chunked"},
            "body": "<h1>hi</h1>",
        }))
        .unwrap();

        let response = response.into_http().unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(headers, vec!["content-type: text/html", "x-report: monthly"]);
    }

    #[test]
    fn should_refuse_a_response_that_is_not_valid_http() {
        let cases = [
            (json!({"status": 42}), "42 is not an HTTP status"),
            (json!({"headers": {"bad name": "x"}}), "\"bad name\" is not a valid header name"),
            (
                json!({"headers": {"x-bad": "line\nbreak"}}),
                "the value of the header x-bad is not a valid header value",
            ),
            (
                json!({"body": "%%%", "body_encoding": "base64"}),
                "the body is not valid base64: Invalid symbol 37, offset 0.",
            ),
        ];
        for (response, message) in cases {
            let response: PluginResponse = serde_json::from_value(response).unwrap();
            assert_eq!(response.into_http().unwrap_err(), message);
        }
    }
}
