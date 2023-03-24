use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

#[derive(Debug, EnumString, PartialEq, Eq, strum_macros::ToString, Deserialize, Serialize, Copy, Clone, Hash)]
pub enum AccountType {
    Assets,
    Liabilities,
    Equity,
    Income,
    Expenses,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Account {
    pub account_type: AccountType,
    pub content: String,
    pub components: Vec<String>,
}

impl Account {
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert_eq!(Account::from_str("Assets:A:B").unwrap().name(), "Assets:A:B");
    /// ```
    pub fn name(&self) -> &str {
        &self.content
    }

    /// Return parent account of the given account.
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert_eq!(Account::from_str("Assets:A:B").unwrap().parent().name(), "Assets:A");
    /// ```
    pub fn parent(&self) -> Account {
        let mut parent_components: Vec<String> = self.components[0..self.components.len() - 1].to_vec();
        parent_components.insert(0, self.account_type.to_string());
        let content = parent_components.join(":");
        Account {
            account_type: self.account_type,
            content,
            components: parent_components,
        }
    }

    /// Get the name of the leaf of this account.
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert_eq!(Account::from_str("Assets:A:B").unwrap().leaf(), "B");
    /// assert_eq!(Account::from_str("Assets:A:B:C").unwrap().leaf(), "C");
    /// ```
    pub fn leaf(&self) -> &str {
        &self.components[self.components.len() - 1]
    }

    /// Join a new component for Account
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// let  account = Account::from_str("Assets:A:B").unwrap();
    /// assert_eq!(account.join("C").name(), "Assets:A:B:C");
    /// ```
    pub fn join(&self, component: impl Into<String>) -> Account {
        let component = component.into();
        let mut cloned: Vec<String> = self.components.to_vec();
        cloned.push(component.clone());
        Account {
            account_type: self.account_type,
            content: format!("{}:{}", self.content, component),
            components: cloned,
        }
    }
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// let  account = Account::from_str("Assets:A:B").unwrap();
    /// assert_eq!(account.components(), vec!["A", "B"]);
    /// ```
    pub fn components(&self) -> Vec<&str> {
        self.components.iter().map(Deref::deref).collect()
    }

