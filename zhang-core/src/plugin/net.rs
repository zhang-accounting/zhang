//! A plugin's HTTP requests: zhang's own `http_request` host function, in place of extism's.
//!
//! A plugin reaches the network through `extism:host/env::http_request` (the PDK's `http::request`), and
//! [`http_status_code`] for the status of the last response. zhang links its own functions under those names, which
//! shadow extism's, because extism's `http_request` checks only the host name against `allowed_hosts`:
//!
//! - it followed redirects, so an allowed host could send the plugin anywhere (`localhost` to `127.0.0.1` worked);
//! - it connected to whatever the name resolved to, so an allowed name could lead to the cloud metadata service
//!   (`169.254.169.254`), the machine itself or the private network, and a name resolving to a public address when it
//!   was checked could resolve to a private one when it was connected to (DNS rebinding).
//!
//! Here a request goes only to an `http` or `https` URL whose host the plugin's effective `allowed_hosts` allow (the
//! directive's grant within the operator's ceiling, see [`PluginCapabilities::bound_by`]); the name is resolved once,
//! every address it resolves to that is not public is dropped ([`is_public`]), and the request connects to the
//! remaining addresses only. Redirects are not followed: the plugin gets the `3xx` response, and following it is a
//! new request, checked the same way. No proxy of the environment is used. The function keeps extism's contract
//! otherwise: it takes the JSON `{"url", "method"?, "headers"?}` and a body, frees both, answers the offset of the
//! response body (0 for none), and fails the call (a trap) when the request is not allowed or fails. A plugin that is
//! registering (its `name`, `version`, `supported_type` and `capabilities` exports) makes no request at all
//! ([`REGISTERING_REFUSED`]): those only declare what it is.
//!
//! [`PluginCapabilities::bound_by`]: crate::plugin::capabilities::PluginCapabilities::bound_by

use std::collections::BTreeMap;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::{mpsc, Mutex, PoisonError};
use std::time::Duration;

use extism::{CurrentPlugin, UserData, Val};
use serde::Deserialize;
use ureq::config::Config;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use ureq::Agent;
use url::{Host, Url};

use crate::plugin::capabilities::{host_allowed, PLUGIN_CONFIG_KEY};
use crate::plugin::host::{read_input, HostState};
use crate::plugin::log_safe;

/// the name of the host function a plugin makes an HTTP request with, in the `extism:host/env` namespace
pub const HTTP_REQUEST: &str = "http_request";
/// the name of the host function a plugin reads the status of its last response with, in `extism:host/env`
pub const HTTP_STATUS_CODE: &str = "http_status_code";

/// the most bytes of request JSON (`url`, `method`, `headers`) read from a plugin
const REQUEST_MAX_LEN: usize = 1024 * 1024;
/// the most bytes of request body read from a plugin
const BODY_MAX_LEN: usize = 16 * 1024 * 1024;
/// the largest response body a plugin receives when its manifest sets none, extism's own default
const DEFAULT_MAX_RESPONSE: u64 = 50 * 1024 * 1024;
/// why `http_request` fails while the plugin registers: one manifest serves the registration and the calls, so the
/// allowed hosts are there, but registering only declares what the plugin is
pub const REGISTERING_REFUSED: &str = "HTTP requests are not allowed while the plugin registers (its name, version, supported_type and \
                                       capabilities exports only declare what it is): make them from a stage or a router request";

/// what a plugin asks `http_request` for, extism's `HttpRequest`
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HttpRequest {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub method: Option<String>,
}

