//! The writes of router plugins: what `zhang_write_preview` and `zhang_write_commit` do with the ledger, the server
//! side of [`zhang_core::plugin::write`] (which checks the phase, the declaration, the grant and the bounds that need
//! no ledger before anything reaches here).
//!
//! One validation, one exporter and one file policy, the API's: a `transaction` is read as `POST /api/transactions`
//! reads one and checked as `POST /api/transactions/preview` checks one ([`transaction_from_request`],
//! [`Ledger::check_transaction_with`]); an `open` is read with the same rules ([`open_from_request`],
//! [`Ledger::check_open`]); the file of each directive is the one the ledger's `directive_output_path` option gives it
//! ([`append_plan`]); and a commit is one `append` of the ledger's data source, as an API write is. The batch is
//! checked in order, each directive with the earlier ones counted as written, so "open, then post to the account"
//! previews clean in one call.
//!
//! A preview mints a [token](PluginWrites::mint) bound to the ledger generation, the plugin, and the exact files and
//! texts the batch plans; a commit recomputes the plan from the batch and verifies the token, so a plugin commits
//! exactly what it was shown, on the ledger it looked at, once: the commit moves the generation on, which voids every
//! token minted before, and the server marks the ledger stale and reloads it after the plugin returns (see
//! [`crate::routes::plugin_router`]). Until that reload, another commit is a `conflict`: one commit per reload cycle,
//! which is what keeps two commits from each including the same new monthly file.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;
use zhang_ast::{Date, Directive, DirectiveType, Open, ZhangString};
use zhang_core::data_source::{append_plan, output_file, slashed};
use zhang_core::inventory::BookingMethod;
use zhang_core::ledger::Ledger;
use zhang_core::plugin::write::{MAX_TEXT_LEN, TOKEN_TTL_SECS};
pub use zhang_shared::plugin_abi::{HostError, HostErrorKind, WriteCommitted, WriteKind, WritePreview, WritePreviewEntry};

use crate::request::{CreateTransactionRequest, LedgerDateTime, OpenAccountRequest};
use crate::response::{InvalidKind, TransactionField, TransactionPreviewErrorEntity};
use crate::routes::transaction::{metas_from_request, transaction_from_request, FieldError};
use crate::validate::{self, Invalid};
use crate::ReloadSender;

type HmacSha256 = Hmac<Sha256>;

/// the domain separating preview tokens from any other use of HMAC-SHA256 in zhang; the `v1` is part of the token
const TOKEN_DOMAIN: &[u8] = b"zhang/plugin-write/v1";

/// the meta of an `open` naming the account's booking method
const BOOKING_METHOD_META: &str = "booking_method";

/// What the writes of every router plugin share: the key their preview tokens are signed with, random per process
/// (a token does not outlive the server, as a session does not), the lock that serializes their commits among
/// themselves (API writes hold the ledger's write lock for their whole write, so they are excluded already: the lock
/// order is the ledger first, then this), and the generation the last commit moved the ledger to, which blocks a
/// second commit until the ledger reloaded.
pub struct PluginWrites {
    key: Vec<u8>,
    append: Mutex<()>,
    /// the generation the last commit moved the ledger to; 0 before any commit, and generations start at 1
    last_commit: AtomicU64,
}

impl Default for PluginWrites {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginWrites {
    /// fresh: a random key, so a token of an earlier run of the server is void
    pub fn new() -> Self {
        PluginWrites {
            key: rand::random::<[u8; 32]>().to_vec(),
            append: Mutex::new(()),
            last_commit: AtomicU64::new(0),
        }
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.key).expect("hmac accepts keys of any length")
    }

