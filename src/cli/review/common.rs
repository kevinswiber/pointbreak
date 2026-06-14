use std::io::Read;
use std::path::Path;

use clap::ValueEnum;
use shoreline::model::Side;
use shoreline::session::DelegationMap;

/// Discover the checked-in delegation map at `<worktree-root>/.shoreline/delegates`.
///
/// `repo` may be the worktree root or any path inside it, matching what the read
/// commands accept — so discovery resolves the worktree root first (the same root
/// the store resolves against) before joining the delegates path; a non-git
/// context (e.g. an exported bundle) falls back to `repo` as given, per the ADR's
/// "works in any non-git context".
///
/// Presence-based: absent file → `None` (zero-setup stores see zero change). A
/// malformed file is **advisory** — a one-line warning to stderr names the parse
/// error and the read proceeds with `None`, never blocking on resolution config
/// (ADR-0003). Shared by every review read command and the inspector server.
pub(crate) fn discover_delegation_map(repo: &Path) -> Option<DelegationMap> {
    let worktree_root =
        shoreline::git::git_worktree_root(repo).unwrap_or_else(|_| repo.to_path_buf());
    let path = worktree_root.join(".shoreline/delegates");
    if !path.exists() {
        return None;
    }
    match DelegationMap::from_delegates_file(&path) {
        Ok(map) => Some(map),
        Err(error) => {
            eprintln!("warning: ignoring {}: {error}", path.display());
            None
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub(super) enum SideArg {
    Old,
    New,
}

pub(crate) fn read_body_input(
    inline: Option<&str>,
    file: Option<&Path>,
    stdin: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if let Some(inline) = inline {
        return Ok(Some(inline.to_owned()));
    }
    if let Some(path) = file {
        return Ok(Some(std::fs::read_to_string(path)?));
    }
    if stdin {
        let mut body = String::new();
        std::io::stdin().read_to_string(&mut body)?;
        return Ok(Some(body));
    }
    Ok(None)
}

impl From<SideArg> for Side {
    fn from(value: SideArg) -> Self {
        match value {
            SideArg::Old => Side::Old,
            SideArg::New => Side::New,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_body_input_prefers_inline_then_file_then_stdin_false() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let body_path = dir.path().join("body.txt");
        std::fs::write(&body_path, "from file").expect("write body file");

        let body = super::read_body_input(Some("from inline"), Some(&body_path), false)
            .expect("body input resolves");

        assert_eq!(body, Some("from inline".to_string()));
    }
}