/// Whether a request may go to `ip`: not to an unspecified, loopback, private, shared (CGNAT), link-local,
/// documentation, benchmarking, multicast, broadcast or reserved address, nor to Azure's host address
/// `168.63.129.16`, nor to an IPv6 address standing for such an IPv4 one (mapped, well-known NAT64, 6to4) or for an
/// IPv4 address a local translator picks (other NAT64 prefixes, IPv4-translated) or a tunnel (Teredo), nor to IPv6
/// unique-local and site-local addresses.
///
/// Every other address counts as public, the server's own public address too: a plugin allowed `*` can reach a port
/// of the server's own machine through it, and any port of an allowed host. Allow only the hosts a plugin needs.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            let s = ip.segments();
            let embedded = |high: u16, low: u16| Ipv4Addr::from((u32::from(high) << 16) | u32::from(low));
            // the well-known NAT64 prefix (64:ff9b::/96) and 6to4 (2002::/16) reach the IPv4 address they embed
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return is_public_v4(embedded(s[6], s[7]));
            }
            if s[0] == 0x2002 {
                return is_public_v4(embedded(s[1], s[2]));
            }
            // the rest of 64:ff9b::/32, such as the local-use NAT64 prefix 64:ff9b:1::/48, and IPv4-translated
            // addresses (::ffff:0:0/96, SIIT) reach an IPv4 address too, embedded where the local translator puts it,
            // which an address alone does not tell: none of them is taken as public
            if s[..2] == [0x64, 0xff9b] || s[..6] == [0, 0, 0, 0, 0xffff, 0] {
                return false;
            }
            let not_public = ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                // IPv4-compatible (::a.b.c.d), deprecated
                || s[..6] == [0, 0, 0, 0, 0, 0]
                // unique local fc00::/7, link-local fe80::/10, site-local fec0::/10
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xffc0) == 0xfec0
                // Teredo 2001::/32, documentation 2001:db8::/32, discard-only 100::/64
                || (s[0] == 0x2001 && (s[1] == 0 || s[1] == 0x0db8))
                || s[..4] == [0x0100, 0, 0, 0];
            !not_public
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    let not_public = o[0] == 0 // 0.0.0.0/8, "this network"
        || ip.is_loopback() // 127.0.0.0/8
        || ip.is_private() // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local() // 169.254/16, the cloud metadata service
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64/10, shared address space (CGNAT)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0/24, IETF protocol assignments
        || ip.is_documentation() // 192.0.2/24, 198.51.100/24, 203.0.113/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18/15, benchmarking
        || o == [168, 63, 129, 16] // Azure's host address (wire server, DNS, health probes), a metadata service
        || o[0] >= 224; // multicast 224/4, reserved 240/4, broadcast
    !not_public
}

/// the resolver of one request: the addresses zhang checked, whatever ureq asks for, so it connects to them only
#[derive(Debug)]
struct Checked(Vec<SocketAddr>);

impl Resolver for Checked {
    fn resolve(&self, _uri: &ureq::http::Uri, _config: &Config, _timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let mut addresses = self.empty();
        for address in &self.0 {
            if addresses.try_push(*address).is_err() {
                break;
            }
        }
        Ok(addresses)
    }
}

/// the most name lookups of plugins that may run at once, see [`resolve`]
const MAX_LOOKUPS: usize = 8;

/// the most name lookups of one plugin that may run at once, see [`resolve`]
const MAX_LOOKUPS_PER_PLUGIN: usize = 2;

/// the name lookups of plugins running now, on their own threads: how many in all, and how many of each plugin
struct Lookups {
    running: usize,
    by_plugin: BTreeMap<String, usize>,
}

static LOOKUPS: Mutex<Lookups> = Mutex::new(Lookups {
    running: 0,
    by_plugin: BTreeMap::new(),
});

/// one name lookup of a plugin, counted in [`LOOKUPS`] until its thread ends
struct Lookup {
    plugin: String,
}

impl Lookup {
    /// a lookup of `plugin`, unless that plugin already runs [`MAX_LOOKUPS_PER_PLUGIN`] or all plugins run
    /// [`MAX_LOOKUPS`]; the error says which
    fn start(plugin: &str) -> Result<Lookup, String> {
        let mut lookups = LOOKUPS.lock().unwrap_or_else(PoisonError::into_inner);
        let of_plugin = lookups.by_plugin.get(plugin).copied().unwrap_or_default();
        if of_plugin >= MAX_LOOKUPS_PER_PLUGIN {
            return Err(format!("{MAX_LOOKUPS_PER_PLUGIN} name lookups of this plugin are still running"));
        }
        if lookups.running >= MAX_LOOKUPS {
            return Err(format!("{MAX_LOOKUPS} name lookups of plugins are still running"));
        }
        lookups.running += 1;
        lookups.by_plugin.insert(plugin.to_owned(), of_plugin + 1);
        Ok(Lookup { plugin: plugin.to_owned() })
    }
}

