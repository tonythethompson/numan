//! Kill switch for active-plugin update orchestration (Issue #22 PR3).
//!
//! Default **off** (opt-in). When enabled via env, `numan update` may
//! deactivate → upgrade → reactivate an active plugin. When disabled (default),
//! update refuses while a matching `activation` is set.
//!
//! Active-plugin **remove** is always refused regardless of this flag; deactivate
//! first, then remove.

#[cfg(test)]
use std::sync::Mutex;

/// Env: set `NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION` to `1`, `true`, `TRUE`, or `yes`
/// to enable active update orchestration. Any other value (or unset) keeps it off.
pub fn is_enabled() -> bool {
    match std::env::var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION") {
        Ok(v) if matches!(v.as_str(), "1" | "true" | "TRUE" | "yes") => true,
        _ => false, // default off until Issue #22 evidence matrix is green
    }
}

/// Shared mutex for tests that mutate `NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION`.
#[cfg(test)]
pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disabled_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
        assert!(!is_enabled());
    }

    #[test]
    fn enabled_only_for_explicit_opt_in_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        for v in ["1", "true", "TRUE", "yes"] {
            std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", v);
            assert!(is_enabled(), "expected enabled for {v}");
        }
        for v in ["0", "false", "FALSE", "no", "", "on", "TRUE "] {
            std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", v);
            assert!(!is_enabled(), "expected disabled for {v:?}");
        }
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
        assert!(!is_enabled());
    }
}
