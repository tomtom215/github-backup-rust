// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Packages backup.
//!
//! Lists the packages of the owner across all supported package ecosystems
//! and writes the results to the owner's JSON directory.  Requires the
//! `read:packages` OAuth scope.  An organisation's packages come from
//! `/orgs/{org}/packages`; the token owner's own from `/user/packages`, which
//! includes private ones; anybody else's from `/users/{username}/packages`.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::{BackupOptions, BackupTarget};

use crate::{
    backup::{
        all_json, describe_unparsed, is_forbidden, is_gone, json_str, merge::merge_and_write,
        refused, Outcome,
    },
    error::CoreError,
    storage::Storage,
};

/// Package types supported by GitHub Packages.
const PACKAGE_TYPES: &[&str] = &["container", "docker", "maven", "npm", "nuget", "rubygems"];

/// What a classic token needs to read packages.
const NEEDS: &str = "the `read:packages` scope";

/// Backs up GitHub Packages metadata for a user or an organisation.
///
/// When `opts.packages` is enabled, iterates over all supported package
/// ecosystems and:
/// - Writes `json_dir/packages_<type>.json` with the package list for each
///   package type that has at least one package.
/// - For each package, writes `json_dir/package_versions_<type>_<name>.json`
///   with all version metadata.  `<name>` is the package name with every
///   character other than ASCII letters, digits, `.`, `-` and `_`
///   percent-encoded (`team/image` → `team%2Fimage`), so two packages never
///   share a file.
///
/// Both files accumulate: they are merged by `id` with what earlier runs
/// stored, so a deleted package or version (GitHub deletes old versions)
/// stays in the backup.
///
/// A package type or a package whose listing fails is recorded in
/// [`Outcome::failed`] and the others are still backed up; a type the server
/// does not know (404) is skipped.  A 403 refusal is handled as described for
/// [`BackupOptions::tolerate_forbidden`].
///
/// Returns the total number of packages backed up across all ecosystems.
///
/// # Errors
///
/// Propagates [`CoreError`] from storage writes, a refusal that is not
/// tolerated, and any fatal error.
pub async fn backup_packages(
    client: &impl BackupClient,
    owner: &str,
    opts: &BackupOptions,
    json_dir: &Path,
    storage: &impl Storage,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.packages || opts.dry_run {
        return Ok(outcome);
    }
    let org = opts.target == BackupTarget::Org;

    for &package_type in PACKAGE_TYPES {
        let packages = match client.list_packages(owner, org, package_type).await {
            Ok(p) => p,
            Err(e) => {
                let e = CoreError::from(e);
                if is_gone(&e) {
                    info!(owner, package_type, "no packages of this type here");
                    continue;
                }
                if is_forbidden(&e) {
                    // Every other type would be refused the same way.
                    refused(opts, "packages", NEEDS, e, true)?;
                    return Ok(outcome);
                }
                outcome.item_error(format!("{package_type} packages"), e)?;
                continue;
            }
        };

        let type_count = (packages.len() + packages.unparsed_count()) as u64;
        if type_count == 0 {
            continue;
        }
        outcome.count += type_count;
        info!(
            owner,
            package_type,
            count = type_count,
            "backing up packages"
        );
        let filename = format!("packages_{package_type}.json");
        merge_and_write(storage, &json_dir.join(&filename), &packages, "id")?;

        for package in all_json(&packages) {
            let Some(name) = json_str(package, "name") else {
                outcome.fail(
                    format!("{package_type} package ({})", describe_unparsed(package)),
                    "the API object has no `name`, so its versions cannot be fetched",
                );
                continue;
            };
            match client
                .list_package_versions(owner, org, package_type, name)
                .await
            {
                Ok(versions) => {
                    let file = format!(
                        "package_versions_{package_type}_{}.json",
                        file_safe_name(name)
                    );
                    merge_and_write(storage, &json_dir.join(&file), &versions, "id")?;
                    info!(
                        owner,
                        package_type,
                        package_name = %name,
                        version_count = versions.len(),
                        "saved package versions"
                    );
                }
                Err(e) => {
                    outcome.item_error(format!("{package_type} package {name} versions"), e)?;
                }
            }
        }
    }

    Ok(outcome)
}

