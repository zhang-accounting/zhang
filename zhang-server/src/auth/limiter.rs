//! Brute-force protection of the secrets entered without a session: the password of the login and
//! the passkey registration secret. Failed attempts are counted per client and globally, in memory
//! (so the counts start over when the server restarts).

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;

use super::first_header_value;

/// Failed attempts a client may make within [`FAILURE_WINDOW`].
pub const MAX_FAILURES_PER_CLIENT: usize = 5;

/// Failed attempts all clients together may make within [`FAILURE_WINDOW`], a bound for attackers
/// rotating their addresses.
pub const MAX_FAILURES_GLOBAL: usize = 50;

/// How long a failed attempt counts.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// The client a request comes from: the first hop of `X-Forwarded-For` when present, else the peer
/// address. IPv6 clients are grouped by their /64 network, which one host usually controls whole.
pub fn client_key(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    let address = match first_header_value(headers, "x-forwarded-for") {
        Some(forwarded) => match parse_address(forwarded) {
            Some(ip) => ip,
            None => return forwarded.chars().take(64).collect(),
        },
        None => match peer {
            Some(peer) => peer.ip(),
            None => return "unknown".to_owned(),
        },
    };
    match address.to_canonical() {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", segments[0], segments[1], segments[2], segments[3])
        }
    }
}

/// An address, possibly with a port (`1.2.3.4:5678`, `[::1]:80`) or in brackets (`[::1]`).
fn parse_address(value: &str) -> Option<IpAddr> {
    IpAddr::from_str(value)
        .ok()
        .or_else(|| SocketAddr::from_str(value).ok().map(|it| it.ip()))
        .or_else(|| IpAddr::from_str(value.trim_start_matches('[').trim_end_matches(']')).ok())
}

/// The failed attempts of the last [`FAILURE_WINDOW`].
#[derive(Default)]
pub struct FailureLimiter {
    clients: HashMap<String, VecDeque<Instant>>,
    global: VecDeque<Instant>,
}

impl FailureLimiter {
    /// How long `client` has to wait before it may try again, when it (or everyone) made too many
    /// failed attempts. Refused attempts are not counted, so the wait ends with the window.
    pub fn blocked_for(&mut self, client: &str, now: Instant) -> Option<Duration> {
        self.prune(now);
        let client_wait = self.clients.get(client).and_then(|failures| wait(failures, MAX_FAILURES_PER_CLIENT, now));
        let global_wait = wait(&self.global, MAX_FAILURES_GLOBAL, now);
        client_wait.max(global_wait)
    }

    pub fn record_failure(&mut self, client: &str, now: Instant) {
        self.prune(now);
        self.clients.entry(client.to_owned()).or_default().push_back(now);
        self.global.push_back(now);
    }

    /// Forgets the failed attempts of `client`, after it signed in.
    pub fn reset(&mut self, client: &str) {
        self.clients.remove(client);
    }

    fn prune(&mut self, now: Instant) {
        fn prune_failures(failures: &mut VecDeque<Instant>, now: Instant) {
            while failures
                .front()
                .is_some_and(|failed_at| now.saturating_duration_since(*failed_at) >= FAILURE_WINDOW)
            {
                failures.pop_front();
            }
        }
        prune_failures(&mut self.global, now);
        self.clients.retain(|_, failures| {
            prune_failures(failures, now);
            !failures.is_empty()
        });
    }
}

/// How long until fewer than `limit` of `failures` are in the window, when `limit` are.
fn wait(failures: &VecDeque<Instant>, limit: usize, now: Instant) -> Option<Duration> {
    if failures.len() < limit {
        return None;
    }
    let unblocking = failures[failures.len() - limit];
    Some((unblocking + FAILURE_WINDOW).saturating_duration_since(now))
}

/// A wait in whole seconds, rounded up.
pub fn whole_seconds(wait: Duration) -> u64 {
    wait.as_secs() + u64::from(wait.subsec_nanos() > 0)
}

/// `too many attempts, try again in N minutes`, N rounded up.
pub fn too_many_attempts_message(wait: Duration) -> String {
    let minutes = whole_seconds(wait).div_ceil(60).max(1);
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    format!("too many attempts, try again in {minutes} {unit}")
}

