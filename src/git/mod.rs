mod command;
mod ingest;
mod patch;
mod raw;

#[cfg(test)]
pub(crate) use command::git_info_exclude_path;
pub use command::git_worktree_root;
pub(crate) use command::{
    Ancestry, git_commit_tree_oid, git_common_dir, git_config_get, git_empty_tree_oid,
    git_for_each_ref, git_head_commit_oid_optional, git_head_oid, git_head_ref, git_is_ancestor,
    git_object_exists, git_path_is_untracked, git_paths_are_ignored, git_rev_list_range,
    git_rev_list_reachable, git_rev_parse_commit_oid, git_tracked_and_untracked_inventory,
    git_untracked_inventory, git_worktree_list, git_write_index_tree_oid,
};
pub use ingest::{IngestOptions, ingest_tracked_diff, ingest_tracked_diff_with_options};
pub(crate) use ingest::{
    capture_commit_range_diff_files, capture_root_commit_diff_files, capture_staged_diff_files,
    capture_unstaged_diff_files, capture_worktree_diff_files,
    capture_worktree_diff_files_from_base,
};
