use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(Debug, EnumString, PartialEq, Eq, Display, Deserialize, Serialize, Copy, Clone, Hash)]
pub enum AccountType {
    Assets,
    Liabilities,
    Equity,
    Income,
    Expenses,
}

impl AccountType {
    pub fn positive_type(&self) -> bool {
        match self {
            AccountType::Assets => true,
            AccountType::Liabilities => false,
            AccountType::Equity => false,
            AccountType::Income => false,
            AccountType::Expenses => true,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Serialize, Deserialize)]
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
        !self.account_type.positive_type()
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
        if self.account_type.positive_type() {
            1
        } else {
            -1
        }
    }
}

/// Whether `c` may appear in a component of an account name, such as `Bank` in `Assets:Bank`: any character but a
/// space, tab, line break, `"`, `:`, `(`, `)` or `,`. The ledger grammar reads account components by this rule, and
/// [`Account::from_str`] accepts exactly the names the grammar reads.
pub fn is_account_component_char(c: char) -> bool {
    !matches!(c, '"' | ':' | '(' | ')' | ',' | ' ' | '\t' | '\n' | '\r')
}

/// A name that is not an account name of the ledger grammar.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct InvalidAccountError {
    /// the rejected name
    pub name: String,
}

impl std::fmt::Display for InvalidAccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} is not an account name: an account is `Assets`, `Liabilities`, `Equity`, `Income` or `Expenses` followed by one or more `:`-separated components, and a component cannot be empty or contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`",
            self.name
        )
    }
}

impl std::error::Error for InvalidAccountError {}

impl FromStr for Account {
    type Err = InvalidAccountError;

    /// Read `s` as an account name by the rule of the ledger grammar: an account type and at least one non-empty
    /// component of [`is_account_component_char`] characters. `Assets`, `Assets:`, `Assets::Bank` and `Assets:My Bank`
    /// are not account names, as a ledger could not read them back.
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use zhang_ast::Account;
    /// assert_eq!(Account::from_str("Assets:Bank:Checking").unwrap().components(), vec!["Bank", "Checking"]);
    /// assert!(Account::from_str("Assets").is_err());
    /// assert!(Account::from_str("Assets:My Bank").is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidAccountError { name: s.to_owned() };
        let mut parts = s.split(':');
        let account_type = parts.next().and_then(|it| AccountType::from_str(it).ok()).ok_or_else(invalid)?;
        let components: Vec<String> = parts.map(str::to_owned).collect();
        let valid_component = |component: &String| !component.is_empty() && component.chars().all(is_account_component_char);
        if components.is_empty() || !components.iter().all(valid_component) {
            return Err(invalid());
        }
        Ok(Account {
            account_type,
            content: s.to_owned(),
            components,
        })
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use crate::Account;

    /// A name the ledger grammar cannot read back is no account: a root alone, an empty component, or a component with
    /// a space, quote, comma or paren
    #[test]
    fn from_str_rejects_what_the_grammar_rejects() {
        for name in [
            "",
            "Assets",
            "Assets:",
            "Assets::Bank",
            "Assets:Bank:",
            "Assets:My Bank",
            "Assets:a\"b",
            "Assets:a,b",
            "Assets:(x)",
            "assets:x",
            "Bank:X",
        ] {
            let error = Account::from_str(name).unwrap_err();
            assert_eq!(error.name, name);
            assert!(error.to_string().contains("is not an account name"), "{error}");
        }
        let account = Account::from_str("Equity:Opening-Balances:中文").unwrap();
        assert_eq!(account.components(), vec!["Opening-Balances", "中文"]);
        assert_eq!(account.name(), "Equity:Opening-Balances:中文");
    }
}
