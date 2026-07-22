//! Kill switch for active-plugin update orchestration (Issue #22 PR3).
//!
//! Default **off** (opt-in). When enabled via env, `numan update` may
//! deactivate → upgrade → reactivate an active plugin. When disabled (default),
//! update refuses while a matching `activation` is set.
//!
//! Active-plugin **remove** is always refused regardless of this flag; deactivate
//! first, then remove.

#[cfg(test)]
use std::sync::{Mutex, MutexGuard};

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

/// RAII helper: holds [`ENV_LOCK`], saves prior env, restores on drop.
#[cfg(test)]
pub(crate) struct EnvOptInGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<String>,
}

#[cfg(test)]
impl EnvOptInGuard {
    pub(crate) fn acquire() -> Self {
        let lock = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION").ok();
        Self {
            _lock: lock,
            previous,
        }
    }

    pub(crate) fn set(&self, value: &str) {
        std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", value);
    }

    pub(crate) fn clear(&self) {
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
    }
}

#[cfg(test)]
impl Drop for EnvOptInGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", value),
            None => std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disabled_when_unset() {
        let guard = EnvOptInGuard::acquire();
        guard.clear();
        assert!(!is_enabled());
    }

    #[test]
    fn enabled_only_for_explicit_opt_in_values() {
        let guard = EnvOptInGuard::acquire();
        for v in ["1", "true", "TRUE", "yes"] {
            guard.set(v);
            assert!(is_enabled(), "expected enabled for {v}");
        }
        for v in ["0", "false", "FALSE", "no", "", "on", "TRUE "] {
            guard.set(v);
            assert!(!is_enabled(), "expected disabled for {v:?}");
        }
        guard.clear();
        assert!(!is_enabled());
    }
}