impl Drop for Lookup {
    fn drop(&mut self) {
        let mut lookups = LOOKUPS.lock().unwrap_or_else(PoisonError::into_inner);
        lookups.running -= 1;
        match lookups.by_plugin.get_mut(&self.plugin) {
            Some(count) if *count > 1 => *count -= 1,
            _ => {
                lookups.by_plugin.remove(&self.plugin);
            }
        }
    }
}

/// The addresses `host` resolves to on `port` for the plugin `plugin`, within `timeout` when there is one. A lookup
/// cannot be interrupted, so it runs on its own thread, which is left behind when the lookup takes longer than the
/// call has left. At most [`MAX_LOOKUPS`] of them run at once, and at most [`MAX_LOOKUPS_PER_PLUGIN`] of one plugin: a
/// resolver that never answers holds that many threads, not one per call, and a plugin whose host's name server hangs
/// (its author may run it) blocks only its own lookups, not those of the other plugins.
fn resolve(host: &str, port: u16, timeout: Option<Duration>, plugin: &str) -> Result<Vec<SocketAddr>, String> {
    let target = (host.to_owned(), port);
    let lookup = move || target.to_socket_addrs().map(Iterator::collect::<Vec<_>>);
    let addresses = match timeout {
        None => lookup(),
        Some(timeout) => {
            let running = Lookup::start(plugin).map_err(|reason| format!("cannot resolve {host}: {reason}"))?;
            let (sender, receiver) = mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let _running = running;
                sender.send(lookup()).ok()
            });
            receiver.recv_timeout(timeout).map_err(|_| "timeout".to_owned())?
        }
    };
    addresses.map_err(|e| format!("cannot resolve {host}: {e}"))
}

/// Send `request` with `body`, if the plugin may: its host must be allowed by `allowed_hosts`, and only the
/// addresses `address_allowed` lets through are connected to ([`is_public`] for a plugin). The status and the body
/// of the response, at most `max_response` bytes; an error says why the request was refused or failed. `timeout` is
/// what is left of the plugin's call, and `plugin` tells the plugin's name lookups apart from those of others (see
/// [`resolve`]).
pub fn send(
    plugin: &str, request: &HttpRequest, body: Option<&[u8]>, allowed_hosts: &[String], timeout: Option<Duration>, max_response: u64,
    address_allowed: fn(IpAddr) -> bool,
) -> Result<(u16, Vec<u8>), String> {
    let url = Url::parse(&request.url).map_err(|e| format!("invalid URL {:?}: {e}", log_safe(&request.url)))?;
    let refused = |reason: String| format!("HTTP request to {url} is not allowed: {reason}");
    if !matches!(url.scheme(), "http" | "https") {
        return Err(refused("only http and https URLs are".to_owned()));
    }
    let host_name = url.host_str().unwrap_or_default();
    if !allowed_hosts.iter().any(|pattern| host_allowed(pattern, host_name)) {
        return Err(refused(format!("{host_name} is not one of the plugin's allowed_hosts")));
    }
    let port = url.port_or_known_default().ok_or_else(|| refused("it has no port".to_owned()))?;
    let (resolved, named) = match url.host() {
        Some(Host::Ipv4(ip)) => (vec![SocketAddr::new(ip.into(), port)], false),
        Some(Host::Ipv6(ip)) => (vec![SocketAddr::new(ip.into(), port)], false),
        Some(Host::Domain(domain)) => (resolve(domain, port, timeout, plugin)?, true),
        None => return Err(refused("it has no host".to_owned())),
    };
    let (checked, dropped): (Vec<SocketAddr>, Vec<SocketAddr>) = resolved.into_iter().partition(|address| address_allowed(address.ip()));
    if checked.is_empty() {
        let dropped = dropped.iter().map(|address| address.ip().to_string()).collect::<Vec<_>>().join(", ");
        return Err(refused(match named {
            true => format!("{host_name} resolves to {dropped}, and none of them is a public address"),
            false => format!("{host_name} is not a public address"),
        }));
    }

    let config = Agent::config_builder()
        .max_redirects(0)
        .proxy(None)
        .http_status_as_error(false)
        .timeout_global(timeout)
        .build();
    let agent = Agent::with_parts(config, DefaultConnector::new(), Checked(checked));
    let method = request.method.as_deref().unwrap_or("GET").to_uppercase();
    let mut builder = ureq::http::Request::builder().method(method.as_str()).uri(url.as_str());
    // the request goes to the host that was checked: a `host` header of the plugin's own could name another site
    // served at the same address
    for (name, value) in request.headers.iter().filter(|(name, _)| !name.eq_ignore_ascii_case("host")) {
        builder = builder.header(name, value);
    }
    let response = match body {
        Some(body) => agent.run(builder.body(body).map_err(|e| e.to_string())?),
        None => agent.run(builder.body(()).map_err(|e| e.to_string())?),
    };
    let response = match response {
        Ok(response) => response,
        // the request's time is what is left of the call's, so the call ran out of time, as extism reports it
        Err(ureq::Error::Timeout(_)) => return Err("timeout".to_owned()),
        Err(e) => return Err(format!("HTTP request to {url} failed: {e}")),
    };
    let status = response.status().as_u16();
    let mut received = vec![];
    response
        .into_body()
        .into_reader()
        .take(max_response.saturating_add(1))
        .read_to_end(&mut received)
        .map_err(|e| format!("HTTP request to {url} failed: {e}"))?;
    if received.len() as u64 > max_response {
        return Err(format!("HTTP response exceeds the configured maximum number of bytes: {max_response}"));
    }
    Ok((status, received))
}

