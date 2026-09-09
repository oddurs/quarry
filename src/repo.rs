//! Attributing a process to a git repository.
//!
//! Deliberately does not shell out to git: reading `.git` directly keeps a
//! whole-machine scan under a millisecond, and it means every case here —
//! linked worktrees, detached heads, submodules — is testable by building a
//! directory in a temp dir rather than by driving a real git.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::Repo;

/// Caches by directory, so twenty worktrees of one repo cost one walk each and
/// twenty processes in one directory cost one walk total.
#[derive(Default)]
pub struct Resolver {
    cache: HashMap<PathBuf, Option<Repo>>,
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resolve(&mut self, start: &Path) -> Option<Repo> {
        if let Some(hit) = self.cache.get(start) {
            return hit.clone();
        }
        let found = find(start);
        self.cache.insert(start.to_path_buf(), found.clone());
        found
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// Walk up from `start` looking for a work tree.
pub fn find(start: &Path) -> Option<Repo> {
    let mut dir = Some(start);
    // A depth bound stops a pathological path (or a symlink loop that `parent`
    // somehow survives) from turning a scan into a hang.
    for _ in 0..64 {
        let d = dir?;
        let dot = d.join(".git");
        if dot.exists() {
            return Some(read(d, &dot));
        }
        dir = d.parent();
    }
    None
}

fn read(work_tree: &Path, dot: &Path) -> Repo {
    let git_dir = resolve_git_dir(dot).unwrap_or_else(|| dot.to_path_buf());
    // A linked worktree lives outside the repository it belongs to. Name it
    // after that repository so every branch of one project groups together.
    let main_root = worktree_parent(&git_dir);
    let name_source = main_root.as_deref().unwrap_or(work_tree);
    let config_dir = main_root
        .as_ref()
        .map(|r| r.join(".git"))
        .unwrap_or_else(|| git_dir.clone());

    Repo {
        name: name_source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| name_source.display().to_string()),
        root: work_tree.to_path_buf(),
        branch: read_branch(&git_dir),
        remote: read_remote(&config_dir),
    }
}

/// A worktree's or submodule's `.git` is a file pointing at the real git dir.
pub fn resolve_git_dir(dot: &Path) -> Option<PathBuf> {
    if dot.is_dir() {
        return Some(dot.to_path_buf());
    }
    let text = std::fs::read_to_string(dot).ok()?;
    let path = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Some(path)
    } else {
        // Submodules record a path relative to the work tree.
        Some(dot.parent()?.join(path))
    }
}

/// `/repo/.git/worktrees/feat-x` → `/repo`.
pub fn worktree_parent(git_dir: &Path) -> Option<PathBuf> {
    let s = git_dir.to_string_lossy();
    let idx = s.find("/.git/worktrees/")?;
    Some(PathBuf::from(&s[..idx]))
}

pub fn read_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if head.is_empty() {
        return None;
    }
    match head.strip_prefix("ref: refs/heads/") {
        Some(branch) if !branch.is_empty() => Some(branch.to_string()),
        _ => {
            let short: String = head.chars().take(8).collect();
            Some(format!("detached @ {short}"))
        }
    }
}

pub fn read_remote(git_dir: &Path) -> Option<String> {
    let config = std::fs::read_to_string(git_dir.join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.replace(' ', "").starts_with("[remote\"origin\"]");
            continue;
        }
        if in_origin
            && let Some(url) = line
                .strip_prefix("url =")
                .or_else(|| line.strip_prefix("url="))
        {
            let url = url.trim();
            if !url.is_empty() {
                return Some(shorten_remote(url));
            }
        }
    }
    None
}

