#[allow(dead_code)]
mod support;

use self::support::{
    TempDirectory, TestRepository, assert_success, git, ragavan_with_state, stderr, stdout,
};
use std::{collections::BTreeSet, fs};

#[test]
fn the_global_dashboard_is_empty_without_creating_state() {
    let directory = TempDirectory::new();
    let state_home = directory.path().join("state-home");

    let output = ragavan_with_state(directory.path(), &state_home, &["dashboard", "--json"]);

    assert_success(&output);
    assert_eq!(
        json_stdout(&output),
        serde_json::json!({"repositories": []})
    );
    assert_eq!(stderr(&output), "", "{output:?}");
    assert!(!state_home.exists());
}

#[test]
fn the_global_dashboard_distinguishes_repositories_with_the_same_basename() {
    let first = TestRepository::new();
    let second = TestRepository::new();
    let state = TempDirectory::new();

    assert_success(&ragavan_with_state(first.path(), state.path(), &["enable"]));
    assert_success(&ragavan_with_state(
        second.path(),
        state.path(),
        &["enable"],
    ));
    let output = ragavan_with_state(state.path(), state.path(), &["dashboard", "--json"]);

    assert_success(&output);
    let dashboard = json_stdout(&output);
    let repositories = dashboard["repositories"]
        .as_array()
        .expect("repositories should be an array");
    assert_eq!(repositories.len(), 2);
    let identifiers = repositories
        .iter()
        .map(|repository| {
            repository["id"]
                .as_str()
                .expect("repository should have an identity")
        })
        .collect::<BTreeSet<_>>();
    let directories = repositories
        .iter()
        .map(|repository| {
            repository["common_directory"]
                .as_str()
                .expect("repository should have a common directory")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(identifiers.len(), 2);
    assert_eq!(directories.len(), 2);
    assert!(
        repositories
            .iter()
            .all(|repository| repository["state"] == "enabled")
    );
}

#[test]
fn current_dashboard_selects_one_repository_and_all_of_its_worktrees() {
    let selected = TestRepository::new();
    let linked = selected.add_worktree("linked");
    let other = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        selected.path(),
        state.path(),
        &["enable"],
    ));
    assert_success(&ragavan_with_state(other.path(), state.path(), &["enable"]));

    let output = ragavan_with_state(&linked, state.path(), &["dashboard", "--current", "--json"]);

    assert_success(&output);
    let dashboard = json_stdout(&output);
    let repositories = dashboard["repositories"]
        .as_array()
        .expect("repositories should be an array");
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0]["state"], "enabled");
    let worktrees = repositories[0]["worktrees"]
        .as_array()
        .expect("worktrees should be an array");
    assert_eq!(worktrees.len(), 2);
    assert!(
        worktrees
            .iter()
            .all(|worktree| worktree["state"] == "available")
    );
    assert!(
        worktrees
            .iter()
            .all(|worktree| worktree["path"].is_string())
    );
}

#[test]
fn current_dashboard_reports_a_disabled_unidentified_repository() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();

    let output = ragavan_with_state(
        repository.path(),
        state.path(),
        &["dashboard", "--current", "--json"],
    );

    assert_success(&output);
    let dashboard = json_stdout(&output);
    let repository = &dashboard["repositories"][0];
    assert_eq!(repository["id"], serde_json::Value::Null);
    assert_eq!(repository["state"], "disabled");
    assert_eq!(
        repository["worktrees"].as_array().map(std::vec::Vec::len),
        Some(1)
    );
}

#[test]
fn current_dashboard_requires_a_git_worktree() {
    let directory = TempDirectory::new();
    let state = TempDirectory::new();

    let output = ragavan_with_state(
        directory.path(),
        state.path(),
        &["dashboard", "--current", "--json"],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = json_stderr(&output);
    assert_eq!(error["error"]["code"], "dashboard.repository.required");
}

#[test]
fn disabling_removes_a_repository_from_global_discovery() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["enable"],
    ));

    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["disable"],
    ));
    let output = ragavan_with_state(state.path(), state.path(), &["dashboard", "--json"]);

    assert_success(&output);
    assert_eq!(
        json_stdout(&output),
        serde_json::json!({"repositories": []})
    );
}

#[test]
fn disabling_uses_the_live_repository_directory_when_its_git_identity_is_invalid() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["enable"],
    ));
    assert_success(&git(
        repository.path(),
        &["config", "--local", "ragavan.repositoryId", ""],
    ));

    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["disable"],
    ));
    let output = ragavan_with_state(state.path(), state.path(), &["dashboard", "--json"]);

    assert_success(&output);
    assert_eq!(
        json_stdout(&output),
        serde_json::json!({"repositories": []})
    );
}

