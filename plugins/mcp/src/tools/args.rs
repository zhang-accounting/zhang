//! The arguments of a tool: declared once, they give both the tool's `inputSchema` and the validation of a call.
//! An argument that fails its declaration is a tool execution error (`isError: true`) naming the argument and what it
//! expects, so the model can correct the call: the specification's "Error Handling" of tools keeps input validation
//! errors ("date in wrong format, value out of range") for tool results, not for JSON-RPC errors (SEP-1303).

use serde_json::{json, Map, Value};

/// what an argument holds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Str,
    /// `YYYY-MM-DD`
    Date,
    /// an integer of at least `min`
    Int {
        min: i64,
    },
    /// one of these strings
    Enum(&'static [&'static str]),
    /// a list of strings
    StrList,
}

/// one argument of a tool
#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub name: &'static str,
    pub description: &'static str,
    pub kind: Kind,
    pub required: bool,
}

impl Arg {
    pub const fn optional(name: &'static str, kind: Kind, description: &'static str) -> Arg {
        Arg {
            name,
            description,
            kind,
            required: false,
        }
    }

    pub const fn required(name: &'static str, kind: Kind, description: &'static str) -> Arg {
        Arg {
            name,
            description,
            kind,
            required: true,
        }
    }

    /// the JSON Schema of this argument
    fn schema(&self) -> Value {
        let mut schema = match self.kind {
            Kind::Str => json!({"type": "string"}),
            Kind::Date => json!({"type": "string", "format": "date", "pattern": "^\\d{4}-\\d{2}-\\d{2}$"}),
            Kind::Int { min } => json!({"type": "integer", "minimum": min}),
            Kind::Enum(values) => json!({"type": "string", "enum": values}),
            Kind::StrList => json!({"type": "array", "items": {"type": "string"}}),
        };
        schema["description"] = json!(self.description);
        schema
    }
}

