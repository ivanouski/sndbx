use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};

use crate::docker::{self, check_docker, image_exists};
use crate::settings::{self, Added, IMAGES_DIR_SETTING, Image, NewEntry, Settings};
use crate::{Result, paths, print_status, unique};

pub const BASE_NAME: &str = "base";
/// Extra keys for the settings entry build adds
const ENTRY_FILE: &str = "sndbx.yml";
const DOCKERFILE: &str = "Dockerfile";

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

    /// Every directory with a Dockerfile except base
    pub fn image_names(&self) -> Result<Vec<String>> {
        self.check_images_dir()?;

        let mut names: Vec<String> = std::fs::read_dir(&self.images_dir)
            .with_context(|| format!("can't read {}", self.images_dir.display()))?
            // Skips entries that can't be read
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != BASE_NAME && self.is_image_dir(name))
            .collect();
        names.sort();
        Ok(names)
    }

    /// Builds base if needed, then each image, adding it to the settings so it runs right away
    pub fn build(&self, names: &[String], rebuild_base: bool) -> Result<()> {
        let known = self.image_names()?;
        for name in names {
            if name == BASE_NAME {
                bail!("{BASE_NAME} is built with --rebuild-base");
            }
            if !known.contains(name) {
                bail!(
                    "unknown image '{name}', expected one of: {}",
                    known.join(", ")
                );
            }
        }
        let images = unique(names.iter().map(String::as_str));
        // Before the slow builds, so a bad entry file fails fast
        let entries = images
            .iter()
            .map(|name| self.new_entry(name))
            .collect::<Result<Vec<_>>>()?;

        check_docker()?;
        self.build_base(&images, rebuild_base)?;
        for entry in &entries {
            self.docker_build(entry.name())?;
            self.add_to_settings(entry);
        }
        Ok(())
    }

    fn new_entry(&self, name: &str) -> Result<NewEntry> {
        let path = self.entry_file(name);
        let keys = if path.exists() {
            paths::read_file(&path)?
        } else {
            String::new()
        };
        NewEntry::new(name, &keys, &path)
    }

    /// Builds the image `run` needs if it's missing and its tag is one build makes
    pub fn build_missing(&self, image: &Image) -> Result<()> {
        if image_exists(&image.tag) {
            return Ok(());
        }
        let name = self.image_dir_for(&image.tag).map_err(|reason| {
            anyhow!(
                "docker image '{}' that {} runs doesn't exist, and sndbx can't build it: {reason:#}. \
                 Pull or build it yourself, or change {}'s tag in {}",
                image.tag,
                image.name,
                image.name,
                settings::path().display()
            )
        })?;

        print_status(&format!("{} doesn't exist yet, building it", image.tag));
        self.build_base(&[name], false)?;
        self.docker_build(name)
    }

    /// The image directory build makes the tag from
    fn image_dir_for<'a>(&self, tag: &'a str) -> Result<&'a str> {
        self.check_images_dir()?;
        let prefix = format!("{}:", self.tag_prefix);
        let Some(name) = tag.strip_prefix(&prefix).filter(|name| is_dir_name(name)) else {
            bail!(
                "it only builds {} from {}",
                self.tag_for("<name>"),
                self.image_dir("<name>").display()
            );
        };
        if !self.has_dockerfile(name) {
            bail!("there's no {}", self.dockerfile(name).display());
        }
        Ok(name)
    }

    fn build_base(&self, images: &[&str], rebuild_base: bool) -> Result<()> {
        if rebuild_base {
            self.docker_build(BASE_NAME)?;
        } else if self.any_needs_base(images)? && !self.base_exists() {
            print_status(&format!("{} doesn't exist yet", self.tag_for(BASE_NAME)));
            self.docker_build(BASE_NAME)?;
        }
        Ok(())
    }

    fn docker_build(&self, name: &str) -> Result<()> {
        let tag = self.tag_for(name);
        let image_dir = self.image_dir(name).display().to_string();
        let tag_prefix_arg = format!("TAG_PREFIX={}", self.tag_prefix);

        let mut args = vec!["build", "--no-cache"];
        // Docker warns about an unused build arg
        if self.needs_base(name)? {
            args.extend(["--build-arg", &tag_prefix_arg]);
        }
        args.extend(["-t", &tag, &image_dir]);

        print_status(&format!("Building {tag} from {image_dir}"));
        if !docker::run(&args) {
            bail!("failed to build {tag}");
        }
        Ok(())
    }

    fn needs_base(&self, name: &str) -> Result<bool> {
        let path = self.dockerfile(name);
        let dockerfile =
            std::fs::read(&path).with_context(|| format!("can't read {}", path.display()))?;
        Ok(is_from_base(&String::from_utf8_lossy(&dockerfile)))
    }

    fn any_needs_base(&self, names: &[&str]) -> Result<bool> {
        for name in names {
            if self.needs_base(name)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn base_exists(&self) -> bool {
        image_exists(&self.tag_for(BASE_NAME))
    }

    fn tag_for(&self, name: &str) -> String {
        settings::tag(&self.tag_prefix, name)
    }

    /// Only warns on failure, as the image is built
    fn add_to_settings(&self, entry: &NewEntry) {
        let name = entry.name();
        let path = settings::path();
        let hint = if entry.has_config_target() {
            String::new()
        } else {
            format!(
                ", set config_target there or in {} if {name} doesn't keep its config in ~/.{name}",
                self.entry_file(name).display()
            )
        };
        match settings::add_image(entry) {
            Ok(Added::Already) => {}
            Ok(Added::OtherTag(tag)) => print_status(&format!(
                "{name} in {} runs {tag}, not {}",
                path.display(),
                self.tag_for(name)
            )),
            Ok(Added::Appended) => {
                print_status(&format!("Added {name} to {}{hint}", path.display()));
            }
            Ok(Added::Created) => {
                print_status(&format!("Created {} with {name}{hint}", path.display()));
            }
            Err(e) => eprint!(
                "Warning: can't add {name} to the settings: {e:#}.\nAdd it to the images in {}:\n{}",
                path.display(),
                entry.item(2)
            ),
        }
    }

    fn image_dir(&self, name: &str) -> PathBuf {
        self.images_dir.join(name)
    }

    fn is_image_dir(&self, name: &str) -> bool {
        is_dir_name(name) && self.has_dockerfile(name)
    }

    fn entry_file(&self, name: &str) -> PathBuf {
        self.image_dir(name).join(ENTRY_FILE)
    }

    fn dockerfile(&self, name: &str) -> PathBuf {
        self.image_dir(name).join(DOCKERFILE)
    }

    fn has_dockerfile(&self, name: &str) -> bool {
        self.dockerfile(name).exists()
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

/// Not empty, hidden or nested
fn is_dir_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains('/')
}

/// Whether a line is `FROM ${TAG_PREFIX}:base`, maybe with `AS <name>`
fn is_from_base(dockerfile: &str) -> bool {
    let image = format!("${{TAG_PREFIX}}:{BASE_NAME}");
    dockerfile.lines().any(|line| {
        let mut words = line.split_ascii_whitespace();
        // Not indented, and FROM is a word of its own
        line.starts_with("FROM")
            && words.next() == Some("FROM")
            && words.next() == Some(image.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_base_image() {
        assert!(is_from_base(
            "ARG TAG_PREFIX=sndbx\nFROM ${TAG_PREFIX}:base\n"
        ));
        assert!(is_from_base("FROM\t${TAG_PREFIX}:base AS build\r\n"));
        assert!(is_from_base("FROM ${TAG_PREFIX}:base"));
        assert!(is_from_base("FROM  ${TAG_PREFIX}:base  "));
        assert!(!is_from_base("FROM ubuntu\n"));
        assert!(!is_from_base("FROM ${TAG_PREFIX}:base2\n"));
        assert!(!is_from_base("FROM${TAG_PREFIX}:base\n"));
        assert!(!is_from_base(" FROM ${TAG_PREFIX}:base\n"));
        assert!(!is_from_base("# FROM ${TAG_PREFIX}:base\n"));
    }

    #[test]
    fn the_repo_images_have_valid_entry_files() {
        let builder = Builder::new(&Settings::default());
        for name in builder.image_names().unwrap() {
            if let Err(e) = builder.new_entry(&name) {
                panic!("{e:#}");
            }
        }
        assert!(builder.new_entry("opencode").unwrap().has_config_target());
    }
}
