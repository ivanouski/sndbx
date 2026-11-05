//! Removing images, for `sndbx rm` and `sndbx top`

use std::collections::HashSet;

use anyhow::bail;

use crate::builder::BASE_NAME;
use crate::docker::{self, check_docker, image_containers, image_exists};
use crate::settings::{self, Settings};
use crate::{Result, print_status, unique};

struct Removal<'a> {
    name: &'a str,
    tag: String,
    exists: bool,
}

/// Removes the images and comments out their entries, checking every image before removing any
pub fn remove(settings: &Settings, names: &[&str]) -> Result<()> {
    if names.contains(&BASE_NAME) {
        bail!("{BASE_NAME} can't be removed, the other images are built on it");
    }
    let names = unique(names.iter().copied());

    check_docker()?;
    let removals = names
        .iter()
        .map(|name| check_removal(settings, name, &names))
        .collect::<Result<Vec<_>>>()?;
    remove_images(&removals)
}

/// Fails if the image is unknown, an image not in `names` shares its tag, or containers use it
fn check_removal<'a>(settings: &Settings, name: &'a str, names: &[&str]) -> Result<Removal<'a>> {
    let configured_tag = settings.configured_tag(name);
    let tag = configured_tag
        .clone()
        .unwrap_or_else(|| settings.default_tag(name));
    let exists = image_exists(&tag);
    if configured_tag.is_none() && !exists {
        bail!(
            "unknown image '{name}', it's neither in {} nor built as {tag}",
            settings::path().display()
        );
    }

    let others: Vec<&str> = settings
        .names_running(&tag)
        .into_iter()
        .filter(|other| !names.contains(other))
        .collect();
    if !others.is_empty() {
        bail!(
            "{tag} is also run by {}, rm them together",
            others.join(", ")
        );
    }
    if exists {
        let containers = image_containers(&tag)?;
        if !containers.is_empty() {
            bail!(
                "{tag} is used by containers: {}, remove them with docker rm first",
                containers.join(", ")
            );
        }
    }
    Ok(Removal { name, tag, exists })
}

/// Removes each tag once and comments out the entries
fn remove_images(removals: &[Removal]) -> Result<()> {
    let mut removed = HashSet::new();
    for removal in removals {
        let tag = removal.tag.as_str();
        if removed.insert(tag) {
            if removal.exists {
                if !docker::run(&["image", "rm", tag]) {
                    bail!("failed to remove {tag}");
                }
                print_status(&format!("Removed {tag}"));
            } else {
                print_status(&format!("{tag} doesn't exist"));
            }
        }
        comment_out(removal.name);
    }
    Ok(())
}

/// Only warns on failure, as the image is removed
fn comment_out(name: &str) {
    let path = settings::path();
    match settings::comment_out_image(name) {
        Ok(false) => {}
        Ok(true) => print_status(&format!("Commented out {name} in {}", path.display())),
        Err(e) => eprintln!(
            "Warning: can't comment out {name} in the settings: {e:#}.\nRemove its entry from {} yourself.",
            path.display()
        ),
    }
}
