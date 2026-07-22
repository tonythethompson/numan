//! Kill switch for active-plugin update orchestration (Issue #22 PR3).
//!
//! When enabled (default), `numan update` may deactivate → upgrade → reactivate
//! an active plugin. When disabled, update refuses while `activation` is set.
//!
//! Active-plugin **remove** is always refused regardless of this flag; deactivate
//! first, then remove.

/// Env: `NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION=0` (or `false` / `FALSE` / `no`)
/// disables active update orchestration.
pub fn is_enabled() -> bool {
    match std::env::var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION") {
        Ok(v) if matches!(v.as_str(), "0" | "false" | "FALSE" | "no") => false,
        _ => true, // default on: deactivate + update + activate path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_enabled_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
        assert!(is_enabled());
    }

    #[test]
    fn disabled_by_zero_false_no() {
        let _guard = ENV_LOCK.lock().unwrap();
        for v in ["0", "false", "FALSE", "no"] {
            std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", v);
            assert!(!is_enabled(), "expected disabled for {v}");
        }
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
        assert!(is_enabled());
    }

    #[test]
    fn enabled_for_other_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        for v in ["1", "true", "yes", ""] {
            std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", v);
            assert!(is_enabled(), "expected enabled for {v:?}");
        }
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
    }
}
