use serde::de::DeserializeOwned;
use zhang_shared::plugin_abi::HostResult;
pub use zhang_shared::plugin_abi::{Error, HostError, HostErrorKind};

/// what every host function answers on a native build, outside zhang
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn outside_zhang(function: &str) -> HostError {
    HostError::new(
        HostErrorKind::Unavailable,
        format!("{function} is only available inside zhang: this is a native build of the plugin"),
    )
}

/// read the answer of the host function `function`
pub(crate) fn host_result<T: DeserializeOwned>(function: &str, answer: &[u8]) -> Result<T, HostError> {
    match serde_json::from_slice::<HostResult<T>>(answer) {
        Ok(HostResult::Ok(value)) => Ok(value),
        Ok(HostResult::Err(error)) => Err(error),
        Err(e) => Err(HostError::new(
            HostErrorKind::Other,
            format!("{function} answered something this SDK cannot read: {e}"),
        )),
    }
}

#[cfg(test)]
mod test {
    use serde_json::Value;

    use super::{host_result, Error, HostError, HostErrorKind};

    #[test]
    fn should_read_ok_and_err_answers() {
        assert_eq!(host_result::<u32>("f", br#"{"Ok": 7}"#), Ok(7));
        assert_eq!(
            host_result::<u32>("f", br#"{"Err": {"kind": "not_found", "message": "a.txt: no such file"}}"#),
            Err(HostError::new(HostErrorKind::NotFound, "a.txt: no such file"))
        );
        let query = host_result::<Value>("f", br#"{"Err": {"kind": "query", "message": "unknown column", "line": 1, "column": 8}}"#).unwrap_err();
        assert_eq!((query.kind, query.line, query.column), (HostErrorKind::Query, Some(1), Some(8)));
        assert_eq!(query.to_string(), "unknown column (line 1, column 8)");
    }

    #[test]
    fn should_tolerate_kinds_from_a_newer_host_and_reject_garbage() {
        let newer = host_result::<u32>("f", br#"{"Err": {"kind": "rate_limited", "message": "later"}}"#).unwrap_err();
        assert_eq!(newer.kind, HostErrorKind::Other);
        let garbage = host_result::<u32>("zhang_now", b"nope").unwrap_err();
        assert_eq!(garbage.kind, HostErrorKind::Other);
        assert!(
            garbage.message.starts_with("zhang_now answered something this SDK cannot read"),
            "{}",
            garbage.message
        );
    }

    #[test]
    fn should_keep_the_sources_of_a_converted_error() {
        let error: Error = serde_json::from_str::<u32>("\"x\"").map_err(Error::from).unwrap_err();
        assert_eq!(error.message(), "invalid type: string \"x\", expected u32 at line 1 column 3");
        assert_eq!(Error::msg("boom").to_string(), "boom");
    }
}
