//! JSON-RPC 2.0 as MCP uses it: one message per request body, string or number ids, the standard error codes
//! and the three MCP codes of the 2026-07-28 revision.

use serde_json::{json, Map, Value};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// 2026-07-28: the `MCP-Protocol-Version`, `Mcp-Method` or `Mcp-Name` header is missing or does not match the body
pub const HEADER_MISMATCH: i64 = -32020;
/// 2026-07-28: the requested protocol version is not one the server supports; `data.supported` lists them
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// a JSON-RPC error: the code, a message for people, and optional data
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> RpcError {
        RpcError::new(INVALID_PARAMS, message)
    }

    pub fn with_data(mut self, data: Value) -> RpcError {
        self.data = Some(data);
        self
    }

    /// the `error` member
    pub fn to_json(&self) -> Value {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some(data) = &self.data {
            error["data"] = data.clone();
        }
        error
    }
}

/// a request: it has an id, and the server answers it
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// a string or a number
    pub id: Value,
    pub method: String,
    /// an object, empty when the request has none
    pub params: Map<String, Value>,
}

/// one message a client posts
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request(Request),
    /// a notification (no id): accepted and ignored
    Notification {
        method: String,
    },
    /// a response to a server request (this server sends none): accepted and ignored
    Response,
}

/// Read one JSON-RPC message. Not JSON is [`PARSE_ERROR`]; a batch (an array), a message without
/// `"jsonrpc": "2.0"`, a `null` id, params that are not an object, or something that is neither a request, a
/// notification nor a response is [`INVALID_REQUEST`].
pub fn parse(body: &[u8]) -> Result<Message, RpcError> {
    let value: Value = serde_json::from_slice(body).map_err(|e| RpcError::new(PARSE_ERROR, format!("the body is not JSON: {e}")))?;
    let invalid = |message: &str| RpcError::new(INVALID_REQUEST, message);
    let object = match value {
        Value::Object(object) => object,
        Value::Array(_) => return Err(invalid("JSON-RPC batches are not supported: post one message per request")),
        _ => return Err(invalid("the body is not a JSON-RPC message object")),
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid("the message must have \"jsonrpc\": \"2.0\""));
    }
    let method = match object.get("method") {
        Some(Value::String(method)) => method.clone(),
        Some(_) => return Err(invalid("the method must be a string")),
        None if object.contains_key("result") || object.contains_key("error") => return Ok(Message::Response),
        None => return Err(invalid("the message is neither a request nor a notification: it has no method")),
    };
    let params = match object.get("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(params)) => params.clone(),
        Some(_) => return Err(invalid("params must be an object")),
    };
    match object.get("id") {
        None => Ok(Message::Notification { method }),
        Some(Value::Null) => Err(invalid("the id of a request must not be null")),
        Some(id @ (Value::String(_) | Value::Number(_))) => Ok(Message::Request(Request {
            id: id.clone(),
            method,
            params,
        })),
        Some(_) => Err(invalid("the id must be a string or a number")),
    }
}

/// the response carrying `result` to the request `id`
pub fn result_body(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// the error response to the request `id`, `null` when the request could not be read
pub fn error_body(id: Option<&Value>, error: &RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id.cloned().unwrap_or(Value::Null), "error": error.to_json()})
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::{error_body, parse, result_body, Message, Request, RpcError, INVALID_REQUEST, PARSE_ERROR};

    fn code_of(body: &str) -> i64 {
        parse(body.as_bytes()).unwrap_err().code
    }

    #[test]
    fn reads_requests_notifications_and_responses() {
        let request = parse(br#"{"jsonrpc":"2.0","id":"a","method":"tools/list","params":{"cursor":null}}"#).unwrap();
        assert_eq!(
            request,
            Message::Request(Request {
                id: json!("a"),
                method: "tools/list".to_owned(),
                params: json!({"cursor": null}).as_object().unwrap().clone(),
            })
        );
        let numbered = parse(br#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).unwrap();
        assert!(matches!(numbered, Message::Request(Request { id, .. }) if id == json!(7)));
        assert_eq!(
            parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap(),
            Message::Notification {
                method: "notifications/initialized".to_owned()
            }
        );
        assert_eq!(parse(br#"{"jsonrpc":"2.0","id":3,"result":{}}"#).unwrap(), Message::Response);
        assert_eq!(
            parse(br#"{"jsonrpc":"2.0","id":3,"error":{"code":1,"message":"x"}}"#).unwrap(),
            Message::Response
        );
    }

    #[test]
    fn rejects_what_is_not_one_message() {
        assert_eq!(code_of("{not json"), PARSE_ERROR);
        assert_eq!(code_of(""), PARSE_ERROR);
        assert_eq!(code_of(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#), INVALID_REQUEST);
        assert_eq!(code_of(r#""ping""#), INVALID_REQUEST);
        assert_eq!(code_of(r#"{"id":1,"method":"ping"}"#), INVALID_REQUEST);
        assert_eq!(code_of(r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#), INVALID_REQUEST);
        assert_eq!(code_of(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#), INVALID_REQUEST);
        assert_eq!(code_of(r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#), INVALID_REQUEST);
        assert_eq!(code_of(r#"{"jsonrpc":"2.0","id":1,"method":7}"#), INVALID_REQUEST);
        assert_eq!(code_of(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":[1]}"#), INVALID_REQUEST);
        assert_eq!(code_of(r#"{"jsonrpc":"2.0","id":1}"#), INVALID_REQUEST);
    }

    #[test]
    fn writes_results_and_errors() {
        assert_eq!(result_body(&json!(1), json!({"a": 1})), json!({"jsonrpc": "2.0", "id": 1, "result": {"a": 1}}));
        let error = RpcError::new(-32022, "nope").with_data(json!({"supported": ["2026-07-28"]}));
        assert_eq!(
            error_body(Some(&json!("x")), &error),
            json!({"jsonrpc": "2.0", "id": "x", "error": {"code": -32022, "message": "nope", "data": {"supported": ["2026-07-28"]}}})
        );
        assert_eq!(
            error_body(None, &RpcError::new(-32700, "bad")),
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "bad"}})
        );
    }
}