#[test]
fn failed_unregistration_retains_the_repository_identity_for_retry() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["enable"],
    ));
    let identity = stdout(&git(
        repository.path(),
        &["config", "--local", "--get", "ragavan.repositoryId"],
    ));
    fs::write(state.path().join("ragavan/state.json"), "{")
        .expect("the test should corrupt Ragavan state");

    let output = ragavan_with_state(repository.path(), state.path(), &["disable", "--json"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(json_stderr(&output)["error"]["code"], "runtime.state.parse");
    assert_eq!(
        stdout(&git(
            repository.path(),
            &["config", "--local", "--get", "ragavan.repositoryId"],
        )),
        identity
    );
    assert_eq!(
        git(
            repository.path(),
            &["config", "--local", "--get", "ragavan.enabled"],
        )
        .status
        .code(),
        Some(1)
    );
}

#[test]
fn missing_registered_repositories_remain_visible_as_unavailable() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["enable"],
    ));
    let hidden_git_directory = repository.path().join("git-directory-away");
    fs::rename(repository.path().join(".git"), &hidden_git_directory)
        .expect("Git directory should be moved out of the registered location");

    let output = ragavan_with_state(state.path(), state.path(), &["dashboard", "--json"]);

    assert_success(&output);
    let dashboard = json_stdout(&output);
    assert_eq!(dashboard["repositories"][0]["state"], "unavailable");
}

#[test]
fn re_enabling_a_moved_repository_refreshes_its_registered_path() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["enable"],
    ));
    let moved = repository
        .path()
        .parent()
        .expect("repository should have a parent")
        .join("moved-repository");
    fs::rename(repository.path(), &moved).expect("repository should be moved");

    assert_success(&ragavan_with_state(&moved, state.path(), &["enable"]));
    let output = ragavan_with_state(state.path(), state.path(), &["dashboard", "--json"]);

    assert_success(&output);
    let dashboard = json_stdout(&output);
    assert_eq!(dashboard["repositories"].as_array().map(Vec::len), Some(1));
    assert_eq!(dashboard["repositories"][0]["state"], "enabled");
    assert!(
        dashboard["repositories"][0]["common_directory"]
            .as_str()
            .is_some_and(|path| path.contains("moved-repository"))
    );
}

#[test]
fn a_registered_identity_mismatch_is_reported_without_guessing() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["enable"],
    ));
    assert_success(&git(
        repository.path(),
        &["config", "--local", "ragavan.repositoryId", "different-id"],
    ));

    let output = ragavan_with_state(state.path(), state.path(), &["dashboard", "--json"]);

    assert_success(&output);
    let dashboard = json_stdout(&output);
    let reported = &dashboard["repositories"][0];
    assert_eq!(reported["state"], "identity_mismatch");
    assert_eq!(reported["observed_id"], "different-id");
    let registered_id = reported["id"].clone();

    let current = ragavan_with_state(
        repository.path(),
        state.path(),
        &["dashboard", "--current", "--json"],
    );
    assert_success(&current);
    let current = json_stdout(&current);
    assert_eq!(current["repositories"][0]["state"], "identity_mismatch");
    assert_eq!(current["repositories"][0]["id"], registered_id);
    assert_eq!(current["repositories"][0]["observed_id"], "different-id");
}

#[test]
fn duplicate_live_repository_identities_are_rejected() {
    let first = TestRepository::new();
    let copied = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(first.path(), state.path(), &["enable"]));
    let repository_id = stdout(&git(
        first.path(),
        &["config", "--local", "--get", "ragavan.repositoryId"],
    ));
    assert_success(&git(
        copied.path(),
        &[
            "config",
            "--local",
            "ragavan.repositoryId",
            repository_id.trim(),
        ],
    ));

    let output = ragavan_with_state(copied.path(), state.path(), &["enable", "--json"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = json_stderr(&output);
    assert_eq!(
        error["error"]["code"],
        "runtime.repository_identity.conflict"
    );
    assert!(
        error["error"]["help"]
            .as_str()
            .is_some_and(|help| help.contains("disable Ragavan"))
    );
    let enrollment = git(
        copied.path(),
        &["config", "--local", "--get", "ragavan.enabled"],
    );
    assert_eq!(enrollment.status.code(), Some(1), "{enrollment:?}");

    let dashboard = ragavan_with_state(
        copied.path(),
        state.path(),
        &["dashboard", "--current", "--json"],
    );
    assert_success(&dashboard);
    let dashboard = json_stdout(&dashboard);
    assert_eq!(dashboard["repositories"][0]["state"], "identity_mismatch");
    assert!(dashboard["repositories"][0]["registered_directory"].is_string());
}

#[test]
fn human_dashboard_explains_repository_worktree_and_service_levels() {
    let repository = TestRepository::new();
    let state = TempDirectory::new();
    assert_success(&ragavan_with_state(
        repository.path(),
        state.path(),
        &["enable"],
    ));

    let output = ragavan_with_state(state.path(), state.path(), &["dashboard"]);

    assert_success(&output);
    let output = stdout(&output);
    assert!(output.contains("Repository "));
    assert!(output.contains("  State: enabled"));
    assert!(output.contains("  Git directory: "));
    assert!(output.contains("  Worktree main"));
    assert!(output.contains("    Services: none"));
    assert!(!output.contains('\u{1b}'));
}

fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout should contain one JSON value")
}

fn json_stderr(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr).expect("stderr should contain one JSON value")
}
