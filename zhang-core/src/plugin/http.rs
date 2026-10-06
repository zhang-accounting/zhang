//! The HTTP request a router plugin handles and the response it returns, as they cross the WASM
//! boundary. Both are JSON and part of the plugin ABI, defined in [`zhang_shared::plugin_abi`] for the
//! host and the plugin SDK alike: a field is never removed or made required, and a plugin ignores
//! fields it does not know. See [`crate::plugin::router`] for how requests reach a plugin.

pub use zhang_shared::plugin_abi::{
    Encoding as BodyEncoding, Request as PluginRequest, Response as PluginResponse, DEFAULT_CONTENT_TYPE, WITHHELD_REQUEST_HEADERS,
};

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
