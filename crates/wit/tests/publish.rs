use std::fs;

use crate::support::*;
use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use predicates::str::contains;
use semver::Version;
use toml_edit::{value, Array};
use warg_client::{Client, FileSystemClient};
use warg_protocol::registry::PackageId;
use wasm_metadata::LinkType;

mod support;

#[test]
fn help() {
    for arg in ["help publish", "publish -h", "publish --help"] {
        wit(arg)
            .assert()
            .stdout(contains("Publish a WIT package to a registry"))
            .success();
    }
}

#[test]
fn it_fails_with_missing_toml_file() -> Result<()> {
    wit("publish")
        .assert()
        .stderr(contains(
            "error: failed to find configuration file `wit.toml`",
        ))
        .failure();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn it_publishes_a_wit_package() -> Result<()> {
    let root = create_root()?;
    let (_server, config) = spawn_server(&root).await?;
    config.write_to_file(&root.join("warg-config.json"))?;

    let project = Project::with_root(&root, "foo", "")?;
    project.file("baz.wit", "package baz:qux\n")?;
    project
        .wit("publish --init")
        .env("WIT_PUBLISH_KEY", test_signing_key())
        .assert()
        .stderr(contains("Published package `baz:qux` v0.1.0"))
        .success();

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn it_does_a_dry_run_publish() -> Result<()> {
    let root = create_root()?;
    let (_server, config) = spawn_server(&root).await?;
    config.write_to_file(&root.join("warg-config.json"))?;

    let project = Project::with_root(&root, "foo", "")?;
    project.file("baz.wit", "package baz:qux\n")?;
    project
        .wit("publish --init --dry-run")
        .env("WIT_PUBLISH_KEY", test_signing_key())
        .assert()
        .stderr(contains(
            "warning: not publishing package to the registry due to the --dry-run option",
        ))
        .success();

    let client = FileSystemClient::new_with_config(None, &config)?;

    assert!(client
        .download(&"baz:qux".parse().unwrap(), &"0.1.0".parse().unwrap())
        .await
        .unwrap_err()
        .to_string()
        .contains("package `baz:qux` does not exist"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn it_publishes_with_registry_metadata() -> Result<()> {
    let root = create_root()?;
    let (_server, config) = spawn_server(&root).await?;
    config.write_to_file(&root.join("warg-config.json"))?;

    let authors = ["Jane Doe <jane@example.com>"];
    let categories = ["wasm"];
    let description = "A test package";
    let license = "Apache-2.0";
    let documentation = "https://example.com/docs";
    let homepage = "https://example.com/home";
    let repository = "https://example.com/repo";

    let project = Project::with_root(&root, "foo", "")?;
    project.file("baz.wit", "package baz:qux\n")?;

    project.update_manifest(|mut doc| {
        doc["authors"] = value(Array::from_iter(authors));
        doc["categories"] = value(Array::from_iter(categories));
        doc["description"] = value(description);
        doc["license"] = value(license);
        doc["documentation"] = value(documentation);
        doc["homepage"] = value(homepage);
        doc["repository"] = value(repository);
        Ok(doc)
    })?;

    project
        .wit("publish --init")
        .env("WIT_PUBLISH_KEY", test_signing_key())
        .assert()
        .stderr(contains("Published package `baz:qux` v0.1.0"))
        .success();

    let client = Client::new_with_config(None, &config)?;
    let download = client
        .download_exact(&PackageId::new("baz:qux")?, &Version::parse("0.1.0")?)
        .await?;

    let bytes = fs::read(&download.path).with_context(|| {
        format!(
            "failed to read downloaded package `{path}`",
            path = download.path.display()
        )
    })?;

    let metadata = wasm_metadata::RegistryMetadata::from_wasm(&bytes)
        .with_context(|| {
            format!(
                "failed to parse registry metadata from `{path}`",
                path = download.path.display()
            )
        })?
        .expect("missing registry metadata");

    assert_eq!(
        metadata.get_authors().expect("missing authors").as_slice(),
        authors
    );
    assert_eq!(
        metadata
            .get_categories()
            .expect("missing categories")
            .as_slice(),
        categories
    );
    assert_eq!(
        metadata.get_description().expect("missing description"),
        description
    );
    assert_eq!(metadata.get_license().expect("missing license"), license);

    let links = metadata.get_links().expect("missing links");
    assert_eq!(links.len(), 3);

    assert_eq!(
        links
            .iter()
            .find(|link| link.ty == LinkType::Documentation)
            .expect("missing documentation")
            .value,
        documentation
    );
    assert_eq!(
        links
            .iter()
            .find(|link| link.ty == LinkType::Homepage)
            .expect("missing homepage")
            .value,
        homepage
    );
    assert_eq!(
        links
            .iter()
            .find(|link| link.ty == LinkType::Repository)
            .expect("missing repository")
            .value,
        repository
    );

    Ok(())
}
