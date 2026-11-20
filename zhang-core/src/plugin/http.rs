//! The HTTP request a router plugin handles and the response it returns, as they cross the WASM
//! boundary. Both are JSON and part of the plugin ABI, defined in [`zhang_shared::plugin_abi`] for the
//! host and the plugin SDK alike: a field is never removed or made required, and a plugin ignores
//! fields it does not know. See [`crate::plugin::router`] for how requests reach a plugin.

pub use zhang_shared::plugin_abi::{
    same_origin_location, Encoding as BodyEncoding, Request as PluginRequest, Response as PluginResponse, DEFAULT_CONTENT_TYPE, FORWARDED_REQUEST_HEADERS,
    FORWARDED_RESPONSE_HEADERS, SECURITY_HEADERS,
};

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use http::header::CONTENT_TYPE;
    use http::StatusCode;
    use serde_json::json;

    use super::{same_origin_location, BodyEncoding, PluginRequest, PluginResponse, FORWARDED_REQUEST_HEADERS};

    fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn should_group_the_query_and_lower_case_the_headers() {
        let request = PluginRequest::new(
            "POST",
            "/sub/path",
            pairs(&[("k", "v1"), ("other", ""), ("k", "v2")]),
            pairs(&[("Accept", "text/html"), ("Accept-Language", "en"), ("accept", "application/json")]),
            b"{\"a\": 1}".to_vec(),
        );

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "method": "POST",
                "path": "/sub/path",
                "query": {"k": ["v1", "v2"], "other": [""]},
                "headers": {"accept": "text/html, application/json", "accept-language": "en"},
                "body": "{\"a\": 1}",
                "body_encoding": "utf8",
            })
        );
    }

    #[test]
    fn should_pass_only_the_allowlisted_request_headers() {
        let allowed = FORWARDED_REQUEST_HEADERS.map(|name| (name.to_ascii_uppercase(), format!("value of {name}")));
        let withheld = [
            // zhang's own credentials
            "authorization",
            "proxy-authorization",
            "cookie",
            // the identity headers of proxies in front of zhang
            "cf-access-jwt-assertion",
            "cf-access-authenticated-user-email",
            "x-auth-request-user",
            "x-auth-request-email",
            "x-auth-request-access-token",
            "tailscale-user-login",
            "tailscale-user-name",
            "x-forwarded-for",
            "x-forwarded-user",
            "x-forwarded-email",
            "x-real-ip",
            "remote-user",
            // anything else
            "x-custom",
            "referer",
        ]
        .map(|name| (name.to_owned(), "secret".to_owned()));

        let request = PluginRequest::new("GET", "/", vec![], allowed.iter().cloned().chain(withheld), vec![]);

        let expected: BTreeMap<String, String> = FORWARDED_REQUEST_HEADERS
            .iter()
            .map(|name| (name.to_string(), format!("value of {name}")))
            .collect();
        assert_eq!(request.headers, expected);
        for name in [
            "origin",
            "host",
            "mcp-session-id",
            "mcp-protocol-version",
            "mcp-method",
            "mcp-name",
            "last-event-id",
        ] {
            assert!(request.headers.contains_key(name), "the MCP transport needs {name}");
        }
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

    /// the headers of `response`, as `name: value`, in order
    fn header_lines(response: &http::Response<Vec<u8>>) -> Vec<String> {
        response
            .headers()
            .iter()
            .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap()))
            .collect()
    }

    #[test]
    fn should_send_the_allowlisted_headers_and_the_security_headers_only() {
        let response: PluginResponse = serde_json::from_value(json!({
            "headers": {
                "Content-Type": "text/html",
                "cache-control": "no-store",
                "content-disposition": "attachment; filename=\"report.csv\"",
                "content-language": "en",
                "etag": "\"v1\"",
                "last-modified": "Wed, 21 Oct 2015 07:28:00 GMT",
                "vary": "accept",
                "location": "/api/plugins/report/by-month",
                // all of these are dropped
                "set-cookie": "zhang_session=evil; Path=/",
                "clear-site-data": "\"*\"",
                "content-security-policy": "default-src *; report-uri https://evil.example/",
                "content-security-policy-report-only": "default-src 'none'",
                "access-control-allow-origin": "https://evil.example",
                "access-control-allow-credentials": "true",
                "refresh": "0; url=https://evil.example/",
                "link": "<https://evil.example/>; rel=preload",
                "x-report": "monthly",
                "x-content-type-options": "sniff",
                "content-length": "999",
                "transfer-encoding": "chunked",
                "connection": "close",
            },
            "body": "<h1>hi</h1>",
        }))
        .unwrap();

        let response = response.into_http().unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header_lines(&response),
            vec![
                // `Content-Type` sorts first, as the plugin wrote it
                "content-type: text/html",
                "cache-control: no-store",
                "content-disposition: attachment; filename=\"report.csv\"",
                "content-language: en",
                "etag: \"v1\"",
                "last-modified: Wed, 21 Oct 2015 07:28:00 GMT",
                "location: /api/plugins/report/by-month",
                "vary: accept",
                "content-security-policy: sandbox allow-scripts allow-forms allow-popups allow-downloads",
                "x-content-type-options: nosniff",
            ]
        );
        assert_eq!(response.body(), b"<h1>hi</h1>");
    }

    #[test]
    fn should_sandbox_every_response_whatever_the_plugin_returns() {
        for response in [
            PluginResponse::default(),
            PluginResponse::html("<script>fetch('/api/files')</script>"),
            PluginResponse::text("plain").with_status(404),
            PluginResponse::bytes("image/svg+xml", b"<svg onload=\"alert(1)\"/>"),
            PluginResponse::default().with_header("content-security-policy", "sandbox allow-same-origin allow-scripts"),
        ] {
            let response = response.into_http().unwrap();
            let csp = response.headers().get_all(http::header::CONTENT_SECURITY_POLICY).iter().collect::<Vec<_>>();
            assert_eq!(csp, vec!["sandbox allow-scripts allow-forms allow-popups allow-downloads"]);
            assert!(!csp[0].to_str().unwrap().contains("allow-same-origin"));
            assert_eq!(response.headers()[http::header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            assert!(response.headers().get(http::header::SET_COOKIE).is_none());
        }
    }

    #[test]
    fn should_send_a_location_only_when_it_stays_on_zhangs_origin() {
        for location in [
            "/api/plugins/report/by-month",
            "by-month",
            "./by-month?year=2024",
            "?year=2024",
            "#top",
            "/a%2F%2Fb",
            "/x:y",
        ] {
            assert!(same_origin_location(location), "{location:?}");
            let response = PluginResponse::default()
                .with_status(302)
                .with_header("location", location)
                .into_http()
                .unwrap();
            assert_eq!(response.headers()[http::header::LOCATION], location, "{location:?}");
        }
        for location in [
            "https://evil.example/",
            "HTTP://evil.example/",
            "//evil.example/",
            "/\\evil.example/",
            "\\\\evil.example/",
            "/\t/evil.example/",
            "/\n/evil.example/",
            " //evil.example/",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "",
        ] {
            assert!(!same_origin_location(location), "{location:?}");
            let response = PluginResponse::default().with_status(302).with_header("location", location).into_http();
            // a header value with a control character is not valid HTTP; any other is dropped
            if let Ok(response) = response {
                assert!(response.headers().get(http::header::LOCATION).is_none(), "{location:?}");
                assert_eq!(response.status(), StatusCode::FOUND);
            }
        }
    }

    #[test]
    fn should_refuse_a_response_that_is_not_valid_http() {
        let cases = [
            (json!({"status": 42}), "42 is not an HTTP status"),
            (json!({"headers": {"bad name": "x"}}), "\"bad name\" is not a valid header name"),
            (
                json!({"headers": {"content-type": "line\nbreak"}}),
                "the value of the header content-type is not a valid header value",
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
        // a header zhang drops anyway is no error, whatever its value
        let response: PluginResponse = serde_json::from_value(json!({"headers": {"x-bad": "line\nbreak"}})).unwrap();
        assert!(response.into_http().is_ok());
    }
}