    /// what a token signs: the generation, the plugin, the expiry and the plan, each file's path and the text
    /// appended to it, length-prefixed so no two plans share a message
    fn message(generation: u64, plugin: &str, plan: &[(String, Vec<u8>)], expiry: i64) -> Vec<u8> {
        fn push(message: &mut Vec<u8>, bytes: &[u8]) {
            message.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            message.extend_from_slice(bytes);
        }
        let mut message = TOKEN_DOMAIN.to_vec();
        message.extend_from_slice(&generation.to_be_bytes());
        push(&mut message, plugin.as_bytes());
        message.extend_from_slice(&expiry.to_be_bytes());
        message.extend_from_slice(&(plan.len() as u64).to_be_bytes());
        for (path, text) in plan {
            push(&mut message, path.as_bytes());
            push(&mut message, text);
        }
        message
    }

    /// The token of a preview of `plan` by `plugin` on the ledger at `generation`, at the unix time `now`:
    /// `<generation>.<expiry>.<base64url(HMAC-SHA256)>`, valid for [`TOKEN_TTL_SECS`]. The generation and the
    /// expiry are in the clear so a mismatch can be named, and signed so they cannot be changed
    pub(crate) fn mint(&self, generation: u64, plugin: &str, plan: &[(String, Vec<u8>)], now: i64) -> String {
        let expiry = now + TOKEN_TTL_SECS;
        let tag = self.mac().chain_update(Self::message(generation, plugin, plan, expiry)).finalize().into_bytes();
        format!("{generation}.{expiry}.{}", URL_SAFE_NO_PAD.encode(tag))
    }

    /// Check `token` against the ledger at `generation`, `plugin` and the plan the commit recomputed, at the unix
    /// time `now`: a `conflict` names what differs, the generation, the expiry, or the plan or plugin
    pub(crate) fn verify(&self, token: &str, generation: u64, plugin: &str, plan: &[(String, Vec<u8>)], now: i64) -> Result<(), HostError> {
        let conflict = |message: String| HostError::new(HostErrorKind::Conflict, message);
        let not_a_token = || conflict("the token is not a preview token of this server: preview again".to_owned());
        let (token_generation, expiry, signature) = match token.splitn(3, '.').collect::<Vec<_>>()[..] {
            [generation, expiry, signature] => (
                generation.parse::<u64>().map_err(|_| not_a_token())?,
                expiry.parse::<i64>().map_err(|_| not_a_token())?,
                URL_SAFE_NO_PAD.decode(signature).map_err(|_| not_a_token())?,
            ),
            _ => return Err(not_a_token()),
        };
        if token_generation != generation {
            return Err(conflict(format!(
                "the ledger changed since the preview: it is at generation {generation}, the token is for generation {token_generation}. Preview again"
            )));
        }
        if expiry <= now {
            return Err(conflict(format!(
                "the preview token expired: a token is valid for {} minutes. Preview again",
                TOKEN_TTL_SECS / 60
            )));
        }
        self.mac()
            .chain_update(Self::message(generation, plugin, plan, expiry))
            .verify_slice(&signature)
            .map_err(|_| conflict("the batch is not the one the token was minted for (the text, the file or the plugin differs): preview again".to_owned()))
    }
}

/// the unix time now
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn invalid(message: String) -> HostError {
    HostError::new(HostErrorKind::Invalid, message)
}

/// The generation of the ledger served, when a plugin may write on it: not while it is stale (a write of the API or a
/// plugin is waiting for the reload that follows it), and not after a commit in this generation (one commit per reload
/// cycle). A reload that failed is named: the owner fixes the files
fn writable_generation(ledger: &Ledger, writes: &PluginWrites, reload_sender: &ReloadSender) -> Result<u64, HostError> {
    let generation = reload_sender.generation();
    if ledger.stale || writes.last_commit.load(Ordering::SeqCst) == generation {
        let message = match reload_sender.last_failure() {
            Some(failure) => format!(
                "the ledger failed to reload after a write, so the ledger served is the one loaded before it; fix its files, then preview again: {}",
                failure.message
            ),
            None => "the ledger is reloading after a write: preview again once it has reloaded".to_owned(),
        };
        return Err(HostError::new(HostErrorKind::Conflict, message));
    }
    Ok(generation)
}

