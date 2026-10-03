//! The current date and deterministic randomness.
//!
//! A ledger should load the same way every time. Plugins therefore read the time from zhang, not from the
//! system, and derive randomness from the seed zhang gives them, not from the OS:
//!
//! - [`now`] and [`today`] call the `zhang_now` host function. zhang reads its clock once per load, so every
//!   plugin of a load sees the same instant, in the ledger's timezone. A processor or mapper reading it makes
//!   the ledger depend on the date, and a server reloads such a ledger at midnight. A router reads the time
//!   afresh for each request.
//! - [`rng`] and [`rng_for`] are seeded from the `zhang.seed` config, which depends only on the plugin's
//!   `plugin` directive (its module, its position among directives of the same module, and its `seed` meta), so
//!   the ids a plugin generates stay the same on every reload.
//!
//! A plugin built for WASI can still reach the host's real clock and entropy through WASI; zhang cannot stop
//! it, so such a plugin is simply not reproducible. Build for `wasm32-unknown-unknown` and use this module.

use std::cell::RefCell;

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Deserialize;
use zhang_ast::{Directive, Spanned};

use crate::abi;
use crate::config::{host_seed, ConfigError};
use crate::error::{host_result, HostError, HostErrorKind};

/// the current time of the load, from `zhang_now`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Now {
    /// the current time, with the offset of the ledger's timezone
    pub now: DateTime<FixedOffset>,
    /// the date of [`Now::now`] in the ledger's timezone
    pub today: NaiveDate,
    /// the ledger's timezone, an IANA name such as `Asia/Shanghai`
    pub timezone: String,
}

/// what `zhang_now` answers inside `Ok`
#[derive(Deserialize)]
struct NowPayload {
    now: String,
    today: String,
    timezone: String,
}

impl Now {
    fn from_answer(answer: &[u8]) -> Result<Now, HostError> {
        let payload: NowPayload = host_result(abi::NOW, answer)?;
        let unreadable = |what: &str, value: &str| HostError::new(HostErrorKind::Other, format!("zhang_now answered a {what} this SDK cannot read: {value:?}"));
        Ok(Now {
            now: DateTime::parse_from_rfc3339(&payload.now).map_err(|_| unreadable("time", &payload.now))?,
            today: NaiveDate::parse_from_str(&payload.today, "%Y-%m-%d").map_err(|_| unreadable("date", &payload.today))?,
            timezone: payload.timezone,
        })
    }
}

/// the current time of the load, in the ledger's timezone
pub fn now() -> Result<Now, HostError> {
    Now::from_answer(&abi::now()?)
}

/// today's date in the ledger's timezone
pub fn today() -> Result<NaiveDate, HostError> {
    Ok(now()?.today)
}

/// A small deterministic pseudo-random generator (SplitMix64). It is fast and well distributed, but **not**
/// cryptographically secure: use it for ids, sampling and tie-breaking, never for secrets.
///
/// The sequence a seed produces is part of the SDK's contract and never changes between versions, so ids a
/// plugin derives from it survive SDK upgrades.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// a generator starting from `seed`
    pub fn new(seed: u64) -> Rng {
        Rng { state: seed }
    }

    /// the generator [`rng_for`] returns for `directive`, given the plugin's seed `seed`
    pub fn for_directive(seed: u64, directive: &Spanned<Directive>) -> Rng {
        Rng::new(mix(seed ^ mix(fnv1a(&directive_text(directive)))))
    }

    /// the next 64 random bits
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.state)
    }

    /// the next 32 random bits
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// a number in `[0, 1)`
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// a number in `[0, bound)`, without modulo bias
    ///
    /// # Panics
    ///
    /// when `bound` is zero
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "Rng::below needs a bound above zero");
        // the values below `threshold` would make low results more likely; draw again
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next_u64();
            if value >= threshold {
                return value % bound;
            }
        }
    }

    /// fill `bytes` with random bytes
    pub fn fill_bytes(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            let random = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&random[..chunk.len()]);
        }
    }
}

