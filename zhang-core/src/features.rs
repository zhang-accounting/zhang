use crate::constants::{KEY_FEATURES_PLUGIN, KEY_FEATURES_PLUGINS, TRUE};

/// [Features] indicates features are not stable, users need to use options to enable the feature
/// the option directive will be like
/// ```zhang
/// option "features.{FEATURE_NAME}" "true"
/// ```
/// plugins are enabled by `features.plugin`, or its alias `features.plugins`
#[derive(Default, Debug)]
pub struct Features {
    pub plugins: bool,
}

impl Features {
    pub fn handle_options(&mut self, key: &str, value: &str) {
        match key {
            KEY_FEATURES_PLUGIN | KEY_FEATURES_PLUGINS => self.plugins = value.to_lowercase().eq(TRUE),
            _ => {}
        }
    }
}

#[cfg(test)]
mod test {
    use crate::features::Features;

    fn plugins_after(options: &[(&str, &str)]) -> bool {
        let mut features = Features::default();
        for (key, value) in options {
            features.handle_options(key, value);
        }
        features.plugins
    }

    #[test]
    fn should_enable_plugins_by_either_spelling() {
        assert!(!plugins_after(&[]));
        assert!(plugins_after(&[("features.plugin", "true")]));
        assert!(plugins_after(&[("features.plugins", "TRUE")]));
        assert!(!plugins_after(&[("features.plugins", "yes")]));
    }

    #[test]
    fn should_let_the_last_option_win() {
        assert!(!plugins_after(&[("features.plugin", "true"), ("features.plugins", "false")]));
        assert!(plugins_after(&[("features.plugins", "false"), ("features.plugin", "true")]));
    }
}