/// What `zhang_write_preview` answers `plugin` for `directives` on `ledger`: each entry checked in order, and a token
/// when every entry is clean and there is something to write.
pub(crate) fn preview(
    ledger: &Ledger, writes: &PluginWrites, reload_sender: &ReloadSender, plugin: &str, directives: &[Value],
) -> Result<WritePreview, HostError> {
    let generation = writable_generation(ledger, writes, reload_sender)?;
    let planned = plan_batch(ledger, directives)?;
    let token = (planned.problem.is_none() && !planned.plan.is_empty()).then(|| writes.mint(generation, plugin, &planned.plan, now()));
    Ok(WritePreview {
        ok: planned.problem.is_none(),
        file: planned.file,
        generation,
        entries: planned.entries,
        token,
    })
}

/// What a commit wrote, for the route handler: what the plugin is told, and the files, for the log.
#[derive(Debug, Clone)]
pub(crate) struct Commit {
    pub committed: WriteCommitted,
    /// the files written, each once, in order
    pub files: Vec<String>,
}

/// Append `directives`, previewed by `plugin` as `token` says, to the files of `ledger`: the plan is recomputed from
/// the batch and the token verified under the commit lock, the text goes to the files in one `append`, and the
/// generation moves on. The caller marks the ledger stale and asks for the reload once the plugin returned.
pub(crate) fn commit(
    ledger: &Ledger, writes: &PluginWrites, reload_sender: &ReloadSender, plugin: &str, directives: &[Value], token: &str,
) -> Result<Commit, HostError> {
    writable_generation(ledger, writes, reload_sender)?;
    let planned = plan_batch(ledger, directives)?;
    if let Some(problem) = planned.problem {
        return Err(invalid(problem));
    }
    if planned.plan.is_empty() {
        return Err(invalid("there is nothing to write: every open of the batch is already open".to_owned()));
    }
    // commits of plugins one at a time: the generation and the token are checked again under the lock, so of two
    // commits with tokens of the same generation only the first one lands
    let _appending = writes.append.lock().unwrap_or_else(PoisonError::into_inner);
    let generation = writable_generation(ledger, writes, reload_sender)?;
    writes.verify(token, generation, plugin, &planned.plan, now())?;
    let entries = planned.directives.len();
    ledger
        .data_source
        .append(ledger, planned.directives)
        .map_err(|e| HostError::new(HostErrorKind::Io, format!("the ledger's files could not be written: {e}")))?;
    let generation = reload_sender.next_generation();
    writes.last_commit.store(generation, Ordering::SeqCst);
    let mut files: Vec<String> = vec![];
    for (path, _) in &planned.plan {
        if !files.contains(path) {
            files.push(path.clone());
        }
    }
    Ok(Commit {
        committed: WriteCommitted {
            file: planned.file,
            // what `append` writes: each text on a line of its own, after a blank line
            bytes: planned.plan.iter().map(|(_, text)| text.len() + 2).sum(),
            entries,
            generation,
        },
        files,
    })
}

/// a batch planned on the ledger: what the preview shows and what a commit writes
struct Planned {
    /// the file of the first directive
    file: String,
    entries: Vec<WritePreviewEntry>,
    /// the directives to append, in batch order, the already-open opens left out
    directives: Vec<Directive>,
    /// what `append` writes: each file with the text appended to it, the `include` lines included
    plan: Vec<(String, Vec<u8>)>,
    /// why the batch cannot be committed, the first problem; `None` when every entry is clean
    problem: Option<String>,
}

/// one entry of the batch, checked
struct Checked {
    entry: WritePreviewEntry,
    /// the directive, when it was read: it counts as written for the entries after it
    directive: Option<Directive>,
    /// whether the directive is written; an `open` of an account open already is not
    written: bool,
    problem: Option<String>,
}

