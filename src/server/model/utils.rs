use async_graphql::OutputType;
use itertools::Itertools;

use super::query::{Paginable, Info};


pub(crate) fn query_paginable<I, T: OutputType, >(data: I, page: usize, size: usize) ->Paginable<T> where I: IntoIterator<Item=T> {

    let data_iter = data.into_iter().collect_vec();
    let total = data_iter.len();
    let ret_data = data_iter.into_iter().skip(size*(page-1)).take(size).collect_vec();
    Paginable {
        data: ret_data,
        page_info: Info {
            page,
            size,
            total
        }
    }
}