/// the `inputSchema` of a tool with these arguments: an object, nothing else allowed
pub fn input_schema(args: &[Arg]) -> Value {
    let mut schema = json!({"type": "object", "additionalProperties": false});
    if !args.is_empty() {
        schema["properties"] = args.iter().map(|arg| (arg.name.to_owned(), arg.schema())).collect::<Map<_, _>>().into();
    }
    let required: Vec<&str> = args.iter().filter(|arg| arg.required).map(|arg| arg.name).collect();
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

/// the arguments of one call, validated against their declarations
#[derive(Debug, Clone, Default)]
pub struct Args {
    values: Map<String, Value>,
}

impl Args {
    /// Read `arguments` for a tool declaring `args`: an unknown name, a missing required argument or a value of the
    /// wrong shape is an error message naming the argument. `null` is the same as absent.
    pub fn validate(args: &[Arg], arguments: &Map<String, Value>) -> Result<Args, String> {
        let mut values = Map::new();
        for (name, value) in arguments {
            let Some(arg) = args.iter().find(|arg| arg.name == name) else {
                let known: Vec<String> = args.iter().map(|arg| format!("`{}`", arg.name)).collect();
                return Err(format!(
                    "unknown argument `{name}`; the arguments are {}",
                    if known.is_empty() { "none".to_owned() } else { known.join(", ") }
                ));
            };
            if value.is_null() {
                continue;
            }
            check(arg, value)?;
            values.insert(name.clone(), value.clone());
        }
        for arg in args.iter().filter(|arg| arg.required) {
            if !values.contains_key(arg.name) {
                return Err(format!("the argument `{}` is required", arg.name));
            }
        }
        Ok(Args { values })
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(Value::as_str)
    }

    /// a string argument, or `default` when it is absent
    pub fn str_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.str(name).unwrap_or(default)
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.values.get(name).and_then(Value::as_i64)
    }

    pub fn list(&self, name: &str) -> Option<Vec<String>> {
        let items = self.values.get(name)?.as_array()?;
        Some(items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
    }
}

fn check(arg: &Arg, value: &Value) -> Result<(), String> {
    let wrong = |expected: &str| format!("the argument `{}` must be {expected}", arg.name);
    match arg.kind {
        Kind::Str => value.as_str().map(|_| ()).ok_or_else(|| wrong("a string")),
        Kind::Date => match value.as_str() {
            Some(text) if is_date(text) => Ok(()),
            _ => Err(wrong("a date written YYYY-MM-DD")),
        },
        Kind::Int { min } => match value.as_i64() {
            Some(it) if it >= min => Ok(()),
            _ => Err(wrong(&format!("an integer of at least {min}"))),
        },
        Kind::Enum(values) => match value.as_str() {
            Some(text) if values.contains(&text) => Ok(()),
            _ => Err(wrong(&format!(
                "one of {}",
                values.iter().map(|it| format!("`{it}`")).collect::<Vec<_>>().join(", ")
            ))),
        },
        Kind::StrList => match value.as_array() {
            Some(items) if items.iter().all(Value::is_string) => Ok(()),
            _ => Err(wrong("a list of strings")),
        },
    }
}

/// whether `text` is a calendar date written `YYYY-MM-DD`
pub fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let number = |range: std::ops::Range<usize>| {
        let part = &text[range];
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse::<u32>().ok()).flatten()
    };
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::{input_schema, is_date, Arg, Args, Kind};

    const ARGS: [Arg; 5] = [
        Arg::required("account", Kind::Str, "an account"),
        Arg::optional("from", Kind::Date, "a date"),
        Arg::optional("limit", Kind::Int { min: 1 }, "rows"),
        Arg::optional("group_by", Kind::Enum(&["account", "month"]), "grouping"),
        Arg::optional("tags", Kind::StrList, "tags"),
    ];

    fn validate(arguments: serde_json::Value) -> Result<Args, String> {
        Args::validate(&ARGS, arguments.as_object().unwrap())
    }

    #[test]
    fn the_schema_follows_the_declarations() {
        assert_eq!(
            input_schema(&ARGS),
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "account": {"type": "string", "description": "an account"},
                    "from": {"type": "string", "format": "date", "pattern": "^\\d{4}-\\d{2}-\\d{2}$", "description": "a date"},
                    "limit": {"type": "integer", "minimum": 1, "description": "rows"},
                    "group_by": {"type": "string", "enum": ["account", "month"], "description": "grouping"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "tags"},
                },
                "required": ["account"],
            })
        );
        assert_eq!(input_schema(&[]), json!({"type": "object", "additionalProperties": false}));
    }

    #[test]
    fn validates_every_kind() {
        let ok = validate(json!({"account": "Assets", "from": "2024-02-29", "limit": 5, "group_by": "month", "tags": ["a"]})).unwrap();
        assert_eq!(
            (ok.str("account"), ok.int("limit"), ok.str_or("group_by", "x")),
            (Some("Assets"), Some(5), "month")
        );
        assert_eq!(ok.list("tags"), Some(vec!["a".to_owned()]));
        // null is absent
        let nulls = validate(json!({"account": "Assets", "from": null})).unwrap();
        assert_eq!(nulls.str("from"), None);

        assert_eq!(validate(json!({})).unwrap_err(), "the argument `account` is required");
        assert_eq!(
            validate(json!({"account": "Assets", "nope": 1})).unwrap_err(),
            "unknown argument `nope`; the arguments are `account`, `from`, `limit`, `group_by`, `tags`"
        );
        assert_eq!(validate(json!({"account": 1})).unwrap_err(), "the argument `account` must be a string");
        assert_eq!(
            validate(json!({"account": "a", "from": "2024-2-9"})).unwrap_err(),
            "the argument `from` must be a date written YYYY-MM-DD"
        );
        assert_eq!(
            validate(json!({"account": "a", "from": "2023-02-29"})).unwrap_err(),
            "the argument `from` must be a date written YYYY-MM-DD"
        );
        assert_eq!(
            validate(json!({"account": "a", "limit": 0})).unwrap_err(),
            "the argument `limit` must be an integer of at least 1"
        );
        assert_eq!(
            validate(json!({"account": "a", "limit": "5"})).unwrap_err(),
            "the argument `limit` must be an integer of at least 1"
        );
        assert_eq!(
            validate(json!({"account": "a", "group_by": "week"})).unwrap_err(),
            "the argument `group_by` must be one of `account`, `month`"
        );
        assert_eq!(
            validate(json!({"account": "a", "tags": "x"})).unwrap_err(),
            "the argument `tags` must be a list of strings"
        );
        assert_eq!(
            validate(json!({"account": "a", "tags": [1]})).unwrap_err(),
            "the argument `tags` must be a list of strings"
        );
    }

    #[test]
    fn dates() {
        assert!(is_date("2024-02-29"));
        assert!(is_date("1900-02-28"));
        assert!(!is_date("1900-02-29"));
        assert!(is_date("2000-02-29"));
        assert!(!is_date("2024-13-01"));
        assert!(!is_date("2024-04-31"));
        assert!(!is_date("2024-00-10"));
        assert!(!is_date("24-01-01"));
        assert!(!is_date("2024/01/01"));
        assert!(!is_date("2024-01-01T00:00"));
        assert!(!is_date("２024-01-01"));
    }
}
