use super::{git_fixture, ForegroundFixture};
use std::fs;

pub(super) fn seed_preserved_issue_branch(fixture: &ForegroundFixture, branch: &str) {
    git_fixture(
        &fixture.repo_dir,
        &["config", "user.email", "test@example.com"],
    );
    git_fixture(&fixture.repo_dir, &["config", "user.name", "Autospec Test"]);
    fs::write(fixture.repo_dir.join("README.md"), "fixture\n").expect("seed repository");
    git_fixture(&fixture.repo_dir, &["add", "README.md"]);
    git_fixture(&fixture.repo_dir, &["commit", "-m", "seed repository"]);
    git_fixture(&fixture.repo_dir, &["push", "-u", "origin", "main"]);
    git_fixture(&fixture.repo_dir, &["checkout", "-b", branch]);
    fs::write(fixture.repo_dir.join("WIP.md"), "preserved issue work\n")
        .expect("seed preserved issue work");
    git_fixture(&fixture.repo_dir, &["add", "WIP.md"]);
    git_fixture(
        &fixture.repo_dir,
        &["commit", "-m", "seed preserved issue work"],
    );
    git_fixture(&fixture.repo_dir, &["checkout", "main"]);
}
