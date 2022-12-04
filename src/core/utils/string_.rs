use crate::core::models::ZhangString;

pub trait StringExt {
    fn to_quote(&self) -> ZhangString;
    fn to_unquote(&self) -> ZhangString;
    fn into_quote(self) -> ZhangString;
    fn into_unquote(self) -> ZhangString;
}


impl StringExt for String {
    fn to_quote(&self) -> ZhangString {
        ZhangString::QuoteString(self.to_owned())
    }

    fn to_unquote(&self) -> ZhangString {
        ZhangString::UnquoteString(self.to_owned())
    }

    fn into_quote(self) -> ZhangString {
        ZhangString::QuoteString(self)
    }

    fn into_unquote(self) -> ZhangString {
        ZhangString::UnquoteString(self)
    }
}