/// the bytes of the block a plugin passed as `val`, freed once read; `None` for no block (offset 0)
fn take_block(plugin: &mut CurrentPlugin, val: Option<&Val>, max_len: usize, what: &str) -> Result<Option<Vec<u8>>, extism::Error> {
    let Some(val) = val.filter(|val| val.i64() != Some(0)) else {
        return Ok(None);
    };
    let bytes = read_input(plugin, val, max_len).map_err(|e| extism::Error::msg(format!("the {what} {e}")));
    // extism's contract: this function owns the blocks it is handed, read or not
    if let Some(handle) = plugin.memory_from_val(val) {
        plugin.memory_free(handle)?;
    }
    bytes.map(Some)
}

/// `http_request(request, body) -> response body`, see the module
pub(super) fn http_request(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let request =
        take_block(plugin, inputs.first(), REQUEST_MAX_LEN, "HTTP request")?.ok_or_else(|| extism::Error::msg("the HTTP request is not a memory block"))?;
    let body = take_block(plugin, inputs.get(1), BODY_MAX_LEN, "HTTP request body")?;
    if state.get()?.lock().unwrap_or_else(PoisonError::into_inner).registering {
        return Err(extism::Error::msg(REGISTERING_REFUSED));
    }
    let request: HttpRequest = serde_json::from_slice(&request).map_err(|e| extism::Error::msg(format!("the HTTP request is not valid: {e}")))?;
    let allowed_hosts = plugin.manifest().allowed_hosts.clone().unwrap_or_default();
    let max_response = plugin.manifest().memory.max_http_response_bytes.unwrap_or(DEFAULT_MAX_RESPONSE);
    let timeout = plugin.time_remaining();
    // the plugin's directive as written tells plugins apart: the plugin cannot pick it, unlike its name
    let identity = plugin.manifest().config.get(PLUGIN_CONFIG_KEY).cloned().unwrap_or_default();
    set_status(&state, 0)?;
    let (status, response) = send(&identity, &request, body.as_deref(), &allowed_hosts, timeout, max_response, is_public).map_err(extism::Error::msg)?;
    set_status(&state, status)?;
    let output = outputs.first_mut().ok_or_else(|| extism::Error::msg("http_request returns one value"))?;
    match response.is_empty() {
        true => *output = Val::I64(0),
        false => plugin.memory_set_val(output, response.as_slice())?,
    }
    Ok(())
}

