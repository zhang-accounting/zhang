// use crate::inventory::{Currency, Inventory};
// use crate::models::{Directive};
// use itertools::Itertools;
// use std::collections::HashMap;
// use crate::data::Transaction;
//
// #[derive(Clone, Debug)]
// pub struct Refer {
//     index: usize,
//     unit_currency: Option<String>,
//     cost_currency: Option<String>,
//     price_currency: Option<String>,
// }
//
// impl Refer {
//     pub fn get_bucket_currency(&self) -> Option<String> {
//         if self.cost_currency.is_some() {
//             self.cost_currency.clone()
//         } else if self.price_currency.is_some() {
//             self.price_currency.clone()
//         } else if self.cost_currency.is_none()
//             && self.price_currency.is_none()
//             && self.unit_currency.is_some()
//         {
//             self.unit_currency.clone()
//         } else {
//             None
//         }
//     }
// }
//
//
// pub fn book(entities: &mut Vec<Directive>) -> Result<Vec<()>, crate::error::ZhangError> {
//     book_full(entities);
//     Ok(vec![])
// }
//
// pub fn book_full(entities: &mut Vec<Directive>) {
//     // let mut new_entities = vec![];
//     let mut error = vec![];
//     let mut balances = HashMap::new();
//
//     for entity in entities {
//         let trx = match entity {
//             Directive::Transaction(trx) => trx,
//             _ => continue,
//         };
//
//         let (refer_groups, cat_errors) = categorize_by_currency(trx, &mut balances);
//         if !cat_errors.is_empty() {
//             error.extend(cat_errors);
//             continue;
//         }
//
//
//     }
// }
// //
// // pub fn replace_currencies(entry: &Transaction, refer_groups: Vec<(String, Vec<Refer>)>) {
// //     let new_groups = vec![];
// //     for (currency, refers) in refer_groups {
// //
// //     }
// // }
//
// pub fn categorize_by_currency(
//     entry: &Transaction,
//     balances: &mut HashMap<Currency, Inventory>,
// ) -> (Vec<(String, Vec<Refer>)>, Vec<String>) {
//     let mut errors = vec![];
//     let mut groups = HashMap::new();
//     let mut sortdict = HashMap::new();
//     let mut auto_postings = vec![];
//     let mut unknown = vec![];
//     for (idx, posting) in entry.lines.iter().enumerate() {
//         let mut unit_currency = posting.amount.clone().map(|it| it.1);
//         let mut cost_currency = posting.cost.clone().map(|it| it.1);
//         let mut price_currency = posting.price.clone().map(|it| it.currency());
//
//         if cost_currency.is_none() && price_currency.is_some() {
//             cost_currency = price_currency.clone()
//         }
//
//         if price_currency.is_none() && cost_currency.is_some() {
//             price_currency = cost_currency.clone()
//         }
//         let refer = Refer {
//             index: idx,
//             unit_currency,
//             cost_currency,
//             price_currency: price_currency.clone(),
//         };
//         if posting.amount.is_none() && price_currency.is_none() {
//             auto_postings.push(refer.clone());
//         } else {
//             let currency = refer.get_bucket_currency();
//
//             if let Some(cry) = currency {
//                 sortdict.entry(cry.clone()).or_insert(idx);
//                 groups
//                     .entry(cry)
//                     .or_insert_with(|| Vec::new())
//                     .push(refer.clone());
//             } else {
//                 // If we need to infer the currency, store in unknown.
//                 unknown.push(refer.clone());
//             }
//         }
//     }
//
//     if unknown.len() == 1 && groups.len() == 1 {
//         let Refer {
//             index,
//             mut unit_currency,
//             mut cost_currency,
//             mut price_currency,
//         } = unknown.pop().unwrap();
//
//         let other_currency = groups.keys().into_iter().next().unwrap();
//         if price_currency.is_none() && cost_currency.is_none() {
//             unit_currency = Some(other_currency.to_string())
//         } else {
//             if price_currency.is_none() {
//                 price_currency = Some(other_currency.to_string())
//             }
//             if cost_currency.is_none() {
//                 cost_currency = Some(other_currency.to_string())
//             }
//         }
//
//         let refer = Refer {
//             index,
//             unit_currency,
//             cost_currency,
//             price_currency,
//         };
//         let currency = refer.get_bucket_currency().expect("currency must be some");
//         sortdict.entry(currency.clone()).or_insert(index);
//         groups
//             .entry(currency)
//             .or_insert_with(|| Vec::new())
//             .push(refer.clone());
//     }
//
//     for refer in unknown {
//         let Refer {
//             index,
//             mut unit_currency,
//             mut cost_currency,
//             mut price_currency,
//         } = refer;
//         let posting = entry
//             .lines
//             .get(index)
//             .expect("index of entry lines out of index");
//
//         let balance = balances
//             .get(&posting.account.to_string())
//             .cloned()
//             .unwrap_or(Inventory::new());
//
//         if unit_currency.is_none() {
//             let mut balance_currencies = balance.currencies();
//             if balance_currencies.len() == 1 {
//                 unit_currency = Some(balance_currencies.pop().unwrap());
//             }
//         }
//         if cost_currency.is_none() || price_currency.is_none() {
//             let mut balance_cost_currencies = balance.cost_currencies();
//             if balance_cost_currencies.len() == 1 {
//                 let balance_cost_currency = balance_cost_currencies.pop().unwrap();
//                 if price_currency.is_none() {
//                     price_currency = Some(balance_cost_currency.clone())
//                 }
//                 if cost_currency.is_none() {
//                     cost_currency = Some(balance_cost_currency)
//                 }
//             }
//         }
//         let refer = Refer {
//             index,
//             unit_currency,
//             cost_currency,
//             price_currency,
//         };
//         let currency = refer.get_bucket_currency();
//         if let Some(cry) = currency {
//             sortdict.entry(cry.clone()).or_insert(index);
//             groups
//                 .entry(cry)
//                 .or_insert_with(|| Vec::new())
//                 .push(refer.clone());
//         } else {
//             errors.push("Failed to categorize posting".to_owned());
//         }
//     }
//
//     for (currency, refers) in &mut groups {
//         for (rindex, mut refer) in refers.into_iter().enumerate() {
//             if refer.unit_currency.is_none() {
//                 let posting = entry
//                     .lines
//                     .get(refer.index)
//                     .expect("posting index out of index");
//                 let balanace = balances.get(&posting.account.to_string());
//                 if let Some(balance) = balanace {
//                     let mut balance_currencies = balance.currencies();
//                     if balance_currencies.len() == 1 {
//                         refer.unit_currency = Some(balance_currencies.pop().unwrap())
//                     }
//                 } else {
//                     continue;
//                 }
//             }
//         }
//     }
//
//     if auto_postings.len() > 1 {
//         let refer = auto_postings.last().unwrap();
//         let option = entry.lines.get(refer.index).unwrap();
//         errors.push("You may not have more than one auto-posting per currency".to_owned());
//         auto_postings = vec![auto_postings.remove(0)];
//     }
//     for refer in auto_postings {
//         let currencies: Vec<Currency> = groups.keys().cloned().collect();
//         for currency in currencies {
//             sortdict.entry(currency.clone()).or_insert(refer.index);
//             groups
//                 .entry(currency.clone())
//                 .or_insert_with(|| Vec::new())
//                 .push(Refer {
//                     index: refer.index,
//                     unit_currency: Some(currency),
//                     cost_currency: None,
//                     price_currency: None,
//                 });
//         }
//     }
//
//     for (currency, refers) in &mut groups {
//         for refer in refers {
//             let posting = entry.lines.get(refer.index).unwrap();
//             for cry in vec![
//                 &refer.unit_currency,
//                 &refer.cost_currency,
//                 &refer.price_currency,
//             ] {
//                 if cry.is_none() {
//                     errors.push("Could not resolve {} currency".to_owned())
//                 }
//             }
//         }
//     }
//
//     let x: Vec<(String, Vec<Refer>)> = groups
//         .into_iter()
//         .sorted_by_key(|(key, value)| sortdict.get(key))
//         .collect();
//
//     (x, errors)
// }
//
// #[cfg(test)]
// mod test {
//     use crate::booking::book;
//
//     #[test]
//     fn test_zero_amount() {
//         let mut entities = parse!(
//             r#"
//         2013-05-18 * ""
//             Assets:Investments:MSFT      0 MSFT
//             Assets:Investments:Cash      0 USD
//         "#
//         );
//
//         let result = book(&mut entities).unwrap();
//         assert_eq!(0, result.len());
//     }
//
//     #[test]
//     fn test_zero_amount_with_cost() {
//         let mut entities = parse!(
//             r#"
//         2013-05-18 * ""
//             Assets:Investments:MSFT      0 MSFT {200.00 USD}
//             Assets:Investments:Cash      1 USD
//         "#
//         );
//
//         let error = book(&mut entities).unwrap();
//         assert_eq!(1, error.len());
//         // todo: check error is  Amount is zero
//     }
//
//     #[test]
//     fn test_cost_zero() {
//         let mut entities = parse!(
//             r#"
//         2013-05-18 * ""
//             Assets:Investments:MSFT      -10 MSFT {0.00 USD}
//             Assets:Investments:Cash  2000.00 USD
//         "#
//         );
//
//         let error = book(&mut entities).unwrap();
//     }
//
//     mod categorize_by_currency {
//         use crate::booking::categorize_by_currency;
//         use crate::models::Directive;
//         use itertools::Itertools;
//         use std::collections::HashMap;
//
//         #[test]
//         fn test_categorize_units_unambiguous() {
//             let mut entries = parse!(
//                 r#"
//             2015-10-02 *
//               Assets:Account  100.00 USD
//               Assets:Other   -100.00 USD
//
//
//             2015-10-02 *
//               Assets:Account
//               Assets:Other   -100.00 USD
//             "#
//             );
//
//             for entry in entries {
//                 let transaction = match entry {
//                     Directive::Transaction(trx) => trx,
//                     _ => unreachable!(),
//                 };
//                 let mut map = HashMap::new();
//                 let (groups, errors) = categorize_by_currency(&transaction, &mut map);
//                 assert_eq!(0, errors.len());
//                 assert_eq!(1, groups.len());
//                 assert_eq!("USA", groups[0].0);
//                 assert_eq!(0, groups[0].1[0].index);
//                 assert_eq!(1, groups[0].1[1].index);
//             }
//         }
//
//         #[test]
//         fn test_categorize_units_price_unambiguous() {
//             let mut entries = parse!(
//                 r#"
//             2015-10-02 *
//                 Assets:Account  100.00 USD @ 1.20 CAD
//                 Assets:Other   -120.00 CAD
//             "#
//             );
//
//             let transaction = match entries.pop().unwrap() {
//                 Directive::Transaction(trx) => trx,
//                 _ => unreachable!(),
//             };
//             let mut map = HashMap::new();
//             let (groups, errors) = categorize_by_currency(&transaction, &mut map);
//             assert_eq!(0, errors.len());
//             assert_eq!(1, groups.len());
//             assert_eq!("CAD", groups[0].0);
//             assert_eq!(0, groups[0].1[0].index);
//             assert_eq!(1, groups[0].1[1].index);
//         }
//
//         #[test]
//         fn test_categorize__units_cost__unambiguous() {
//             let mut entries = parse!(
//                 r#"
//             2015-10-02 *
//                 Assets:Account    10 HOOL {100.00 USD}
//                 Assets:Other   -1000 USD
//             "#
//             );
//
//             let transaction = match entries.pop().unwrap() {
//                 Directive::Transaction(trx) => trx,
//                 _ => unreachable!(),
//             };
//             let mut map = HashMap::new();
//             let (groups, errors) = categorize_by_currency(&transaction, &mut map);
//             assert_eq!(0, errors.len());
//             assert_eq!(1, groups.len());
//             assert_eq!("USD", groups[0].0);
//             assert_eq!(0, groups[0].1[0].index);
//             assert_eq!(1, groups[0].1[1].index);
//         }
//
//         #[test]
//         fn test_categorize__units_cost_price__unambiguous() {
//             let mut entries = parse!(
//                 r#"
//             2015-10-02 *
//                 Assets:Account  10 HOOL {100.00 USD} @ 120.00 USD
//                 Assets:Other
//             "#
//             );
//
//             let transaction = match entries.pop().unwrap() {
//                 Directive::Transaction(trx) => trx,
//                 _ => unreachable!(),
//             };
//             let mut map = HashMap::new();
//             let (groups, errors) = categorize_by_currency(&transaction, &mut map);
//             assert_eq!(0, errors.len());
//             assert_eq!(1, groups.len());
//             assert_eq!("USD", groups[0].0);
//             assert_eq!(0, groups[0].1[0].index);
//             assert_eq!(1, groups[0].1[1].index);
//         }
//
//         #[test]
//         fn test_categorize__multiple_auto_postings() {
//             let mut entries = parse!(
//                 r#"
//             2015-10-02 *
//               Assets:Account   100.00 USD
//               Assets:Account   100.00 CAD
//               Assets:Other
//             "#
//             );
//
//             let transaction = match entries.pop().unwrap() {
//                 Directive::Transaction(trx) => trx,
//                 _ => unreachable!(),
//             };
//             let mut map = HashMap::new();
//             let (groups, errors) = categorize_by_currency(&transaction, &mut map);
//             assert_eq!(0, errors.len());
//             assert_eq!(2, groups.len());
//             assert_eq!("USD", groups[0].0);
//             assert_eq!(0, groups[0].1[0].index);
//             assert_eq!(2, groups[0].1[1].index);
//             assert_eq!("USD", groups[1].0);
//             assert_eq!(1, groups[1].1[0].index);
//             assert_eq!(2, groups[1].1[1].index);
//         }
//     }
// }
