pub mod activate;
pub mod completions;
pub mod deactivate;
pub mod doctor;
pub mod gc;
pub mod info;
pub mod init;
pub mod install;
pub mod list;
pub mod nupm;
pub mod registry;
pub mod remove;
pub mod search;
pub mod setup;
pub mod snapshot;
pub mod update;

use crate::state::autoload_recovery::AutoloadRecoveryOutcome;

fn print_autoload_recovery(outcome: AutoloadRecoveryOutcome) {
    match outcome {
        AutoloadRecoveryOutcome::NoJournal => {}
        AutoloadRecoveryOutcome::PreparedCleared => {
            eprintln!("   Module journal cleared (no external change occurred).");
        }
        AutoloadRecoveryOutcome::ReplacedCompleted => {
            eprintln!("   Module journal recovery complete.");
        }
    }
}
