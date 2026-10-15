use std::path::PathBuf;

use anyhow::bail;

use crate::docker::{self, check_docker, image_exists};
use crate::settings::{self, Settings};
use crate::{Result, print_line};

pub const BASE_NAME: &str = "base";
pub const IMAGES_DIR_SETTING: &str = "images_dir";

pub struct Builder {
    tag_prefix: String,
    images_dir: PathBuf,
}

impl Builder {
    pub fn new(settings: &Settings) -> Self {
        Self {
            tag_prefix: settings.tag_prefix().to_string(),
            images_dir: settings.images_dir(),
        }
    }

    pub fn image_names(&self) -> Result<Vec<String>> {
        self.check_images_dir()?;

        let mut names: Vec<String> = std::fs::read_dir(&self.images_dir)?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.') && self.image_dir(name).join("Dockerfile").exists())
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn build(&self, name: &str) -> Result<()> {
        let names = self.image_names()?;
        if !names.iter().any(|known| known == name) {
            bail!("unknown image '{name}', expected one of: {}", names.join(", "));
        }

        check_docker()?;
        self.build_with_base(name)
    }

    /// Builds the base image, then every other one
    pub fn build_all(&self) -> Result<()> {
        let names = self.image_names()?;
        check_docker()?;

        self.build_with_base(BASE_NAME)?;
        names
            .iter()
            .filter(|name| *name != BASE_NAME)
            .try_for_each(|name| self.build_with_base(name))
    }

    fn build_with_base(&self, name: &str) -> Result<()> {
        if name != BASE_NAME && self.needs_base(name)? && !self.base_exists() {
            print_line(&format!("==> {} doesn't exist yet", self.tag_for(BASE_NAME)));
            self.docker_build(BASE_NAME)?;
        }

        self.docker_build(name)
    }

    fn docker_build(&self, name: &str) -> Result<()> {
        let tag = self.tag_for(name);
        let image_dir = self.image_dir(name).display().to_string();
        let tag_prefix_arg = format!("TAG_PREFIX={}", self.tag_prefix);

        let mut args = vec!["build", "--no-cache"];
        // Only images FROM ${TAG_PREFIX}:base declare the arg, docker warns about unused ones.
        if self.needs_base(name)? {
            args.extend(["--build-arg", &tag_prefix_arg]);
        }
        args.extend(["-t", &tag, &image_dir]);

        print_line(&format!("==> Building {tag} from {image_dir}"));
        if !docker::run(&args) {
            bail!("failed to build {tag}");
        }
        Ok(())
    }

    fn needs_base(&self, name: &str) -> Result<bool> {
        let dockerfile = std::fs::read(self.image_dir(name).join("Dockerfile"))?;
        Ok(is_from_base(&String::from_utf8_lossy(&dockerfile)))
    }

    fn base_exists(&self) -> bool {
        image_exists(&self.tag_for(BASE_NAME))
    }

    fn tag_for(&self, name: &str) -> String {
        format!("{}:{name}", self.tag_prefix)
    }

    fn image_dir(&self, name: &str) -> PathBuf {
        self.images_dir.join(name)
    }

    fn check_images_dir(&self) -> Result<()> {
        if self.images_dir.is_dir() {
            return Ok(());
        }

        bail!(
            "images directory {} does not exist, set {IMAGES_DIR_SETTING} in {}",
            self.images_dir.display(),
            settings::path().display()
        )
    }
}

/// Whether a line of the Dockerfile is `FROM ${TAG_PREFIX}:base`, maybe followed by `AS <name>`
fn is_from_base(dockerfile: &str) -> bool {
    let image = format!("${{TAG_PREFIX}}:{BASE_NAME}");
    dockerfile.lines().any(|line| {
        let mut words = line.split_ascii_whitespace();
        // Not indented, and FROM is a word of its own
        line.starts_with("FROM") && words.next() == Some("FROM") && words.next() == Some(image.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_base_image() {
        assert!(is_from_base("ARG TAG_PREFIX=sndbx\nFROM ${TAG_PREFIX}:base\n"));
        assert!(is_from_base("FROM\t${TAG_PREFIX}:base AS build\r\n"));
        assert!(is_from_base("FROM ${TAG_PREFIX}:base"));
        assert!(is_from_base("FROM  ${TAG_PREFIX}:base  "));
        assert!(!is_from_base("FROM ubuntu\n"));
        assert!(!is_from_base("FROM ${TAG_PREFIX}:base2\n"));
        assert!(!is_from_base("FROM${TAG_PREFIX}:base\n"));
        assert!(!is_from_base(" FROM ${TAG_PREFIX}:base\n"));
        assert!(!is_from_base("# FROM ${TAG_PREFIX}:base\n"));
    }
}
