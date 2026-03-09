//! Helpers for resolving paths inside the Copilot data directory.

use std::path::{Path, PathBuf};

/// Default Copilot data directory (`~/.copilot`).
pub fn default_copilot_dir() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".copilot"))
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory; provide --copilot-dir"))
}

/// Path to `session-store.db` inside the Copilot data directory.
pub fn session_store_path(copilot_dir: &Path) -> PathBuf {
    copilot_dir.join("session-store.db")
}

/// Path to `session-state/` inside the Copilot data directory.
pub fn session_state_dir(copilot_dir: &Path) -> PathBuf {
    copilot_dir.join("session-state")
}

/// Path to the bridge stats cache DB inside the Copilot data directory.
pub fn stats_cache_db_path(copilot_dir: &Path) -> PathBuf {
    copilot_dir.join("remo-stats-cache.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_child_paths_are_derived_from_copilot_dir() {
        let base = Path::new("/tmp/custom-copilot");
        assert_eq!(session_store_path(base), base.join("session-store.db"));
        assert_eq!(session_state_dir(base), base.join("session-state"));
        assert_eq!(stats_cache_db_path(base), base.join("remo-stats-cache.db"));
    }
}
