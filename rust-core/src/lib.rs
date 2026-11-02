mod adapter;
mod bundle_fs;
mod bundle_index;
mod catalog;
mod database;
mod desktop_state;
mod job;
mod journal;
mod platform;
mod rpc;
mod storage;
mod transfer;
pub use adapter::{all_threads, catalog_json, roots, valid_name, Adapter};
mod rollout;
pub use job::Job;
pub use journal::Journal;
pub use platform::{check_version, command_output, desktop_closed, find_codex};
pub use rollout::{prepare_rollout, prepare_rollout_with_job, rollout_target, ThreadImport};
pub use storage::{
    absolute, discover_stores, discover_stores_with_job, ensure_plain, read_store, resolve_store,
    Store,
};
pub use transfer::{
    copy_file, copy_tree, digest, safe_destination, scan_tree, tree_size, write_json_new, Entry,
    Tree,
};
pub use transfer::{rename_new, resume_copy_tree};

pub use bundle_fs::{load_verified_bundle, verified_bundle, verify_bundle, VerifiedBundle};
mod backup;
mod deletion;
mod file_manager;
mod runtime;
pub use runtime::{protected_workspaces, runtime_roots, runtime_roots_with_job};
mod selection;
mod verified_cleanup;
pub use verified_cleanup::remove_verified_tree;
mod undo;
pub use selection::expand_thread_ids;
mod workflow;
pub use backup::{
    backup_project, clean_path, discover_bundles, manifest_tree, map_workspace, paths_overlap,
};
pub use bundle_index::{valid_bundle_path, validate_bundle_index};
pub use catalog::{
    merge_legacy_assignments, merge_legacy_projects, organize_catalog, Catalog, ProjectGroup,
};
pub use database::{read_database_catalog, read_database_catalog_with_state};
pub use file_manager::{file_identity, identity, list_files, signature, FILE_RECOVERY};
pub use rpc::{
    collect_pages, read_response, thread_list_params, CodexProcess, ProjectFilter, RpcClient,
    TimedReader,
};

mod cross_store;
pub use cross_store::{
    cross_store_conclude, cross_store_conclude_plan, cross_store_plan, cross_store_resume,
    cross_store_transfer,
};
