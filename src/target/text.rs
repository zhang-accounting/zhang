use crate::account::Account;
use crate::amount::Amount;
use crate::data::{Posting, Transaction};
use crate::models::Flag;
use crate::target::AvaroTarget;
use crate::to_file::ToAvaroFile;
use crate::utils::escape_with_quote;
use itertools::Itertools;
use std::fmt::format;

impl AvaroTarget<String> for Flag {
    fn to_target(self) -> String {
        match self {
            Flag::Okay => "*".to_owned(),
            Flag::Warning => "!".to_owned(),
        }
    }
}

impl AvaroTarget<String> for Account {
    fn to_target(self) -> String {
        self.content
    }
}
impl AvaroTarget<String> for Amount {
    fn to_target(self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

impl AvaroTarget<String> for Transaction {
    fn to_target(self) -> String {
        let mut vec1 = vec![
            Some(self.date.format("%Y-%m-%d %H:%M:%S").to_string()),
            self.flag.map(|it| format!(" {}", it.to_target())),
            self.payee.map(|it| escape_with_quote(&it).to_string()),
            self.narration.map(|it| escape_with_quote(&it).to_string()),
        ];
        let mut tags = self
            .tags
            .into_iter()
            .map(|it| Some(format!("#{}", it)))
            .collect_vec();
        let mut links = self
            .links
            .into_iter()
            .map(|it| Some(format!("^{}", it)))
            .collect_vec();
        vec1.append(&mut tags);
        vec1.append(&mut links);

        let mut vec2 = self
            .postings
            .into_iter()
            .map(|it| format!("  {}", it.to_target()))
            .collect_vec();
        vec2.insert(0, vec1.into_iter().flatten().join(" "));
        vec2.into_iter().join("\n")
    }
}

impl AvaroTarget<String> for Posting {
    fn to_target(self) -> String {
        // todo cost and price
        let vec1 = vec![
            self.flag.map(|it| format!(" {}", it.to_target())),
            Some(self.account.to_target()),
            self.units.map(|it| it.to_target()),
        ];

        vec1.into_iter().flatten().join(" ")
    }
}