/// the SplitMix64 output function
fn mix(value: u64) -> u64 {
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// the 64-bit FNV-1a hash of `bytes`
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// what identifies a directive for [`rng_for`]: its source text, or its JSON when it has none (a directive
/// another plugin created)
fn directive_text(directive: &Spanned<Directive>) -> Vec<u8> {
    if !directive.span.content.is_empty() {
        return directive.span.content.as_bytes().to_vec();
    }
    // through `Value`, whose maps are sorted, so the metadata's hash map order does not leak in
    serde_json::to_value(&directive.data)
        .map(|value| value.to_string().into_bytes())
        .unwrap_or_default()
}

thread_local! {
    /// the generator [`rng`] splits new generators off, seeded on first use
    static ROOT: RefCell<Option<Rng>> = const { RefCell::new(None) };
}

/// A generator seeded from the plugin's `zhang.seed`.
///
/// The first call in a plugin instance returns the same generator on every load; each later call in the same
/// instance returns a further, different one. zhang runs a mapper in one instance for the whole stream, so a
/// mapper calling `rng()` for every directive gets a new generator each time, and the same sequence of them on
/// every load. Fails with [`ConfigError::Missing`] on a zhang older than plugin ABI v1.
pub fn rng() -> Result<Rng, ConfigError> {
    ROOT.with(|root| {
        let mut root = root.borrow_mut();
        if root.is_none() {
            *root = Some(Rng::new(host_seed()?));
        }
        let root = root.as_mut().expect("seeded above");
        Ok(Rng::new(root.next_u64()))
    })
}

/// A generator for one directive: seeded from the plugin's `zhang.seed` and the directive's source text, so it is
/// the same on every load, whatever else changes in the ledger, until the directive itself is edited. Two
/// directives with identical text get identical generators. A directive without source text (one another plugin
/// created) is identified by its content instead. Fails with [`ConfigError::Missing`] on a zhang older than plugin
/// ABI v1.
pub fn rng_for(directive: &Spanned<Directive>) -> Result<Rng, ConfigError> {
    Ok(Rng::for_directive(host_seed()?, directive))
}

#[cfg(test)]
mod test {
    use chrono::NaiveDate;
    use zhang_ast::{Comment, Directive, SpanInfo, Spanned};

    use super::{fnv1a, Now, Rng};
    use crate::error::HostErrorKind;

    fn comment(content: &str, source: &str) -> Spanned<Directive> {
        Spanned::new(
            Directive::Comment(Comment { content: content.to_owned() }),
            SpanInfo {
                start: 0,
                end: source.len(),
                content: source.to_owned(),
                filename: None,
            },
        )
    }

    #[test]
    fn should_produce_the_pinned_splitmix64_sequence() {
        // the reference SplitMix64 outputs for seed 0; changing them would move every generated id
        let mut rng = Rng::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
        assert_eq!(fnv1a(b""), 0xCBF2_9CE4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xAF63_DC4C_8601_EC8C);
    }

    #[test]
    fn should_stay_in_range_and_fill_every_byte() {
        let mut rng = Rng::new(42);
        assert!((0..1000).map(|_| rng.below(7)).all(|value| value < 7));
        assert!((0..1000).map(|_| rng.next_f64()).all(|value| (0.0..1.0).contains(&value)));
        let mut bytes = [0u8; 13];
        rng.fill_bytes(&mut bytes);
        assert_ne!(bytes[8..], [0u8; 5]);
        assert_eq!(Rng::new(7), Rng::new(7));
    }

    #[test]
    fn should_derive_a_directive_generator_from_its_text_only() {
        let first = comment("; a", "; a");
        let moved = Spanned::new(
            first.data.clone(),
            SpanInfo {
                start: 100,
                end: 103,
                ..first.span.clone()
            },
        );
        let edited = comment("; b", "; b");
        let generated = comment("; a", "");

        let next = |seed: u64, directive: &Spanned<Directive>| Rng::for_directive(seed, directive).next_u64();
        assert_eq!(next(1, &first), next(1, &moved), "moving a directive keeps its generator");
        assert_ne!(next(1, &first), next(1, &edited));
        assert_ne!(next(1, &first), next(2, &first), "the plugin's seed changes it");
        assert_eq!(next(1, &generated), next(1, &generated.clone()));
        assert_ne!(next(1, &first), next(1, &generated));
    }

    #[test]
    fn should_read_the_now_answer() {
        let now = Now::from_answer(br#"{"Ok": {"now": "2024-03-16T00:30:00+08:00", "today": "2024-03-16", "timezone": "Asia/Shanghai"}}"#).unwrap();
        assert_eq!(now.today, NaiveDate::from_ymd_opt(2024, 3, 16).unwrap());
        assert_eq!(now.now.to_rfc3339(), "2024-03-16T00:30:00+08:00");
        assert_eq!(now.timezone, "Asia/Shanghai");
        assert_eq!(
            Now::from_answer(br#"{"Ok": {"now": "later", "today": "", "timezone": ""}}"#).unwrap_err().kind,
            HostErrorKind::Other
        );
        // outside zhang
        assert_eq!(super::today().unwrap_err().kind, HostErrorKind::Unavailable);
        assert!(super::rng().is_err());
    }
}
