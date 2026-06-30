//! compat-schema-v1 constants — must match docs/nupm-compatibility.md.

pub const COMPAT_SCHEMA_VERSION: u32 = 1;
pub const PINNED_NUPM_REVISION: &str = "421eee1c5ec9a8d751c4480157dcfcabf9d7b963";

pub const METADATA_FILENAME: &str = "nupm.nuon";
pub const BUILD_SCRIPT_NAME: &str = "build.nu";
pub const MODULE_ENTRY: &str = "mod.nu";

pub const MAX_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_TOKEN_COUNT: usize = 4096;
pub const MAX_NESTING_DEPTH: usize = 2;
pub const MAX_RECORD_FIELDS: usize = 16;
pub const MAX_LIST_LENGTH: usize = 64;
pub const MAX_STRING_LEN: usize = 4096;

pub const MAX_PARENT_WALK_HOPS: usize = 32;
pub const MAX_DISCOVERY_ENTRIES: usize = 1024;
pub const MAX_MODULE_TREE_ENTRIES: usize = 1024;

pub const NUPM_IMPORT_ORIGIN: &str = "nupm_import";
pub const NUPM_IMPORT_SELECTION_REASON: &str = "explicit_nupm_import";
pub const NUPM_TRUST_LEVEL: &str = "local_foreign_import";
