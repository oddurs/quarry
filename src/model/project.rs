//! Which project a service belongs to, and what that claim is worth.

use std::path::{Path, PathBuf};

/// Where a group's name came from. Drives both ordering and styling: a real
/// repository is a stronger claim than a directory that merely has a name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroupSource {
    Repo,
    Folder,
    Unattributed,
    System,
}

impl GroupSource {
    pub fn rank(self) -> u8 {
        match self {
            GroupSource::Repo => 0,
            GroupSource::Folder => 1,
            GroupSource::Unattributed => 2,
            GroupSource::System => 3,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Repo {
    pub name: String,
    /// The work tree this process is running in. For a linked worktree that is
    /// the worktree's own directory, not the repository's.
    pub root: PathBuf,
    /// The main repository's root — the same path for every worktree of one
    /// project, and equal to `root` when there are none.
    ///
    /// This, rather than the name, is what says two services belong to the
    /// same project. Names are basenames and two unrelated checkouts called
    /// `site` are not one repository.
    pub main_root: PathBuf,
    pub branch: Option<String>,
    pub remote: Option<String>,
}

/// One project, seen from inside it.
///
/// quarry normally answers "what is running on this machine". Started with
/// `--here` it answers a narrower question — "what is running for the project
/// I am in" — which is the one you have while working. The answer spans more
/// than the directory you are standing in: a linked worktree lives somewhere
/// else entirely, and a Compose stack declared in the repository is part of
/// the same project even though it runs in a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// The main repository root. Compared against [`Repo::main_root`], so
    /// every worktree of this project is in scope and an unrelated checkout
    /// with the same name is not.
    pub root: PathBuf,
    pub name: String,
    pub remote: Option<String>,
}

impl Scope {
    /// The repository containing `dir`, if it is in one.
    pub fn containing(dir: &Path) -> Option<Scope> {
        let repo = crate::repo::find(dir)?;
        Some(Scope {
            root: repo.main_root,
            name: repo.name,
            remote: repo.remote,
        })
    }

    /// The repository quarry was started in.
    pub fn here() -> Option<Scope> {
        Scope::containing(&std::env::current_dir().ok()?)
    }

    /// `quarry · oddurs/quarry`, or just the name when there is no remote.
    pub fn label(&self) -> String {
        match &self.remote {
            Some(r) => format!("{} · {r}", self.name),
            None => self.name.clone(),
        }
    }
}
