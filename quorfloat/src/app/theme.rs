//! Which palette the panel asks for.
//!
//! The palettes themselves are the frontend's, as CSS custom properties (the `--q-*`
//! tokens, taken from the design verbatim). What stays here is the *preference* — the one
//! value the settings file and the host configuration both carry, and the one the shell
//! must keep reading.

/// Which palette the panel is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    /// Follow the platform, which is what a panel floating over other apps should do.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl Preference {
    /// Read the preference from the environment.
    ///
    /// An unreadable or unknown value means [`Preference::System`]: a typo in an
    /// environment variable should not decide how the panel looks.
    ///
    /// @returns the configured preference.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_name(std::env::var("DSH_QUORFLOAT_THEME").ok().as_deref())
    }

    /// Read a preference from its name, which is also how the host will send it.
    ///
    /// @param name - `light`, `dark`, `system`, or anything else.
    /// @returns the matching preference, defaulting to following the system.
    #[must_use]
    pub fn from_name(name: Option<&str>) -> Self {
        match name.map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("light") => Self::Light,
            Some(value) if value.eq_ignore_ascii_case("dark") => Self::Dark,
            _ => Self::System,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_the_host_and_the_settings_file_write_are_read_back() {
        assert_eq!(Preference::from_name(Some("light")), Preference::Light);
        assert_eq!(Preference::from_name(Some(" Dark ")), Preference::Dark);
        assert_eq!(Preference::from_name(Some("system")), Preference::System);
    }

    #[test]
    fn an_unknown_value_is_not_a_decision() {
        // A typo must not decide how the panel looks.
        assert_eq!(Preference::from_name(Some("chartreuse")), Preference::System);
        assert_eq!(Preference::from_name(None), Preference::System);
    }
}
