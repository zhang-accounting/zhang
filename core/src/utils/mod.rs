pub mod bigdecimal_ext;
pub mod date_range;
pub mod price_grip;
pub mod string_;

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}