/// `name` as one file-name component, injectively: ASCII letters, digits,
/// `.`, `-` and `_` stay, every other byte becomes `%XX` (`%` itself
/// included), so distinct names give distinct files and the result never
/// contains a path separator.
fn file_safe_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    // `.` and `..` alone would name a directory.
    if out.bytes().all(|b| b == b'.') {
        out = out.replace('.', "%2E");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::package::{Package, PackageRepository, PackageVersion};
    use github_backup_types::user::User;
    use std::path::PathBuf;

    fn make_user() -> User {
        User {
            login: "octocat".to_string(),
            id: 1,
            avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
            html_url: "https://github.com/octocat".to_string(),
            user_type: "User".to_string(),
        }
    }

    fn make_package(id: u64, name: &str, package_type: &str) -> Package {
        Package {
            id,
            name: name.to_string(),
            package_type: package_type.to_string(),
            visibility: "public".to_string(),
            version_count: 1,
            html_url: format!("https://github.com/users/octocat/packages/{package_type}/{name}"),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            owner: Some(make_user()),
            repository: Some(PackageRepository {
                name: "my-repo".to_string(),
                full_name: "octocat/my-repo".to_string(),
                private: false,
            }),
        }
    }

    fn make_version(id: u64, name: &str) -> PackageVersion {
        PackageVersion {
            id,
            name: name.to_string(),
            html_url: Some(format!(
                "https://github.com/users/octocat/packages/npm/my-pkg/versions/{id}"
            )),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            metadata: None,
        }
    }

    #[tokio::test]
    async fn backup_packages_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // packages: false

        let count = backup_packages(&client, "octocat", &opts, &PathBuf::from("/json"), &storage)
            .await
            .expect("backup_packages")
            .count;

        assert_eq!(count, 0);
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_packages_enabled_writes_json() {
        let pkg = make_package(1, "my-npm-pkg", "npm");
        let ver = make_version(100, "v1.0.0");
        let client = MockBackupClient::new()
            .with_packages(vec![pkg])
            .with_package_versions(vec![ver]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            packages: true,
            ..Default::default()
        };

        let count = backup_packages(&client, "octocat", &opts, &PathBuf::from("/json"), &storage)
            .await
            .expect("backup_packages")
            .count;

        // MockBackupClient returns the same packages list for every package_type,
        // so PACKAGE_TYPES.len() packages will be written.
        assert!(count > 0, "should back up at least one package");
        // At least one packages_<type>.json should exist.
        let has_packages_file = PACKAGE_TYPES.iter().any(|t| {
            storage
                .get(&PathBuf::from(format!("/json/packages_{t}.json")))
                .is_some()
        });
        assert!(
            has_packages_file,
            "at least one packages_<type>.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_packages_empty_writes_nothing() {
        let client = MockBackupClient::new(); // returns empty list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            packages: true,
            ..Default::default()
        };

        let count = backup_packages(&client, "octocat", &opts, &PathBuf::from("/json"), &storage)
            .await
            .expect("backup_packages")
            .count;

        assert_eq!(count, 0);
        assert_eq!(
            storage.len(),
            0,
            "empty package list should produce no files"
        );
    }

    /// A dry run writes nothing: the e2e harness found package metadata being
    /// saved by `--dry-run`.
    #[tokio::test]
    async fn dry_run_writes_no_package_files() {
        let client = crate::backup::mock_client::MockBackupClient::new();
        let storage = crate::storage::test_support::MemStorage::default();
        let opts = BackupOptions {
            packages: true,
            dry_run: true,
            ..Default::default()
        };
        let n = backup_packages(&client, "octocat", &opts, Path::new("/json"), &storage)
            .await
            .expect("dry run")
            .count;
        assert_eq!(n, 0);
        assert_eq!(storage.len(), 0);
    }

    /// `a/b` and `a_b` used to share one file (`/` became `_`).
    #[test]
    fn package_file_names_are_collision_free() {
        assert_eq!(file_safe_name("my-image_1.0"), "my-image_1.0");
        assert_eq!(file_safe_name("a/b"), "a%2Fb");
        assert_ne!(file_safe_name("a/b"), file_safe_name("a_b"));
        assert_ne!(file_safe_name("a%2Fb"), file_safe_name("a/b"));
        assert_eq!(file_safe_name("@scope/pkg"), "%40scope%2Fpkg");
        assert_eq!(file_safe_name(".."), "%2E%2E");
    }

    fn opts() -> BackupOptions {
        BackupOptions {
            packages: true,
            ..Default::default()
        }
    }

    /// One package type that fails used to abort every later type.
    #[tokio::test]
    async fn a_failing_type_or_package_is_recorded_and_the_rest_continue() {
        let client = MockBackupClient::new()
            .with_packages(vec![make_package(1, "team/image", "container")])
            .with_package_versions(vec![make_version(100, "v1")])
            .with_error("packages:container", 500)
            .with_error("package_versions", 502);
        let storage = MemStorage::default();
        let outcome = backup_packages(&client, "octocat", &opts(), Path::new("/json"), &storage)
            .await
            .expect("not fatal");

        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(whats[0], "container packages");
        assert!(
            whats.contains(&"npm package team/image versions"),
            "{whats:?}"
        );
        assert!(storage
            .get(&PathBuf::from("/json/packages_npm.json"))
            .is_some());
    }

    #[tokio::test]
    async fn a_refusal_fails_when_requested_and_is_skipped_under_all() {
        let client = MockBackupClient::new().with_error("packages", 403);
        let err = backup_packages(
            &client,
            "octocat",
            &opts(),
            Path::new("/json"),
            &MemStorage::default(),
        )
        .await
        .expect_err("explicit --packages: failure");
        assert!(err.to_string().contains("read:packages"), "{err}");
        assert_eq!(
            client.calls().len(),
            1,
            "a refusal is not repeated for every type"
        );

        let all = BackupOptions {
            tolerate_forbidden: true,
            ..opts()
        };
        backup_packages(
            &client,
            "octocat",
            &all,
            Path::new("/json"),
            &MemStorage::default(),
        )
        .await
        .expect("--all: skipped");
    }

    #[tokio::test]
    async fn versions_are_written_under_an_encoded_name() {
        let client = MockBackupClient::new()
            .with_packages(vec![make_package(1, "team/image", "container")])
            .with_package_versions(vec![make_version(100, "v1")]);
        let storage = MemStorage::default();
        backup_packages(&client, "octocat", &opts(), Path::new("/json"), &storage)
            .await
            .expect("ok");
        assert!(storage
            .get(&PathBuf::from(
                "/json/package_versions_container_team%2Fimage.json"
            ))
            .is_some());
    }
}