#[cfg(test)]
mod test {
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    use axum::http::{HeaderMap, HeaderValue};

    use super::{client_key, too_many_attempts_message, whole_seconds, FailureLimiter, FAILURE_WINDOW, MAX_FAILURES_GLOBAL, MAX_FAILURES_PER_CLIENT};

    fn forwarded(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn clients_are_the_first_forwarded_hop_or_the_peer() {
        let peer: SocketAddr = "198.51.100.1:5000".parse().unwrap();
        assert_eq!(client_key(&HeaderMap::new(), Some(peer)), "198.51.100.1");
        assert_eq!(client_key(&HeaderMap::new(), None), "unknown");
        assert_eq!(client_key(&forwarded("203.0.113.7, 10.0.0.1"), Some(peer)), "203.0.113.7");
        assert_eq!(client_key(&forwarded("203.0.113.7:4711"), None), "203.0.113.7");
        assert_eq!(client_key(&forwarded("::ffff:203.0.113.7"), None), "203.0.113.7");
        assert_eq!(client_key(&forwarded("2001:db8:1:2:aaaa::1"), None), "2001:db8:1:2::/64");
        assert_eq!(client_key(&forwarded("[2001:db8:1:2:bbbb::2]:443"), None), "2001:db8:1:2::/64");
        assert_eq!(client_key(&forwarded("not an address"), None), "not an address");
    }

    #[test]
    fn clients_are_blocked_after_too_many_failures_until_the_window_passes() {
        let start = Instant::now();
        let mut limiter = FailureLimiter::default();
        for minute in 0..MAX_FAILURES_PER_CLIENT as u64 {
            assert_eq!(limiter.blocked_for("a", start + Duration::from_secs(minute * 60)), None);
            limiter.record_failure("a", start + Duration::from_secs(minute * 60));
        }
        let now = start + Duration::from_secs(5 * 60);
        assert_eq!(
            limiter.blocked_for("a", now),
            Some(Duration::from_secs(10 * 60)),
            "until the first failure expires"
        );
        assert_eq!(limiter.blocked_for("b", now), None, "other clients are not affected");
        assert_eq!(limiter.blocked_for("a", start + FAILURE_WINDOW), None);
        // the four failures left in the window allow one more attempt
        limiter.record_failure("a", start + FAILURE_WINDOW);
        assert_eq!(limiter.blocked_for("a", start + FAILURE_WINDOW), Some(Duration::from_secs(60)));
    }

    #[test]
    fn signing_in_resets_the_client() {
        let now = Instant::now();
        let mut limiter = FailureLimiter::default();
        for _ in 0..MAX_FAILURES_PER_CLIENT {
            limiter.record_failure("a", now);
        }
        assert!(limiter.blocked_for("a", now).is_some());
        limiter.reset("a");
        assert_eq!(limiter.blocked_for("a", now), None);
    }

    #[test]
    fn everyone_is_blocked_after_too_many_failures_overall() {
        let now = Instant::now();
        let mut limiter = FailureLimiter::default();
        for failure in 0..MAX_FAILURES_GLOBAL {
            limiter.record_failure(&format!("client-{failure}"), now);
        }
        assert_eq!(limiter.blocked_for("fresh", now), Some(FAILURE_WINDOW));
        limiter.reset("client-0");
        assert_eq!(
            limiter.blocked_for("fresh", now),
            Some(FAILURE_WINDOW),
            "a sign-in does not reset the global count"
        );
        assert_eq!(limiter.blocked_for("fresh", now + FAILURE_WINDOW), None);
        assert!(limiter.clients.is_empty(), "expired clients are forgotten");
    }

    #[test]
    fn waits_are_announced_in_whole_minutes() {
        assert_eq!(whole_seconds(Duration::from_millis(1500)), 2);
        assert_eq!(too_many_attempts_message(FAILURE_WINDOW), "too many attempts, try again in 15 minutes");
        assert_eq!(too_many_attempts_message(Duration::from_secs(61)), "too many attempts, try again in 2 minutes");
        assert_eq!(too_many_attempts_message(Duration::from_secs(5)), "too many attempts, try again in 1 minute");
    }
}
