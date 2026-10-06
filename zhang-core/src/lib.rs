pub use error::ZhangError;

#[macro_use]
pub mod utils;

pub(crate) mod booking;
pub mod clock;
pub mod constants;
pub mod data_source;
pub mod data_type;
pub mod derived;
pub mod domains;
pub mod error;
pub mod inputs;
pub mod inventory;
pub mod ledger;
pub mod options;
pub mod outcome;
pub mod pipeline;
#[cfg(feature = "plugin_runtime")]
pub mod plugin;
pub(crate) mod process;

pub mod features;

pub use zhang_ast as ast;

pub type ZhangResult<T> = Result<T, ZhangError>;

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use serde_json_path::JsonPath;
    use tempfile::tempdir;

    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::domains::schemas::CommodityDomain;
    use crate::ledger::Ledger;

    /// the commodity `name` of the ledger ([`Ledger::commodities`])
    fn commodity(ledger: &Ledger, name: &str) -> Option<CommodityDomain> {
        ledger.commodities().into_iter().map(|(commodity, _)| commodity).find(|it| it.name == name)
    }

    fn load_from_text(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().keep();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        Ledger::load_with_data_source(temp_dir, "example.zhang".to_string(), Arc::new(source)).unwrap()
    }
    fn load_store(content: &str) -> StoreTest {
        let temp_dir = tempdir().unwrap().keep();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        StoreTest {
            ledger: Ledger::load_with_data_source(temp_dir, "example.zhang".to_string(), Arc::new(source)).unwrap(),
        }
    }

    struct StoreTest {
        ledger: Ledger,
    }

    impl StoreTest {
        /// assert the value at `path` of the ledger's options as JSON, `{"options": {key: value}}`
        pub fn assert_string(self, path: &str, expected_data: &str, msg: &str) -> Self {
            let value = serde_json::json!({ "options": self.ledger.options.values });
            let json_path = JsonPath::parse(path).unwrap();
            let node = json_path.query(&value).exactly_one().unwrap().as_str().unwrap();
            assert_eq!(node, expected_data, "{}", msg);
            self
        }
    }

    mod options {
        use indoc::indoc;
        use strum::IntoEnumIterator;

        use crate::options::BuiltinOption;
        use crate::test::{load_from_text, load_store};

        #[test]
        fn should_get_option() -> Result<(), Box<dyn std::error::Error>> {
            load_store(indoc! {r#"
                 option "title" "Example"
            "#})
            .assert_string("$.options.title", "Example", "");
            Ok(())
        }

        #[test]
        fn should_get_latest_option_given_same_options() -> Result<(), Box<dyn std::error::Error>> {
            load_store(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
            "#})
            .assert_string("$.options.title", "Example2", "");
            Ok(())
        }

        #[test]
        fn should_get_default_options() -> Result<(), Box<dyn std::error::Error>> {
            load_store(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
            "#})
            .assert_string("$.options.title", "Example2", "")
            .assert_string("$.options.operating_currency", "CNY", "")
            .assert_string("$.options.default_rounding", "RoundDown", "")
            .assert_string("$.options.default_balance_tolerance_precision", "2", "");
            Ok(())
        }

        #[test]
        fn should_be_override_by_user_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "operating_currency" "USD"
            "#});

            assert_eq!(ledger.options.option::<String>("operating_currency").unwrap().unwrap(), "USD");
            Ok(())
        }

        #[test]
        fn should_get_all_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
                 option "url" "url here"
            "#});
            let options = ledger.options.all();
            assert_eq!(BuiltinOption::iter().count() + 2, options.len());
            assert_eq!(1, options.iter().filter(|it| it.key.eq("title")).count());
            assert_eq!(1, options.iter().filter(|it| it.key.eq("url")).count());
            Ok(())
        }
    }

    mod meta {
        use indoc::indoc;
        use zhang_ast::Directive;

        use crate::test::load_from_text;

        #[test]
        fn should_get_account_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                  a: "b"
            "#});
            let open = ledger.directives.iter().find_map(|it| match &it.data {
                Directive::Open(open) => Some(open),
                _ => None,
            });
            let open = open.unwrap();
            let mut vec = open.meta.clone().sorted_pairs();
            assert_eq!(1, vec.len());
            let (key, value) = vec.pop().unwrap();
            assert_eq!(key, "a");
            assert_eq!(value, "b");
            assert_eq!(open.account.name(), "Assets:MyCard");
            Ok(())
        }

        #[test]
        fn should_keep_posting_meta_per_posting() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
                1970-01-01 open Assets:Cash
                1970-01-01 open Expenses:Food
                  budget: food
                2024-01-01 budget food CNY

                2024-01-02 * "Cafe" "lunch"
                  memo: "m"
                  document: "receipts/transaction.pdf"
                  Assets:Cash -5 CNY
                    receipt: "r1"
                  Expenses:Food 5 CNY
                    document: "receipts/posting.pdf"
                    b: "2"
                    a: "1"
                    a: "0"
            "#});
            assert!(ledger.errors.is_empty(), "{:?}", ledger.errors);
            let (_, txn) = ledger.transactions().into_iter().next().unwrap();
            let pairs = |posting: &zhang_ast::Posting| {
                posting
                    .meta
                    .clone()
                    .sorted_pairs()
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
            };
            let postings = txn.postings.iter().map(pairs).collect::<Vec<_>>();
            // sorted by key, the values of a repeated key in ledger order
            assert_eq!(postings, vec![vec!["receipt=r1"], vec!["a=1", "a=0", "b=2", "document=receipts/posting.pdf"]]);

            // the transaction's own metadata is unchanged
            let mut metas = txn
                .meta
                .clone()
                .sorted_pairs()
                .into_iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>();
            metas.sort();
            assert_eq!(metas, vec!["document=receipts/transaction.pdf", "memo=m"]);

            // Budget ownership remains account metadata; figures are computed by zhang-query.
            let food = ledger.directives.iter().find_map(|it| match &it.data {
                Directive::Open(open) if open.account.name() == "Expenses:Food" => Some(open),
                _ => None,
            });
            assert_eq!(food.unwrap().meta.get_one("budget").unwrap().as_str(), "food");
            Ok(())
        }

        /// Directives go to a WASM plugin, and come back from it, as JSON.
        #[test]
        fn postings_without_meta_from_an_older_plugin_still_deserialise() {
            use zhang_ast::{Directive, Spanned};

            use crate::data_type::text::ZhangDataType;
            use crate::data_type::DataType;

            let directives = ZhangDataType {}
                .transform(
                    "2024-01-02 * \"Cafe\"\n  memo: \"m\"\n  Assets:Cash -5 CNY\n    receipt: \"r1\"\n  Expenses:Food 5 CNY\n".to_owned(),
                    None,
                )
                .unwrap();
            let mut json = serde_json::to_value(&directives).unwrap();
            let postings = json[0]["data"]["Transaction"]["postings"].as_array_mut().unwrap();
            assert_eq!(
                postings[0]["meta"]["inner"]["receipt"][0]["QuoteString"], "r1",
                "a plugin receives the posting metadata"
            );

            // what a plugin built against a zhang-ast without posting metadata returns
            for posting in postings {
                posting.as_object_mut().unwrap().remove("meta");
            }
            let returned: Vec<Spanned<Directive>> = serde_json::from_value(json).expect("an older plugin's directives deserialise");
            let (Directive::Transaction(returned), Directive::Transaction(original)) = (&returned[0].data, &directives[0].data) else {
                unreachable!()
            };
            assert!(returned.postings.iter().all(|posting| posting.meta.clone().get_flatten().is_empty()));
            assert_eq!(returned.meta, original.meta);
            assert_eq!(returned.postings.len(), original.postings.len());
        }
    }
    mod account {
        use chrono::NaiveDateTime;
        use indoc::indoc;
        use zhang_ast::Directive;

        use crate::domains::schemas::AccountStatus;
        use crate::ledger::Ledger;
        use crate::test::{load_from_text, load_store};

        /// the alias the `alias` metadata of the first `open` of `account` gives it
        fn alias(ledger: &Ledger, account: &str) -> Option<String> {
            let open = ledger.directives.iter().find_map(|it| match &it.data {
                Directive::Open(open) if open.account.name() == account => Some(open),
                _ => None,
            });
            open?.meta.get_one("alias").map(|it| it.as_str().to_owned())
        }

        #[test]
        fn should_closed_account() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                1970-01-02 close Assets:MyCard
            "#});

            // its status after every directive, and the alias of its `open`
            assert_eq!(ledger.account_status("Assets:MyCard", NaiveDateTime::MAX), Some(AccountStatus::Close));
            assert_eq!(alias(&ledger, "Assets:MyCard"), None);
            Ok(())
        }

        #[test]
        fn should_get_alias_from_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                  alias: "MyCardAliasName"
            "#});

            assert_eq!(alias(&ledger, "Assets:MyCard").unwrap(), "MyCardAliasName");
            Ok(())
        }

        #[test]
        fn should_return_all_accounts() {
            let ledger = load_store(indoc! {r#"
                1970-01-01 commodity USD
                1970-01-01 open Assets:A
                1970-01-01 open Expenses:A

                1970-01-02 "Apple Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A

                1970-01-02 "Origan Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A
            "#})
            .ledger;
            let result = ledger
                .directives
                .iter()
                .filter_map(|it| match &it.data {
                    Directive::Open(open) => Some(open.account.name().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(result.contains(&"Assets:A".to_owned()));
            assert!(result.contains(&"Expenses:A".to_owned()));
        }
    }

    mod account_balance {
        use std::collections::BTreeMap;

        use bigdecimal::BigDecimal;
        use indoc::indoc;

        use crate::ledger::Ledger;
        use crate::test::load_from_text;

        fn balances(ledger: &Ledger, account: &str) -> BTreeMap<String, BigDecimal> {
            let mut amounts = BTreeMap::new();
            for (_, txn) in ledger.transactions() {
                for units in txn
                    .postings
                    .iter()
                    .filter(|posting| posting.account.name() == account)
                    .filter_map(|posting| posting.units.as_ref())
                {
                    *amounts.entry(units.commodity.clone()).or_default() += &units.number;
                }
            }
            amounts
        }

        #[test]
        fn should_return_zero_balance_given_zero_directive() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
            "#});

            let result = balances(&ledger, "Assets:MyCard");
            assert_eq!(0, result.len());

            Ok(())
        }
        #[test]
        fn should_return_correct_balance_given_txn() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                1970-01-01 open Expenses:Lunch
                1970-01-02 "KFC" "Crazy Thursday"
                  Assets:MyCard -50 CNY
                  Expenses:Lunch 50 CNY
            "#});

            assert_eq!(balances(&ledger, "Expenses:Lunch"), BTreeMap::from([("CNY".to_owned(), BigDecimal::from(50))]));
            assert_eq!(balances(&ledger, "Assets:MyCard"), BTreeMap::from([("CNY".to_owned(), BigDecimal::from(-50))]));
            Ok(())
        }

        #[test]
        fn should_get_correct_balance_after_pad_and_trx() {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
                1970-01-01 open Assets:A
                1970-01-01 open Expenses:B
                1970-01-01 open Equity:Open-Balancing

                2023-01-01 balance Assets:A 3000 CNY with pad Equity:Open-Balancing

                2023-01-02 "Shopping" ""
                    Assets:A -30 CNY
                    Expenses:B
            "#});

            assert_eq!(balances(&ledger, "Assets:A"), BTreeMap::from([("CNY".to_owned(), BigDecimal::from(2970))]));
        }
    }
    mod commodity {
        use indoc::indoc;

        use crate::test::load_from_text;

        #[test]
        fn should_get_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
            "#});

            let commodity = crate::test::commodity(&ledger, "CNY").unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(2, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[test]
        fn should_not_get_non_exist_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
            "#});

            let commodity = crate::test::commodity(&ledger, "USD");
            assert!(commodity.is_none());
            Ok(())
        }

        #[test]
        fn should_get_correct_precision_given_override_default_precision() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "default_commodity_precision" "3"
                1970-01-01 commodity CNY
            "#});

            let commodity = crate::test::commodity(&ledger, "CNY").unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(3, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[test]
        fn should_get_info_from_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
                  precision: "3"
                  prefix: "¥"
                  suffix: "CNY"
            "#});

            let commodity = crate::test::commodity(&ledger, "CNY").unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(3, commodity.precision);
            assert_eq!("¥", commodity.prefix.unwrap());
            assert_eq!("CNY", commodity.suffix.unwrap());
            Ok(())
        }
        #[test]
        fn should_meta_precision_have_higher_priority() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "default_commodity_precision" "3"
                1970-01-01 commodity CNY
                  precision: "4"
            "#});

            let commodity = crate::test::commodity(&ledger, "CNY").unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(4, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[test]
        fn should_work_with_same_default_operating_currency_and_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "operating_currency" "CNY"
                1970-01-01 commodity CNY
                  precision: "4"
            "#});

            let commodity = crate::test::commodity(&ledger, "CNY").unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(4, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }
    }
    mod error {
        use indoc::indoc;
        use zhang_ast::error::ErrorKind;

        use crate::test::load_from_text;

        mod close_non_zero_account {
            use indoc::indoc;
            use zhang_ast::error::ErrorKind;

            use crate::test::load_from_text;

            #[test]
            fn should_not_raise_error() -> Result<(), Box<dyn std::error::Error>> {
                let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard
                    1970-01-03 close Assets:MyCard
                "#});

                let errors = ledger.errors.clone();
                assert_eq!(errors.len(), 0);
                Ok(())
            }
            #[test]
            fn should_raise_error() -> Result<(), Box<dyn std::error::Error>> {
                let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard
                    1970-01-01 open Expenses:Lunch
                    1970-01-02 "KFC" "Crazy Thursday"
                      Assets:MyCard -50 CNY
                      Expenses:Lunch 50 CNY

                    1970-01-03 close Assets:MyCard
                "#});

                let mut errors = ledger.errors.clone();
                assert_eq!(errors.len(), 1);
                let error = errors.pop().unwrap();
                assert_eq!(error.error_type, ErrorKind::CloseNonZeroAccount);
                Ok(())
            }
        }

        #[test]
        fn should_raise_non_balance_error_only() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard CNY
                    1970-01-03 balance Assets:MyCard 10 CNY
                "#});

            let mut errors = ledger.errors.clone();
            assert_eq!(errors.len(), 1);
            let domain = errors.pop().unwrap();
            assert_eq!(domain.error_type, ErrorKind::AccountBalanceCheckError);
            assert_eq!(domain.metas.get("account_name").unwrap(), "Assets:MyCard");
            Ok(())
        }
    }
    mod timezone {
        use indoc::indoc;

        use crate::test::load_from_text;

        #[test]
        fn should_get_system_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard CNY
                "#});

            let timezone: String = ledger.options.option("timezone")?.unwrap();
            assert_eq!(iana_time_zone::get_timezone().unwrap(), timezone);
            Ok(())
        }

        #[test]
        fn should_fallback_to_use_system_timezone_given_invalid_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    option "timezone" "MYZone"
                "#});

            let timezone: String = ledger.options.option("timezone")?.unwrap();
            assert_eq!(iana_time_zone::get_timezone().unwrap(), timezone);
            Ok(())
        }
        #[test]
        fn should_parse_user_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    option "timezone" "Antarctica/South_Pole"
                "#});

            let timezone: String = ledger.options.option("timezone")?.unwrap();
            assert_eq!("Antarctica/South_Pole", timezone);
            assert_eq!(ledger.options.timezone, "Antarctica/South_Pole".parse().unwrap());
            Ok(())
        }

        /// the instants (in UTC) of the stored transactions, keyed by narration
        fn transaction_instants(ledger: &crate::ledger::Ledger) -> std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> {
            let timezone = ledger.options.timezone;
            let transactions = ledger.transactions().into_iter();
            transactions
                .map(|(_, txn)| {
                    let narration = txn.narration.as_ref().map(|it| it.as_str().to_owned()).unwrap_or_default();
                    (narration, txn.date.to_timezone_datetime(&timezone).to_utc())
                })
                .collect()
        }

        fn utc(text: &str) -> chrono::DateTime<chrono::Utc> {
            text.parse().unwrap()
        }

        #[test]
        fn should_load_a_local_time_skipped_by_dst() -> Result<(), Box<dyn std::error::Error>> {
            // 02:30 does not exist in New York on 2023-03-12: clocks jump 02:00 EST -> 03:00 EDT
            let ledger = load_from_text(indoc! {r#"
                option "timezone" "America/New_York"
                option "operating_currency" "USD"

                2023-01-01 open Assets:Cash USD
                2023-01-01 open Expenses:Food USD

                2023-03-12 02:30:00 "gap" "this local time does not exist"
                  Assets:Cash -1 USD
                  Expenses:Food
            "#});

            assert!(ledger.errors.is_empty());
            let instants = transaction_instants(&ledger);
            // read with the offset before the transition: 02:30 EST = 07:30 UTC = 03:30 EDT
            assert_eq!(instants["this local time does not exist"], utc("2023-03-12T07:30:00Z"));

            let postings: usize = ledger.transactions().iter().map(|(_, txn)| txn.postings.len()).sum();
            assert_eq!(postings, 2);
            assert!(transaction_instants(&ledger).values().all(|instant| *instant == utc("2023-03-12T07:30:00Z")));
            Ok(())
        }

        #[test]
        fn should_load_ambiguous_and_midnight_skipped_local_times() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "timezone" "America/New_York"
                option "operating_currency" "USD"

                2023-01-01 open Assets:Cash USD
                2023-01-01 open Expenses:Food USD

                2023-11-05 01:30:00 "overlap" "this local time happens twice"
                  Assets:Cash -1 USD
                  Expenses:Food
            "#});
            assert!(ledger.errors.is_empty());
            // the earlier of 01:30 EDT and 01:30 EST
            assert_eq!(transaction_instants(&ledger)["this local time happens twice"], utc("2023-11-05T05:30:00Z"));

            // Santiago switches at midnight, so 2023-09-03 has no 00:00 for a date-only directive
            let ledger = load_from_text(indoc! {r#"
                option "timezone" "America/Santiago"
                option "operating_currency" "CLP"

                2023-09-03 open Assets:Cash CLP
                2023-09-03 open Expenses:Food CLP

                2023-09-03 "midnight" "this day has no midnight"
                  Assets:Cash -1 CLP
                  Expenses:Food
            "#});
            assert!(ledger.errors.is_empty());
            // 00:00 -04 = 04:00 UTC = 01:00 -03
            assert_eq!(transaction_instants(&ledger)["this day has no midnight"], utc("2023-09-03T04:00:00Z"));
            Ok(())
        }
    }
}
