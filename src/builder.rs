use std::path::PathBuf;

use crate::docker::{check_docker, docker_system, image_exists};
use crate::settings::{self, Settings};
use crate::{Result, error, puts};

pub const BASE_NAME: &str = "base";
pub const IMAGES_DIR_SETTING: &str = "images_dir";

pub struct Builder {
    tag_prefix: String,
    images_dir: PathBuf,
}

impl Builder {
    pub fn new(settings: &mut Settings) -> Result<Self> {
        Ok(Self {
            tag_prefix: settings.tag_prefix()?,
            images_dir: settings.images_dir()?,
        })
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

    /// Builds `image`, or every image when it's `None`
    pub fn call(&self, image: Option<&str>) -> Result<()> {
        let names = self.image_names()?;
        if let Some(image) = image
            && !names.iter().any(|name| name == image)
        {
            return error(format!(
                "unknown image '{image}', expected one of: {}",
                names.join(", ")
            ));
        }

        check_docker()?;

        if let Some(image) = image {
            return self.build_with_base(image);
        }

        self.build_with_base(BASE_NAME)?;
        names
            .iter()
            .filter(|name| *name != BASE_NAME)
            .try_for_each(|name| self.build_with_base(name))
    }

    fn build_with_base(&self, name: &str) -> Result<()> {
        if name != BASE_NAME && self.needs_base(name)? && !self.base_exists() {
            puts(&format!("==> {} doesn't exist yet", self.tag_for(BASE_NAME)));
            self.build(BASE_NAME)?;
        }

        self.build(name)
    }

    fn build(&self, name: &str) -> Result<()> {
        let tag = self.tag_for(name);

        // Only images FROM ${TAG_PREFIX}:base declare the arg, docker warns about unused ones.
        let mut args = vec![String::from("build"), String::from("--no-cache")];
        if self.needs_base(name)? {
            args.extend([String::from("--build-arg"), format!("TAG_PREFIX={}", self.tag_prefix)]);
        }
        let image_dir = self.image_dir(name).display().to_string();
        args.extend([String::from("-t"), tag.clone(), image_dir.clone()]);

        puts(&format!("==> Building {tag} from {image_dir}"));
        if !docker_system(&args) {
            return error(format!("failed to build {tag}"));
        }
        Ok(())
    }

    fn needs_base(&self, name: &str) -> Result<bool> {
        let dockerfile = std::fs::read(self.image_dir(name).join("Dockerfile"))?;
        Ok(from_base(&String::from_utf8_lossy(&dockerfile)))
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

        error(format!(
            "images directory {} does not exist, set {IMAGES_DIR_SETTING} in {}",
            self.images_dir.display(),
            settings::path().display()
        ))
    }
}

/// Whether a line of the Dockerfile is `FROM ${TAG_PREFIX}:base`, maybe followed by `AS <name>`
fn from_base(dockerfile: &str) -> bool {
    let image = format!("${{TAG_PREFIX}}:{BASE_NAME}");
    dockerfile.lines().any(|line| {
        let Some(rest) = line.strip_prefix("FROM") else {
            return false;
        };
        let reference = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        reference.len() < rest.len()
            && reference
                .strip_prefix(&image)
                .is_some_and(|after| after.is_empty() || after.starts_with(|c: char| c.is_ascii_whitespace()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_base_image() {
        assert!(from_base("ARG TAG_PREFIX=sndbx\nFROM ${TAG_PREFIX}:base\n"));
        assert!(from_base("FROM\t${TAG_PREFIX}:base AS build\r\n"));
        assert!(from_base("FROM ${TAG_PREFIX}:base"));
        assert!(!from_base("FROM ubuntu\n"));
        assert!(!from_base("FROM ${TAG_PREFIX}:base2\n"));
        assert!(!from_base("FROM${TAG_PREFIX}:base\n"));
        assert!(!from_base(" FROM ${TAG_PREFIX}:base\n"));
        assert!(!from_base("# FROM ${TAG_PREFIX}:base\n"));
    }
}