/// Plan `directives` on `ledger`: each entry checked in order, the earlier ones counted as written, then the files
/// and texts an `append` of the clean ones writes. The bounds that need the ledger, the exported text's size, are
/// `invalid` here; what the ledger would report against an entry is in the entry.
fn plan_batch(ledger: &Ledger, directives: &[Value]) -> Result<Planned, HostError> {
    let mut overlay: Vec<Directive> = vec![];
    let mut entries = vec![];
    let mut to_write = vec![];
    let mut problem = None;
    let mut text_len = 0;
    for (index, value) in directives.iter().enumerate() {
        let kind = value["kind"]
            .as_str()
            .and_then(WriteKind::parse)
            .ok_or_else(|| invalid(format!("directives[{index}] has no known kind")))?;
        let checked = match kind {
            WriteKind::Transaction => check_transaction(ledger, index, value.clone(), &overlay)?,
            WriteKind::Open => check_open(ledger, index, value.clone(), &overlay)?,
        };
        text_len += checked.entry.text.as_ref().map(String::len).unwrap_or(0);
        if problem.is_none() {
            problem = checked.problem;
        }
        if let Some(directive) = checked.directive {
            if checked.written {
                to_write.push(directive.clone());
            }
            overlay.push(directive);
        }
        entries.push(checked.entry);
    }
    if text_len > MAX_TEXT_LEN {
        return Err(invalid(format!("the text of the batch is {text_len} bytes, over the limit of {MAX_TEXT_LEN}")));
    }
    let plan = append_plan(ledger, to_write.clone())
        .map_err(|e| invalid(format!("the batch cannot be planned: {e}")))?
        .into_iter()
        .map(|(path, directive)| Ok((path, ledger.data_source.export(directive).map_err(export_failed)?)))
        .collect::<Result<Vec<_>, HostError>>()?;
    Ok(Planned {
        file: entries.first().map(|entry| entry.file.clone()).unwrap_or_default(),
        entries,
        directives: to_write,
        plan,
        problem,
    })
}

fn export_failed(error: zhang_core::ZhangError) -> HostError {
    HostError::new(HostErrorKind::Io, format!("the directive cannot be written in the ledger's format: {error}"))
}

/// the file a directive of `directive_type` dated `datetime` goes to, by its path within the ledger
fn file_of(ledger: &Ledger, datetime: Option<chrono::NaiveDateTime>, directive_type: DirectiveType) -> Result<String, HostError> {
    let file = output_file(ledger, datetime, directive_type).map_err(|e| invalid(format!("the ledger's directive_output_path option cannot be used: {e}")))?;
    let within = ledger
        .path_in_ledger(&file)
        .filter(|it| !it.as_os_str().is_empty())
        .ok_or_else(|| invalid(format!("{} is not in the ledger's directory", file.display())))?;
    Ok(slashed(&within))
}

