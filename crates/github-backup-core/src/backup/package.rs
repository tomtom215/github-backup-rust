// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Packages backup.
//!
//! Lists all packages published by a user across all supported package
//! ecosystems and writes the results to the owner's JSON directory.
//! Requires the `read:packages` OAuth scope.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Package types supported by GitHub Packages.
const PACKAGE_TYPES: &[&str] =
    &["container", "docker", "maven", "npm", "nuget", "rubygems"];

/// Backs up GitHub Packages metadata for a user.
///
/// When `opts.packages` is enabled, iterates over all supported package
/// ecosystems and:
/// - Writes `json_dir/packages_<type>.json` with the package list for each
///   package type that has at least one package.
/// - For each package, writes `json_dir/package_versions_<type>_<name>.json`
///   with all version metadata.
///
/// The function requires the `read:packages` OAuth scope.  If the API returns
/// 403 or 404 for a particular package type (no packages or insufficient
/// permissions) that type is skipped with an informational log message.
///
/// Returns the total number of packages backed up across all ecosystems.
///
/// # Errors
///
/// Propagates [`CoreError`] from storage writes or unexpected API errors.
pub async fn backup_packages(
    client: &impl BackupClient,
    username: &str,
    opts: &BackupOptions,
    json_dir: &Path,
    storage: &impl Storage,
) -> Result<u64, CoreError> {
    if !opts.packages {
        return Ok(0);
    }

    let mut total: u64 = 0;

    for &package_type in PACKAGE_TYPES {
        let packages = match client.list_user_packages(username, package_type).await {
            Ok(p) => p,
            Err(github_backup_client::ClientError::ApiError {
                status: 403 | 404, ..
            }) => {
                info!(
                    username,
                    package_type,
                    "skipping packages (not available or insufficient permissions)"
                );
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        if packages.is_empty() {
            continue;
        }

        let type_count = packages.len() as u64;
        total += type_count;
        info!(
            username,
            package_type,
            count = type_count,
            "backing up packages"
        );

        let filename = format!("packages_{package_type}.json");
        storage.write_json(&json_dir.join(&filename), &packages)?;

        // Back up versions for each package.
        for package in &packages {
            let versions = match client
                .list_package_versions(username, package_type, &package.name)
                .await
            {
                Ok(v) => v,
                Err(github_backup_client::ClientError::ApiError {
                    status: 403 | 404, ..
                }) => {
                    info!(
                        username,
                        package_type,
                        package_name = %package.name,
                        "skipping package versions (not available)"
                    );
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            // Sanitise package name for use in a filename (replace '/' and '@').
            let safe_name = package.name.replace('/', "_").replace('@', "");
            let ver_filename = format!("package_versions_{package_type}_{safe_name}.json");
            storage.write_json(&json_dir.join(&ver_filename), &versions)?;
            info!(
                username,
                package_type,
                package_name = %package.name,
                version_count = versions.len(),
                "saved package versions"
            );
        }
    }

    Ok(total)
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
            owner: make_user(),
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
            html_url: format!("https://github.com/users/octocat/packages/npm/my-pkg/versions/{id}"),
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

        let count = backup_packages(
            &client,
            "octocat",
            &opts,
            &PathBuf::from("/json"),
            &storage,
        )
        .await
        .expect("backup_packages");

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

        let count = backup_packages(
            &client,
            "octocat",
            &opts,
            &PathBuf::from("/json"),
            &storage,
        )
        .await
        .expect("backup_packages");

        // MockBackupClient returns the same packages list for every package_type,
        // so PACKAGE_TYPES.len() packages will be written.
        assert!(count > 0, "should back up at least one package");
        // At least one packages_<type>.json should exist.
        let has_packages_file = PACKAGE_TYPES.iter().any(|t| {
            storage
                .get(&PathBuf::from(format!("/json/packages_{t}.json")))
                .is_some()
        });
        assert!(has_packages_file, "at least one packages_<type>.json should be written");
    }

    #[tokio::test]
    async fn backup_packages_empty_writes_nothing() {
        let client = MockBackupClient::new(); // returns empty list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            packages: true,
            ..Default::default()
        };

        let count = backup_packages(
            &client,
            "octocat",
            &opts,
            &PathBuf::from("/json"),
            &storage,
        )
        .await
        .expect("backup_packages");

        assert_eq!(count, 0);
        assert_eq!(
            storage.len(),
            0,
            "empty package list should produce no files"
        );
    }
}