    /// Return true if the account name is a root account.
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(Account::from_str("Assets:A").unwrap().is_root_account());
    /// assert!(Account::from_str("Income:A").unwrap().is_root_account());
    /// assert!(Account::from_str("Liabilities:A").unwrap().is_root_account());
    /// assert!(!Account::from_str("Liabilities:A:B").unwrap().is_root_account());
    /// assert!(!Account::from_str("Assets:A:B").unwrap().is_root_account());
    /// ```
    pub fn is_root_account(&self) -> bool {
        self.components.len() == 1
    }

    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(Account::from_str("Assets:A").unwrap().is_assets());
    /// assert!(!Account::from_str("Income:A").unwrap().is_assets());
    /// assert!(!Account::from_str("Expenses:A").unwrap().is_assets());
    /// assert!(!Account::from_str("Liabilities:A").unwrap().is_assets());
    /// assert!(!Account::from_str("Equity:A").unwrap().is_assets());
    /// ```
    pub fn is_assets(&self) -> bool {
        matches!(self.account_type, AccountType::Assets)
    }
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(!Account::from_str("Assets:A").unwrap().is_equity());
    /// assert!(!Account::from_str("Income:A").unwrap().is_equity());
    /// assert!(!Account::from_str("Expenses:A").unwrap().is_equity());
    /// assert!(!Account::from_str("Liabilities:A").unwrap().is_equity());
    /// assert!(Account::from_str("Equity:A").unwrap().is_equity());
    /// ```
    pub fn is_equity(&self) -> bool {
        matches!(self.account_type, AccountType::Equity)
    }
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(!Account::from_str("Assets:A").unwrap().is_liabilities());
    /// assert!(!Account::from_str("Income:A").unwrap().is_liabilities());
    /// assert!(!Account::from_str("Expenses:A").unwrap().is_liabilities());
    /// assert!(Account::from_str("Liabilities:A").unwrap().is_liabilities());
    /// assert!(!Account::from_str("Equity:A").unwrap().is_liabilities());
    /// ```
    pub fn is_liabilities(&self) -> bool {
        matches!(self.account_type, AccountType::Liabilities)
    }
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(!Account::from_str("Assets:A").unwrap().is_expenses());
    /// assert!(!Account::from_str("Income:A").unwrap().is_expenses());
    /// assert!(Account::from_str("Expenses:A").unwrap().is_expenses());
    /// assert!(!Account::from_str("Liabilities:A").unwrap().is_expenses());
    /// assert!(!Account::from_str("Equity:A").unwrap().is_expenses());
    /// ```
    pub fn is_expenses(&self) -> bool {
        matches!(self.account_type, AccountType::Expenses)
    }
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(!Account::from_str("Assets:A").unwrap().is_income());
    /// assert!(Account::from_str("Income:A").unwrap().is_income());
    /// assert!(!Account::from_str("Expenses:A").unwrap().is_income());
    /// assert!(!Account::from_str("Liabilities:A").unwrap().is_income());
    /// assert!(!Account::from_str("Equity:A").unwrap().is_income());
    /// ```
    pub fn is_income(&self) -> bool {
        matches!(self.account_type, AccountType::Income)
    }
    /// Return true if the given account is a balance sheet account.
    ///     Assets, liabilities and equity accounts are balance sheet accounts.
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(Account::from_str("Assets:A").unwrap().is_balance_sheet_account());
    /// assert!(!Account::from_str("Income:A").unwrap().is_balance_sheet_account());
    /// assert!(!Account::from_str("Expenses:A").unwrap().is_balance_sheet_account());
    /// assert!(Account::from_str("Liabilities:A").unwrap().is_balance_sheet_account());
    /// assert!(Account::from_str("Equity:A").unwrap().is_balance_sheet_account());
    /// ```
    pub fn is_balance_sheet_account(&self) -> bool {
        self.is_assets() || self.is_liabilities() || self.is_equity()
    }
    /// Return true if the given account is an income statement account.
    ///     Income and expense accounts are income statement accounts.
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(!Account::from_str("Assets:A").unwrap().is_income_statement_account());
    /// assert!(Account::from_str("Income:A").unwrap().is_income_statement_account());
    /// assert!(Account::from_str("Expenses:A").unwrap().is_income_statement_account());
    /// assert!(!Account::from_str("Liabilities:A").unwrap().is_income_statement_account());
    /// assert!(!Account::from_str("Equity:A").unwrap().is_income_statement_account());
    /// ```
    pub fn is_income_statement_account(&self) -> bool {
        self.is_income() || self.is_expenses()
    }
    /// Return true if the given account has inverted signs.
    ///     An inverted sign is the inverse as you'd expect in an external report, i.e.,
    ///     with all positive signs expected.
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert!(!Account::from_str("Assets:A").unwrap().is_invert_account());
    /// assert!(Account::from_str("Income:A").unwrap().is_invert_account());
    /// assert!(!Account::from_str("Expenses:A").unwrap().is_invert_account());
    /// assert!(Account::from_str("Liabilities:A").unwrap().is_invert_account());
    /// assert!(Account::from_str("Equity:A").unwrap().is_invert_account());
    /// ```
    pub fn is_invert_account(&self) -> bool {
        self.is_income() || self.is_liabilities() || self.is_equity()
    }
    /// Return the sign of the normal balance of a particular account.
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert_eq!(Account::from_str("Assets:A").unwrap().get_account_sign(), 1);
    /// assert_eq!(Account::from_str("Income:A").unwrap().get_account_sign(), -1);
    /// assert_eq!(Account::from_str("Expenses:A").unwrap().get_account_sign(), 1);
    /// assert_eq!(Account::from_str("Liabilities:A").unwrap().get_account_sign(), -1);
    /// assert_eq!(Account::from_str("Equity:A").unwrap().get_account_sign(), -1);
    /// ```
    pub fn get_account_sign(&self) -> i8 {
        match self.account_type {
            AccountType::Assets => 1,
            AccountType::Liabilities => -1,
            AccountType::Equity => -1,
            AccountType::Income => -1,
            AccountType::Expenses => 1,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidAccountError;

impl FromStr for Account {
    type Err = InvalidAccountError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();

        let split = parts.split_first();
        if let Some((account_type, rest)) = split {
            Ok(Account {
                account_type: AccountType::from_str(account_type).map_err(|_|InvalidAccountError)?,
                content: s.to_string(),
                components: rest.iter().map(|it| it.to_string()).collect(),
            })
        } else {
            Err(InvalidAccountError)
        }
    }
}