/// `directive` as the ledger's format writes it
fn text_of(ledger: &Ledger, directive: Directive) -> Result<String, HostError> {
    let bytes = ledger.data_source.export(directive).map_err(export_failed)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// the `field_errors` of an entry, as `POST /api/transactions/preview` lists them, and the first as the problem
fn field_errors(index: usize, errors: Vec<FieldError>) -> (Vec<Value>, Option<String>) {
    let problem = errors.first().map(|error| format!("directives[{index}]: {}", error.invalid.message));
    let entities = errors
        .into_iter()
        .map(|error| serde_json::to_value(error.into_entity()).expect("a field error serializes"))
        .collect();
    (entities, problem)
}

/// the entry of the `transaction` at `index`, checked as `POST /api/transactions/preview` checks one, with `overlay`
/// counted as written before it
fn check_transaction(ledger: &Ledger, index: usize, value: Value, overlay: &[Directive]) -> Result<Checked, HostError> {
    let payload: CreateTransactionRequest =
        serde_json::from_value(value).map_err(|e| invalid(format!("directives[{index}] is not a transaction as POST /api/transactions takes one: {e}")))?;
    let at = payload.datetime.in_ledger(&ledger.options.timezone);
    let file = file_of(ledger, Some(at), DirectiveType::Transaction)?;
    let entry = |text, field_errors, unbalanced, errors| WritePreviewEntry {
        index,
        kind: WriteKind::Transaction.name().to_owned(),
        file,
        text,
        field_errors,
        unbalanced,
        errors,
        already_open: None,
    };
    let transaction = match transaction_from_request(payload, ledger, None) {
        Ok(transaction) => transaction,
        Err(errors) => {
            let (field_errors, problem) = field_errors(index, errors);
            return Ok(Checked {
                entry: entry(None, field_errors, None, vec![]),
                directive: None,
                written: false,
                problem,
            });
        }
    };
    let text = text_of(ledger, Directive::Transaction(transaction.clone()))?;
    let check = ledger
        .check_transaction_with(transaction.clone(), None, overlay)
        .map_err(|e| HostError::new(HostErrorKind::Io, format!("the ledger could not check the transaction: {e}")))?;
    let problem = check
        .errors
        .first()
        .map(|(kind, metas)| format!("directives[{index}]: the ledger would report {kind} ({})", metas_text(metas)));
    let errors = check
        .errors
        .into_iter()
        .map(|(error_type, metas)| serde_json::to_value(TransactionPreviewErrorEntity { error_type, metas }).expect("an error serializes"))
        .collect();
    let unbalanced = check.unbalanced.map(|amounts| serde_json::to_value(amounts).expect("amounts serialize"));
    Ok(Checked {
        entry: entry(Some(text), vec![], unbalanced, errors),
        directive: Some(Directive::Transaction(transaction)),
        written: true,
        problem,
    })
}

/// the metas of a ledger error, `key=value` sorted by key
fn metas_text(metas: &std::collections::HashMap<String, String>) -> String {
    let mut pairs: Vec<_> = metas.iter().map(|(key, value)| format!("{key}={value}")).collect();
    pairs.sort();
    pairs.join(", ")
}

/// the entry of the `open` at `index`: read with the rules of a transaction's fields, then checked against the
/// account's state at its date, with `overlay` counted as written before it
fn check_open(ledger: &Ledger, index: usize, value: Value, overlay: &[Directive]) -> Result<Checked, HostError> {
    let payload: OpenAccountRequest =
        serde_json::from_value(value).map_err(|e| invalid(format!("directives[{index}] is not an open as a write batch takes one: {e}")))?;
    let datetime = open_date(&payload.date, ledger).ok().map(|date| date.naive_datetime());
    let file = file_of(ledger, datetime, DirectiveType::Open)?;
    let entry = |text, field_errors, already_open| WritePreviewEntry {
        index,
        kind: WriteKind::Open.name().to_owned(),
        file,
        text,
        field_errors,
        unbalanced: None,
        errors: vec![],
        already_open: Some(already_open),
    };
    let open = match open_from_request(payload, ledger) {
        Ok(open) => open,
        Err(errors) => {
            let (field_errors, problem) = field_errors(index, errors);
            return Ok(Checked {
                entry: entry(None, field_errors, false),
                directive: None,
                written: false,
                problem,
            });
        }
    };
    let check = ledger.check_open(&open, overlay);
    if check.already_open {
        return Ok(Checked {
            entry: entry(None, vec![], true),
            directive: Some(Directive::Open(open)),
            written: false,
            problem: None,
        });
    }
    if let Some(open_with) = check.open_with {
        let with = match open_with.is_empty() {
            true => "for any commodity".to_owned(),
            false => format!("with {}", open_with.join(", ")),
        };
        let requested = match open.commodities.is_empty() {
            true => "any commodity".to_owned(),
            false => open.commodities.join(", "),
        };
        let error = FieldError {
            posting: None,
            field: TransactionField::Commodities,
            invalid: Invalid {
                kind: InvalidKind::AlreadyOpen,
                value: open.commodities.join(", "),
                message: format!(
                    "{} is already open {with}, not {requested}: a plugin cannot change the commodities of an open account",
                    open.account.name()
                ),
            },
        };
        let (field_errors, problem) = field_errors(index, vec![error]);
        return Ok(Checked {
            entry: entry(None, field_errors, false),
            directive: None,
            written: false,
            problem,
        });
    }
    let text = text_of(ledger, Directive::Open(open.clone()))?;
    Ok(Checked {
        entry: entry(Some(text), vec![], false),
        directive: Some(Directive::Open(open)),
        written: true,
        problem: None,
    })
}

/// the date of an `open` request: a date `2026-10-13`, or the ledger's wall-clock date and time, as every request
/// writes one ([`LedgerDateTime`])
fn open_date(text: &str, ledger: &Ledger) -> Result<Date, Invalid> {
    LedgerDateTime::from_str(text)
        .map(|datetime| datetime.ledger_date(&ledger.options.timezone))
        .map_err(|message| Invalid {
            kind: InvalidKind::InvalidDate,
            value: text.to_owned(),
            message,
        })
}

/// The `open` a request describes, with the checks of a transaction's fields: the account and the commodities must
/// read back unchanged (in a beancount ledger, as beancount reads them), the metadata keys too, and the booking
/// method must be one the ledger books with. Every field is checked, in the order date, account, commodities,
/// metadata; the errors are those of each field that fails.
pub(crate) fn open_from_request(payload: OpenAccountRequest, ledger: &Ledger) -> Result<Open, Vec<FieldError>> {
    let rules = validate::Rules::of(ledger);
    let mut errors = vec![];
    let mut check = |field: TransactionField, result: Result<(), Invalid>| {
        if let Err(invalid) = result {
            errors.push(FieldError { posting: None, field, invalid });
        }
    };
    let date = open_date(&payload.date, ledger);
    check(TransactionField::Date, date.as_ref().map(|_| ()).map_err(Clone::clone));
    let account = validate::account(&payload.account, &rules);
    check(TransactionField::Account, account.as_ref().map(|_| ()).map_err(Clone::clone));
    for commodity in &payload.commodities {
        check(TransactionField::Commodities, validate::commodity(commodity, &rules));
    }
    let meta = metas_from_request(payload.metas, &rules, None);
    check(TransactionField::Metas, meta.as_ref().map(|_| ()).map_err(Clone::clone));
    if let Some(method) = &payload.booking_method {
        if BookingMethod::from_str(method).is_err() {
            check(
                TransactionField::Metas,
                Err(Invalid {
                    kind: InvalidKind::InvalidBookingMethod,
                    value: method.clone(),
                    message: format!("invalid booking method {method:?}: it is STRICT, FIFO, LIFO, AVERAGE, AVERAGE_ONLY or NONE"),
                }),
            );
        }
    }
    let (Ok(date), Ok(account), Ok(mut meta), true) = (date, account, meta, errors.is_empty()) else {
        return Err(errors);
    };
    if let Some(method) = payload.booking_method {
        meta.insert(BOOKING_METHOD_META.to_owned(), ZhangString::quote(method));
    }
    Ok(Open {
        date,
        account,
        commodities: payload.commodities,
        meta,
    })
}

#[cfg(test)]
mod test {
    use serde_json::{json, Value};
    use zhang_testkit::ledger::Scratch;

    use super::{PluginWrites, TOKEN_TTL_SECS};
    use crate::ReloadSender;

    const LEDGER: &str = "option \"operating_currency\" \"USD\"\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food USD\n";

    fn reload_sender() -> ReloadSender {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        ReloadSender::new(sender)
    }

    fn plan() -> Vec<(String, Vec<u8>)> {
        vec![("data/2026/10.zhang".to_owned(), b"2026-10-13 open Assets:Bank\n".to_vec())]
    }

    #[test]
    fn a_token_is_bound_to_the_generation_the_plugin_the_plan_and_its_expiry() {
        let writes = PluginWrites::new();
        let token = writes.mint(3, "mcp", &plan(), 1_000);
        assert!(token.starts_with("3."), "{token}");
        assert_eq!(writes.verify(&token, 3, "mcp", &plan(), 1_000 + TOKEN_TTL_SECS - 1), Ok(()));

        let conflict = |token: &str, generation, plugin: &str, plan: &[(String, Vec<u8>)], now| {
            let error = writes.verify(token, generation, plugin, plan, now).unwrap_err();
            assert_eq!(error.kind, super::HostErrorKind::Conflict, "{}", error.message);
            error.message
        };
        assert!(conflict(&token, 4, "mcp", &plan(), 1_000).contains("at generation 4, the token is for generation 3"));
        assert!(conflict(&token, 3, "mcp", &plan(), 1_000 + TOKEN_TTL_SECS).contains("expired"));
        assert!(conflict(&token, 3, "other", &plan(), 1_000).contains("not the one the token was minted for"));
        let other_text = vec![("data/2026/10.zhang".to_owned(), b"2026-10-13 open Assets:Bank USD\n".to_vec())];
        assert!(conflict(&token, 3, "mcp", &other_text, 1_000).contains("not the one the token was minted for"));
        let other_file = vec![("data/2026/11.zhang".to_owned(), plan()[0].1.clone())];
        assert!(conflict(&token, 3, "mcp", &other_file, 1_000).contains("not the one the token was minted for"));
        // another server's key, and something that is no token at all
        assert!(conflict(&PluginWrites::new().mint(3, "mcp", &plan(), 1_000), 3, "mcp", &plan(), 1_000).contains("not the one"));
        for garbage in ["", "3", "3.1600", "x.y.z", "3.1600.not base64!"] {
            assert!(conflict(garbage, 3, "mcp", &plan(), 1_000).contains("not a preview token"), "{garbage}");
        }
    }

    /// what the preview of `directives` answers, `{"Ok": ...}` or `{"Err": ...}`
    fn preview(ledger: &zhang_core::ledger::Ledger, directives: Value) -> Value {
        let writes = PluginWrites::new();
        let reload = reload_sender();
        let directives = directives.as_array().unwrap().clone();
        serde_json::to_value(super::preview(ledger, &writes, &reload, "p", &directives)).unwrap()
    }

    /// the preview of `directives`, which must succeed
    fn ok(ledger: &zhang_core::ledger::Ledger, directives: Value) -> Value {
        let answer = preview(ledger, directives);
        assert!(answer["Ok"].is_object(), "{answer}");
        answer["Ok"].clone()
    }

    #[test]
    fn an_open_is_read_with_the_rules_of_a_transaction_and_checked_against_the_account() {
        let scratch = Scratch::zhang(LEDGER);
        let ledger = scratch.ledger().unwrap();
        let open = |fields: Value| {
            let mut object = json!({"kind": "open", "date": "2026-10-13", "account": "Assets:Bank"});
            object.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
            object
        };

        let clean = ok(
            &ledger,
            json!([open(
                json!({"commodities": ["USD", "EUR"], "booking_method": "FIFO", "metas": [{"key": "note", "value": "new"}]})
            )]),
        );
        assert_eq!(clean["ok"], true);
        assert_eq!(clean["file"], "data/2026/10.zhang");
        assert_eq!(
            clean["entries"][0]["text"],
            "2026-10-13 open Assets:Bank USD, EUR\n  booking_method: \"FIFO\"\n  note: \"new\""
        );
        assert_eq!(clean["entries"][0]["already_open"], false);
        assert!(clean["token"].is_string(), "{clean}");

        // every field is checked, in order, and the entry names the file of its date all the same
        let wrong = ok(
            &ledger,
            json!([open(
                json!({"date": "soon", "account": "Assets", "commodities": ["1USD"], "booking_method": "RANDOM", "metas": [{"key": "a b", "value": "x"}]})
            )]),
        );
        assert_eq!(wrong["ok"], false);
        assert_eq!(wrong["token"], Value::Null);
        assert_eq!(
            wrong["entries"][0]["file"], "main.zhang",
            "a directive without a readable date goes to the main file"
        );
        let kinds: Vec<_> = wrong["entries"][0]["field_errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| (error["field"].as_str().unwrap().to_owned(), error["kind"].as_str().unwrap().to_owned()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("date".to_owned(), "invalid_date".to_owned()),
                ("account".to_owned(), "invalid_account".to_owned()),
                ("commodities".to_owned(), "invalid_commodity".to_owned()),
                // `a b` is a key a zhang ledger writes quoted; a beancount ledger would refuse it
                ("metas".to_owned(), "invalid_booking_method".to_owned()),
            ]
        );

        // already open with the same commodities: nothing to write, and no token for nothing
        let same = ok(&ledger, json!([open(json!({"account": "Expenses:Food", "commodities": ["USD"]}))]));
        assert_eq!((&same["ok"], &same["token"]), (&json!(true), &Value::Null));
        assert_eq!(same["entries"][0]["already_open"], true);
        assert_eq!(same["entries"][0]["text"], Value::Null);
        // open with other commodities: a field error on `commodities`
        let other = ok(&ledger, json!([open(json!({"account": "Expenses:Food", "commodities": ["USD", "EUR"]}))]));
        assert_eq!(other["ok"], false);
        assert_eq!(other["entries"][0]["already_open"], false);
        assert_eq!(other["entries"][0]["field_errors"][0]["kind"], "already_open");
        assert_eq!(
            other["entries"][0]["field_errors"][0]["message"],
            "Expenses:Food is already open with USD, not USD, EUR: a plugin cannot change the commodities of an open account"
        );
        let any = ok(&ledger, json!([open(json!({"account": "Assets:Cash", "commodities": ["USD"]}))]));
        assert_eq!(
            any["entries"][0]["field_errors"][0]["message"],
            "Assets:Cash is already open for any commodity, not USD: a plugin cannot change the commodities of an open account"
        );
        // a wall-clock time is a date and time
        let timed = ok(&ledger, json!([open(json!({"date": "2026-10-13T08:30:00"}))]));
        assert_eq!(timed["entries"][0]["text"], "2026-10-13 08:30:00 open Assets:Bank");
    }

    #[test]
    fn a_batch_is_checked_in_order_with_its_earlier_entries_written() {
        let scratch = Scratch::zhang(LEDGER);
        let ledger = scratch.ledger().unwrap();
        let coffee = json!({"kind": "transaction", "datetime": "2026-10-13", "payee": "Blue Bottle", "narration": "coffee",
            "postings": [{"account": "Expenses:Coffee", "unit": {"number": "4.50", "commodity": "USD"}}, {"account": "Assets:Cash"}],
            "tags": ["mcp"], "links": [], "metas": []});

        let alone = ok(&ledger, json!([coffee]));
        assert_eq!(alone["ok"], false);
        assert_eq!(alone["entries"][0]["errors"][0]["error_type"], "AccountDoesNotExist");
        assert_eq!(alone["entries"][0]["unbalanced"], json!([]));
        assert_eq!(
            alone["entries"][0]["text"],
            "2026-10-13 * \"Blue Bottle\" \"coffee\" #mcp\n  Expenses:Coffee 4.50 USD\n  Assets:Cash"
        );

        let with_open = ok(
            &ledger,
            json!([{"kind": "open", "date": "2026-10-13", "account": "Expenses:Coffee", "commodities": ["USD"]}, coffee]),
        );
        assert_eq!(with_open["ok"], true, "{with_open}");
        assert_eq!(with_open["entries"][1]["errors"], json!([]));
        assert_eq!(with_open["file"], "data/2026/10.zhang");
        assert!(with_open["token"].is_string());

        // a field error names the posting, and a JSON amount cannot even be read as a transaction
        let bad = ok(
            &ledger,
            json!([{"kind": "transaction", "datetime": "2026-10-13", "payee": "x", "postings": [{"account": "Assets:Cash", "unit": "4.50 U$D"}], "tags": [], "links": [], "metas": []}]),
        );
        assert_eq!(bad["entries"][0]["field_errors"][0]["posting"], 0);
        assert_eq!(bad["entries"][0]["field_errors"][0]["field"], "unit");
        assert_eq!(bad["entries"][0]["text"], Value::Null);
        let unreadable = preview(&ledger, json!([{"kind": "transaction", "payee": "x"}]));
        assert_eq!(unreadable["Err"]["kind"], "invalid");
        assert!(
            unreadable["Err"]["message"]
                .as_str()
                .unwrap()
                .starts_with("directives[0] is not a transaction as POST /api/transactions takes one"),
            "{unreadable}"
        );
    }
}
