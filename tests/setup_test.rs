//! `numan setup loader` integration tests.

use numan_cli::cmd::setup::{config_already_sources_loader, execute_loader_with_probe, LoaderArgs};

#[test]
fn setup_loader_install_and_configure_without_live_nu() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.nu");
    std::fs::write(&config_path, "# user config\n").unwrap();

    let args = LoaderArgs {
        force: false,
        configure: true,
        yes: true,
    };

    execute_loader_with_probe(&args, || Ok(config_path.clone())).unwrap();

    let loader_path = dir.path().join("loader.nu");
    assert!(loader_path.is_file());
    let loader = std::fs::read_to_string(&loader_path).unwrap();
    assert!(loader.contains("aidnem_loader_configs"));
    assert!(loader.contains("github.com/aidnem/nushell-loader"));

    let config = std::fs::read_to_string(&config_path).unwrap();
    assert!(config_already_sources_loader(&config));
}