fn set_status(state: &UserData<HostState>, status: u16) -> Result<(), extism::Error> {
    state.get()?.lock().unwrap_or_else(PoisonError::into_inner).http_status = status;
    Ok(())
}

/// `http_status_code() -> status`: the status of the last response, 0 before any
pub(super) fn http_status_code(_plugin: &mut CurrentPlugin, _inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let status = state.get()?.lock().unwrap_or_else(PoisonError::into_inner).http_status;
    let output = outputs.first_mut().ok_or_else(|| extism::Error::msg("http_status_code returns one value"))?;
    *output = Val::I32(i32::from(status));
    Ok(())
}

#[cfg(test)]
mod test {
    use std::io::{Read, Write};
    use std::net::{IpAddr, TcpListener};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{is_public, send, HttpRequest, Lookup, MAX_LOOKUPS, MAX_LOOKUPS_PER_PLUGIN};

    #[test]
    fn should_refuse_every_address_that_is_not_public() {
        for ip in [
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::127.0.0.1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::a9fe:a9fe",
            "64:ff9b:1:fffe::808:808",
            "64:ff9b:0:1::808:808",
            "::ffff:0:a9fe:a9fe",
            "::ffff:0:808:808",
            "168.63.129.16",
            "::ffff:168.63.129.16",
            "2002:7f00:1::",
            "2001::1",
            "2001:db8::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "100::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip} is not public");
        }
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.0.1",
            "172.32.0.1",
            "198.17.0.1",
            "198.20.0.1",
            "2606:4700:4700::1111",
            "2a00:1450::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip} is public");
        }
    }

    fn get(url: &str) -> HttpRequest {
        HttpRequest {
            url: url.to_owned(),
            ..HttpRequest::default()
        }
    }

    fn hosts(patterns: &[&str]) -> Vec<String> {
        patterns.iter().map(|it| it.to_string()).collect()
    }

    fn any_address(_: IpAddr) -> bool {
        true
    }

    const TIMEOUT: Option<Duration> = Some(Duration::from_secs(5));

    #[test]
    fn a_plugin_whose_lookups_hang_cannot_starve_the_others() {
        // a lookup is counted until its thread ends, which a name server that never answers delays forever: these
        // stand for such hung lookups
        let hung = [Lookup::start("slow-dns"), Lookup::start("slow-dns")];
        assert!(hung.iter().all(Result::is_ok));

        // the plugin gets no more lookups, and says why
        assert_eq!(
            Lookup::start("slow-dns").err().as_deref(),
            Some("2 name lookups of this plugin are still running")
        );
        let error = send(
            "slow-dns",
            &get("http://localhost:9/"),
            None,
            &hosts(&["localhost"]),
            TIMEOUT,
            1024,
            any_address,
        )
        .unwrap_err();
        assert_eq!(error, "cannot resolve localhost: 2 name lookups of this plugin are still running");

        // every other plugin still resolves names
        let error = send("other", &get("http://localhost:9/"), None, &hosts(&["localhost"]), TIMEOUT, 1024, is_public).unwrap_err();
        assert!(error.contains("localhost resolves to"), "{error}");
        assert!(Lookup::start("other").is_ok());

        // and once the hung lookups end, the plugin gets its share back
        drop(hung);
        assert!(Lookup::start("slow-dns").is_ok());
    }

    #[test]
    fn should_run_at_most_a_few_name_lookups_of_all_plugins_at_once() {
        let lookups = (0..MAX_LOOKUPS)
            .map(|it| Lookup::start(&format!("plugin-{}", it / MAX_LOOKUPS_PER_PLUGIN)))
            .collect::<Vec<_>>();
        // a lookup of another test may hold one; when this test holds them all, no plugin starts one more
        if lookups.iter().all(Result::is_ok) {
            assert_eq!(
                Lookup::start("yet-another").err().as_deref(),
                Some("8 name lookups of plugins are still running")
            );
        }
        drop(lookups);
        assert!(Lookup::start("yet-another").is_ok(), "a lookup ending frees its place");
    }

    #[test]
    fn should_refuse_a_host_outside_the_allowed_hosts_and_other_schemes() {
        let error = send(
            "test",
            &get("https://evil.example/"),
            None,
            &hosts(&["api.example.com"]),
            TIMEOUT,
            1024,
            is_public,
        )
        .unwrap_err();
        assert_eq!(
            error,
            "HTTP request to https://evil.example/ is not allowed: evil.example is not one of the plugin's allowed_hosts"
        );
        let error = send("test", &get("https://api.example.com/"), None, &[], TIMEOUT, 1024, is_public).unwrap_err();
        assert!(error.ends_with("api.example.com is not one of the plugin's allowed_hosts"), "{error}");
        let error = send("test", &get("file:///etc/passwd"), None, &hosts(&["*"]), TIMEOUT, 1024, is_public).unwrap_err();
        assert!(error.ends_with("only http and https URLs are"), "{error}");
    }

    #[test]
    fn should_refuse_private_addresses_before_connecting() {
        for (url, address) in [
            ("http://169.254.169.254/latest/meta-data/", "169.254.169.254"),
            ("http://127.0.0.1:9/", "127.0.0.1"),
            ("http://[::1]:9/", "::1"),
            ("http://10.0.0.1/", "10.0.0.1"),
            ("http://100.64.0.1/", "100.64.0.1"),
            ("http://[::ffff:127.0.0.1]/", "127.0.0.1"),
        ] {
            let error = send("test", &get(url), None, &hosts(&["*"]), TIMEOUT, 1024, is_public).unwrap_err();
            assert!(error.ends_with("is not a public address"), "{url}: {error}");
            assert!(error.contains(address) || url.contains("ffff"), "{url}: {error}");
        }
    }

    /// a server on the loopback answering every request with `response`; the number of requests it got
    fn server(response: impl Fn(u16) -> String + Send + 'static) -> (u16, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                counted.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(response(port).as_bytes());
            }
        });
        (port, requests)
    }

    #[test]
    fn should_resolve_a_name_and_refuse_it_when_it_leads_to_a_private_address() {
        let (port, requests) = server(|_| "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned());

        // localhost is allowed by name, and resolves to the loopback: refused, nothing is sent
        let error = send(
            "test",
            &get(&format!("http://localhost:{port}/")),
            None,
            &hosts(&["localhost"]),
            TIMEOUT,
            1024,
            is_public,
        )
        .unwrap_err();
        assert!(error.contains("localhost resolves to "), "{error}");
        assert!(error.ends_with("none of them is a public address"), "{error}");
        assert_eq!(requests.load(Ordering::SeqCst), 0);

        // where every address is allowed, the same request goes through to the address the name was checked at
        let (status, body) = send(
            "test",
            &get(&format!("http://localhost:{port}/")),
            None,
            &hosts(&["localhost"]),
            TIMEOUT,
            1024,
            any_address,
        )
        .unwrap();
        assert_eq!((status, body.as_slice()), (200, &b"ok"[..]));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn should_hand_a_redirect_to_the_plugin_instead_of_following_it() {
        let (port, requests) =
            server(|port| format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));

        let (status, body) = send(
            "test",
            &get(&format!("http://127.0.0.1:{port}/")),
            None,
            &hosts(&["127.0.0.1"]),
            TIMEOUT,
            1024,
            any_address,
        )
        .unwrap();

        assert_eq!((status, body), (302, vec![]));
        assert_eq!(requests.load(Ordering::SeqCst), 1, "the redirect is not followed");
    }

    #[test]
    fn should_refuse_a_response_larger_than_the_limit() {
        let (port, _) = server(|_| "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789".to_owned());

        let error = send(
            "test",
            &get(&format!("http://127.0.0.1:{port}/")),
            None,
            &hosts(&["127.0.0.1"]),
            TIMEOUT,
            4,
            any_address,
        )
        .unwrap_err();

        assert_eq!(error, "HTTP response exceeds the configured maximum number of bytes: 4");
    }
}
