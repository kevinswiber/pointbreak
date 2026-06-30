mod command;
mod ingest;
mod patch;
mod raw;

pub use command::git_worktree_root;
pub(crate) use command::{
    Ancestry, git_commit_tree_oid, git_common_dir, git_config_get, git_for_each_ref, git_head_oid,
    git_head_ref, git_head_tree_oid, git_info_exclude_path, git_is_ancestor, git_object_exists,
    git_path_is_ignored, git_paths_are_ignored, git_rev_list_range, git_rev_list_reachable,
    git_rev_parse_commit_oid, git_tracked_and_untracked_inventory, git_untracked_inventory,
    git_worktree_list,
};
pub use ingest::{IngestOptions, ingest_tracked_diff, ingest_tracked_diff_with_options};
pub(crate) use ingest::{capture_commit_range_diff_files, capture_worktree_diff_files};
