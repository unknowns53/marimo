use std::fs;
use std::path::{Component, Path, PathBuf};

// フックの中で調べるので、stat の遅い場所でも Claude Code を長く止めないよう、たどる段数に上限を置く。
const MAX_DEPTH: usize = 64;

/// cwd を含む git リポジトリの名前を返す。linked worktree の中では、worktree のフォルダ名ではなく
/// 元のリポジトリのフォルダ名を返す。リポジトリの外なら None を返す。git は呼ばずにファイルだけを読む。
pub fn repo_name(cwd: &Path) -> Option<String> {
    for dir in cwd.ancestors().take(MAX_DEPTH) {
        let dot_git = dir.join(".git");
        let Ok(meta) = fs::metadata(&dot_git) else {
            continue;
        };
        if meta.is_dir() {
            return folder_name(dir);
        }
        if meta.is_file() {
            let main = fs::read_to_string(&dot_git)
                .ok()
                .and_then(|text| gitdir(&text, dir))
                .and_then(|g| main_worktree(&g));
            return main
                .as_deref()
                .and_then(folder_name)
                .or_else(|| folder_name(dir));
        }
    }
    None
}

// .git ファイルの `gitdir: <path>` 行を読む。相対パスは .git ファイルのあるフォルダから数える。
// `..` を残すと親のフォルダ名が取れないので、字面の上で畳む。
fn gitdir(text: &str, dir: &Path) -> Option<PathBuf> {
    let path = text
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|p| !p.is_empty())?;
    let mut out = PathBuf::new();
    for c in dir.join(path).components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

// linked worktree の gitdir は `<元のリポジトリ>/.git/worktrees/<名前>` の形になる。submodule の
// `<親>/.git/modules/<名前>` などほかの形では、.git ファイルのあるフォルダを名前にするので None を返す。
fn main_worktree(gitdir: &Path) -> Option<PathBuf> {
    let worktrees = gitdir.parent()?;
    let dot_git = worktrees.parent()?;
    if worktrees.file_name()? == "worktrees" && dot_git.file_name()? == ".git" {
        dot_git.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

fn folder_name(dir: &Path) -> Option<String> {
    dir.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        (dir, root)
    }

    fn mkdir(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    #[test]
    fn plain_repo_and_its_subfolders() {
        let (_d, root) = tree();
        let repo = root.join("marimo");
        mkdir(&repo.join(".git"));
        mkdir(&repo.join("crates/core/src"));
        assert_eq!(repo_name(&repo).as_deref(), Some("marimo"));
        assert_eq!(
            repo_name(&repo.join("crates/core/src")).as_deref(),
            Some("marimo")
        );
    }

    #[test]
    fn linked_worktree_names_the_main_repo() {
        let (_d, root) = tree();
        let main = root.join("marimo");
        mkdir(&main.join(".git/worktrees/marimo-title"));
        let wt = root.join("marimo-title");
        mkdir(&wt.join("app/src"));
        let gitdir = main.join(".git/worktrees/marimo-title");
        fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        assert_eq!(repo_name(&wt).as_deref(), Some("marimo"));
        assert_eq!(repo_name(&wt.join("app/src")).as_deref(), Some("marimo"));

        let rel = root.join("marimo-rel");
        mkdir(&rel);
        fs::write(
            rel.join(".git"),
            "gitdir: ../marimo/.git/worktrees/marimo-rel\n",
        )
        .unwrap();
        assert_eq!(repo_name(&rel).as_deref(), Some("marimo"));

        let inner = main.join(".claude/worktrees/agent-a1");
        mkdir(&inner);
        fs::write(
            inner.join(".git"),
            "gitdir: ../../../.git/worktrees/agent-a1\n",
        )
        .unwrap();
        assert_eq!(repo_name(&inner).as_deref(), Some("marimo"));
    }

    #[test]
    fn submodule_and_unknown_gitdir_use_the_folder_name() {
        let (_d, root) = tree();
        let sub = root.join("super/vendor/lib");
        mkdir(&sub);
        mkdir(&root.join("super/.git/modules/lib"));
        fs::write(sub.join(".git"), "gitdir: ../../.git/modules/lib\n").unwrap();
        assert_eq!(repo_name(&sub).as_deref(), Some("lib"));

        let odd = root.join("odd");
        mkdir(&odd);
        fs::write(odd.join(".git"), "not a gitdir line\n").unwrap();
        assert_eq!(repo_name(&odd).as_deref(), Some("odd"));
    }

    #[test]
    fn outside_any_repo() {
        let (_d, root) = tree();
        mkdir(&root.join("plain/deeper"));
        assert_eq!(repo_name(&root.join("plain/deeper")), None);
        assert_eq!(repo_name(&root.join("missing/folder")), None);
    }
}
