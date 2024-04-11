use crate::constants::{KEY_FEATURES_PLUGIN, TRUE};

/// [Features] indicates features are not stable, users need to use options to enable the feature
/// the option directive will be like
/// ```zhang
/// option "features.{FEATURE_NAME}" "true"
/// ```
#[derive(Default, Debug)]
pub struct Features {
    pub plugins: bool,
}

impl Features {
    pub fn handle_options(&mut self, key: &str, value: &str) {
        match key {
            s if s == KEY_FEATURES_PLUGIN => self.plugins = value.to_lowercase().eq(TRUE),
            _ => {}
        }
    }
}
