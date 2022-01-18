use crate::error::AvaroError;
use serde::{Deserialize, Serialize, Serializer};
use std::ops::Deref;
use std::str::FromStr;
use strum_macros::EnumString;

#[derive(
    Debug,
    EnumString,
    PartialEq,
    Eq,
    strum_macros::ToString,
    Deserialize,
    Serialize,
    Copy,
    Clone,
    Hash,
)]
pub enum AccountType {
    Assets,
    Liabilities,
    Equity,
    Income,
    Expenses,
}

#[derive(Debug, PartialEq)]
pub struct Account {
    pub(crate) account_type: AccountType,
    pub(crate) content: String,
    pub(crate) components: Vec<String>,
}

impl Account {
    // todo add new account method
    /// Return parent account of the given account.
    pub fn parent(&self) -> Account {
        let parent_components: Vec<String> = self.components[0..self.components.len() - 1].to_vec();
        let content = parent_components.join(":");
        Account {
            account_type: self.account_type,
            content,
            components: parent_components,
        }
    }

    /// Get the name of the leaf of this account.
    pub fn leaf(&self) -> &str {
        &self.components[self.components.len() - 1]
    }

    pub fn join(&self, component: impl Into<String>) -> Account {
        let component = component.into();
        let mut cloned: Vec<String> = self.components.iter().cloned().collect();
        cloned.push(component.clone());
        Account {
            account_type: self.account_type,
            content: format!("{}:{}", self.content, component),
            components: cloned,
        }
    }
    pub fn components(&self) -> Vec<&str> {
        self.components.iter().map(Deref::deref).collect()
    }

    /// Return true if the account name is a root account.
    pub fn is_root_account(&self) -> bool {
        self.components.len() == 1
    }

    pub fn is_assets(&self) -> bool {
        matches!(self.account_type, AccountType::Assets)
    }
    pub fn is_equity(&self) -> bool {
        matches!(self.account_type, AccountType::Equity)
    }
    pub fn is_liabilities(&self) -> bool {
        matches!(self.account_type, AccountType::Liabilities)
    }
    pub fn is_expenses(&self) -> bool {
        matches!(self.account_type, AccountType::Expenses)
    }
    pub fn is_income(&self) -> bool {
        matches!(self.account_type, AccountType::Income)
    }
    /// Return true if the given account is a balance sheet account.
    ///     Assets, liabilities and equity accounts are balance sheet accounts.
    pub fn is_balance_sheet_account(&self) -> bool {
        self.is_assets() || self.is_liabilities() || self.is_equity()
    }
    /// Return true if the given account is an income statement account.
    ///     Income and expense accounts are income statement accounts.
    pub fn is_income_statement_account(&self) -> bool {
        self.is_income() || self.is_expenses()
    }
    /// Return true if the given account has inverted signs.
    ///     An inverted sign is the inverse as you'd expect in an external report, i.e.,
    ///     with all positive signs expected.
    pub fn is_invert_account(&self) -> bool {
        self.is_income() || self.is_liabilities() || self.is_equity()
    }
    /// Return the sign of the normal balance of a particular account.
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

impl FromStr for Account {
    type Err = AvaroError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(":").collect();

        let split = parts.split_first();
        if let Some((account_type, rest)) = split {
            Ok(Account {
                account_type: AccountType::from_str(account_type)?,
                content: s.to_string(),
                components: rest.into_iter().map(|it| it.to_string()).collect(),
            })
        } else {
            Err(AvaroError::InvalidAccount)
        }
    }
}

#[cfg(test)]
mod test {
    use crate::core::account::{Account, AccountType};

    #[test]
    fn get_parent() {
        let account = Account {
            account_type: AccountType::Assets,
            content: "Assets:A:B".to_string(),
            components: vec!["Assets".to_owned(), "A".to_owned(), "B".to_owned()],
        };
        let parent = account.parent();

        assert_eq!("Assets:A", parent.content);
        assert_eq!(vec!["Assets", "A"], parent.components);
    }

    #[test]
    fn get_leaf() {
        let account = Account {
            account_type: AccountType::Assets,
            content: "Assets:A:B".to_string(),
            components: vec!["Assets".to_owned(), "A".to_owned(), "B".to_owned()],
        };
        assert_eq!("B", account.leaf());
    }
    #[test]
    fn test_join() {
        let account = Account {
            account_type: AccountType::Assets,
            content: "Assets:A:B".to_string(),
            components: vec!["Assets".to_owned(), "A".to_owned(), "B".to_owned()],
        };
        let child = account.join("C");
        assert_eq!("Assets:A:B:C", child.content);
    }

    #[test]
    fn test_components() {
        let account = Account {
            account_type: AccountType::Assets,
            content: "Assets:A:B".to_string(),
            components: vec!["Assets".to_owned(), "A".to_owned(), "B".to_owned()],
        };
        assert_eq!(vec!["Assets", "A", "B"], account.components());
    }
}