/// `git@github.com:acme/web.git` and the https form both become `acme/web`.
pub fn shorten_remote(url: &str) -> String {
    let s = url.trim().trim_end_matches('/').trim_end_matches(".git");
    if let Some((_, rest)) = s.split_once("://") {
        // Drop the host, and any credentials in front of it.
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        return rest
            .split_once('/')
            .map(|(_, p)| p.to_string())
            .unwrap_or_else(|| rest.to_string());
    }
    if let Some((_, path)) = s.split_once(':') {
        return path.trim_start_matches('/').to_string();
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Build a plain repository with a work tree at `<tmp>/<name>`.
    fn repo(tmp: &TempDir, name: &str, head: &str, remote: Option<&str>) -> PathBuf {
        let root = tmp.path().join(name);
        let git = root.join(".git");
        fs::create_dir_all(&git).expect("create .git");
        fs::write(git.join("HEAD"), head).expect("write HEAD");
        if let Some(url) = remote {
            fs::write(
                git.join("config"),
                format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n"),
            )
            .expect("write config");
        }
        root
    }

    #[test]
    fn finds_a_repo_from_a_nested_directory() {
        let tmp = TempDir::new().expect("tempdir");
        let root = repo(
            &tmp,
            "acme",
            "ref: refs/heads/main\n",
            Some("git@github.com:acme/web.git"),
        );
        let deep = root.join("packages/api/src");
        fs::create_dir_all(&deep).expect("create nested dirs");

        let found = find(&deep).expect("repo found from a nested path");
        assert_eq!(found.name, "acme");
        assert_eq!(found.root, root);
        assert_eq!(found.branch.as_deref(), Some("main"));
        assert_eq!(found.remote.as_deref(), Some("acme/web"));
    }

    #[test]
    fn returns_nothing_outside_a_repo() {
        let tmp = TempDir::new().expect("tempdir");
        let plain = tmp.path().join("not-a-repo");
        fs::create_dir_all(&plain).expect("create dir");
        // A temp dir can itself sit under a repository on a developer machine,
        // so assert on the name rather than on absence.
        if let Some(found) = find(&plain) {
            assert_ne!(found.name, "not-a-repo");
        }
    }

    #[test]
    fn a_linked_worktree_reports_its_parent_repository() {
        let tmp = TempDir::new().expect("tempdir");
        let main = repo(
            &tmp,
            "almanac",
            "ref: refs/heads/main\n",
            Some("git@github.com:oddurs/almanac.git"),
        );

        // git stores linked worktrees under <repo>/.git/worktrees/<id>.
        let wt_git = main.join(".git/worktrees/site-ci");
        fs::create_dir_all(&wt_git).expect("create worktree git dir");
        fs::write(wt_git.join("HEAD"), "ref: refs/heads/feat/site-ci\n").expect("write HEAD");

        let checkout = tmp.path().join(".worktrees/almanac/feat-site-ci");
        fs::create_dir_all(&checkout).expect("create checkout");
        fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", wt_git.display()),
        )
        .expect("write .git file");

        let found = find(&checkout).expect("worktree resolves");
        assert_eq!(found.name, "almanac", "grouped under the parent repository");
        assert_eq!(found.branch.as_deref(), Some("feat/site-ci"));
        assert_eq!(
            found.remote.as_deref(),
            Some("oddurs/almanac"),
            "remote comes from the parent repository's config"
        );
        assert_eq!(found.root, checkout, "path still points at the checkout");
    }

    #[test]
    fn detached_head_is_reported_as_such() {
        let tmp = TempDir::new().expect("tempdir");
        let root = repo(&tmp, "detached", "9f8e7d6c5b4a39281706\n", None);
        let found = find(&root).expect("repo found");
        assert_eq!(found.branch.as_deref(), Some("detached @ 9f8e7d6c"));
    }

    #[test]
    fn survives_a_repo_with_no_head_or_config() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path().join("fresh");
        fs::create_dir_all(root.join(".git")).expect("create .git");
        let found = find(&root).expect("a repo with no commits is still a repo");
        assert_eq!(found.name, "fresh");
        assert_eq!(found.branch, None);
        assert_eq!(found.remote, None);
    }

    #[test]
    fn survives_a_dangling_gitdir_pointer() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path().join("broken");
        fs::create_dir_all(&root).expect("create dir");
        fs::write(root.join(".git"), "gitdir: /nowhere/at/all\n").expect("write .git");
        let found = find(&root).expect("a broken pointer still names the directory");
        assert_eq!(found.name, "broken");
        assert_eq!(found.branch, None);
    }

    #[test]
    fn shortens_every_remote_form() {
        let cases = [
            ("git@github.com:acme/web.git", "acme/web"),
            ("https://github.com/acme/web.git", "acme/web"),
            ("https://token@github.com/acme/web", "acme/web"),
            (
                "ssh://git@git.example.com/team/sub/proj.git",
                "team/sub/proj",
            ),
            ("/local/path/repo", "/local/path/repo"),
        ];
        for (input, want) in cases {
            assert_eq!(shorten_remote(input), want, "input {input}");
        }
    }

    #[test]
    fn resolver_caches_repeated_lookups() {
        let tmp = TempDir::new().expect("tempdir");
        let root = repo(&tmp, "cached", "ref: refs/heads/main\n", None);
        let mut r = Resolver::new();
        assert!(r.is_empty());
        let a = r.resolve(&root);
        let b = r.resolve(&root);
        assert_eq!(a.map(|r| r.name), b.map(|r| r.name));
        assert_eq!(r.len(), 1, "second lookup was served from the cache");
    }
}
