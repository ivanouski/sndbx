mod de;
mod edit;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::paths::expand_path;
use crate::{Abort, Result, paths};
use de::{optional_text, or_default, strings, text};
pub use edit::{Added, NewEntry, add_image, comment_out_image};

pub const IMAGES_DIR_SETTING: &str = "images_dir";
const DEFAULT_TAG_PREFIX: &str = "sndbx";
// The images directory of the repo this binary was built from
const REPO_IMAGES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/images");

const EXAMPLE: &str = include_str!("example.yml");

pub fn dir() -> PathBuf {
    paths::xdg_dir("XDG_CONFIG_HOME", ".config").join("sndbx")
}

pub fn path() -> PathBuf {
    dir().join("settings.yml")
}

pub struct Image {
    pub name: String,
    pub tag: String,
    pub config_dir: PathBuf,
    /// Overrides the image's user
    pub user: Option<String>,
    /// Relative to the user's home
    pub config_target: String,
    pub ports: Vec<String>,
    pub docker_args: Vec<String>,
    /// Whether the image uses the proxy when it's on
    pub proxy: bool,
}

/// The settings file as written
#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(
    default,
    deny_unknown_fields,
    expecting = "a mapping with an images list"
)]
struct File {
    #[serde(deserialize_with = "optional_text")]
    tag_prefix: Option<String>,
    #[serde(deserialize_with = "optional_text")]
    images_dir: Option<String>,
    /// Before each image's own
    #[serde(deserialize_with = "strings")]
    docker_args: Vec<String>,
    proxy: Option<Proxy>,
    images: Option<Vec<Entry>>,
}

#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields, expecting = "a mapping with an image")]
pub struct Proxy {
    #[serde(deserialize_with = "text")]
    pub image: String,
    #[serde(default, deserialize_with = "or_default")]
    pub allow: Allow,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Allow {
    /// The ones in `allowed_hosts`
    #[default]
    Listed,
    /// Every host, to watch what sandboxes reach
    All,
}

#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields, expecting = "a mapping with a name")]
struct Entry {
    #[serde(deserialize_with = "text")]
    name: String,
    #[serde(default, deserialize_with = "optional_text")]
    user: Option<String>,
    #[serde(default, deserialize_with = "optional_text")]
    tag: Option<String>,
    #[serde(default, deserialize_with = "optional_text")]
    config_dir: Option<String>,
    #[serde(default, deserialize_with = "optional_text")]
    config_target: Option<String>,
    #[serde(default, deserialize_with = "strings")]
    ports: Vec<String>,
    #[serde(default, deserialize_with = "strings")]
    docker_args: Vec<String>,
    #[serde(default)]
    proxy: Option<bool>,
}

#[derive(Default)]
pub struct Settings {
    file: File,
}

impl Settings {
    /// Creates an example and stops when there's no settings file
    pub fn load() -> Result<Self> {
        let path = path();
        if !path.exists() {
            paths::write_file(&path, EXAMPLE)?;
            return Err(Abort(format!(
                "Created an example config at {}, edit it and run again.",
                path.display()
            ))
            .into());
        }
        read(&path)
    }

    pub fn load_or_default() -> Result<Self> {
        let path = path();
        if !path.exists() {
            return Ok(Self::default());
        }
        read(&path)
    }

    pub fn image(&self, name: &str) -> Result<Image> {
        let Some(entry) = self.entry(name) else {
            bail!(
                "unknown image '{name}', expected one of: {}",
                self.image_names().join(", ")
            );
        };

        let config_dir = match entry.config_dir.as_deref() {
            None => name,
            Some("") => bail!(
                "image '{name}' has a config_dir that isn't a path in {}",
                path().display()
            ),
            Some(dir) => dir,
        };
        let config_target = entry
            .config_target
            .clone()
            .unwrap_or_else(|| format!(".{name}"));

        Ok(Image {
            name: name.to_string(),
            tag: self.entry_tag(entry),
            config_dir: expand_path(config_dir, Some(&dir())),
            user: entry.user.clone(),
            config_target,
            ports: entry.ports.clone(),
            // Later arguments win in docker, so the image's override the top-level ones
            docker_args: self
                .file
                .docker_args
                .iter()
                .chain(&entry.docker_args)
                .cloned()
                .collect(),
            proxy: entry.proxy.unwrap_or(true),
        })
    }

    fn entry_tag(&self, entry: &Entry) -> String {
        entry
            .tag
            .clone()
            .unwrap_or_else(|| self.default_tag(&entry.name))
    }

    pub fn default_tag(&self, name: &str) -> String {
        tag(self.tag_prefix(), name)
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.images().iter().find(|entry| entry.name == name)
    }

    /// `None` when the image has no entry
    pub fn configured_tag(&self, name: &str) -> Option<String> {
        self.entry(name).map(|entry| self.entry_tag(entry))
    }

    pub fn names_running(&self, tag: &str) -> Vec<&str> {
        self.images()
            .iter()
            .filter(|entry| self.entry_tag(entry) == tag)
            .map(|entry| entry.name.as_str())
            .collect()
    }

    pub fn image_names(&self) -> Vec<String> {
        self.images()
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    pub fn tag_prefix(&self) -> &str {
        self.file
            .tag_prefix
            .as_deref()
            .unwrap_or(DEFAULT_TAG_PREFIX)
    }

    pub fn images_dir(&self) -> PathBuf {
        match self.file.images_dir.as_deref() {
            Some(images_dir) => expand_path(images_dir, Some(&dir())),
            None => expand_path(REPO_IMAGES_DIR, None),
        }
    }

    pub fn proxy(&self) -> Option<&Proxy> {
        self.file.proxy.as_ref()
    }

    fn images(&self) -> &[Entry] {
        self.file.images.as_deref().unwrap_or_default()
    }
}

fn read(path: &Path) -> Result<Settings> {
    let content = paths::read_file(path)?;
    validate(&content, path)
}

fn validate(content: &str, path: &Path) -> Result<Settings> {
    let file = match parse(content) {
        // serde names the field without a path at the top level
        Err(error) if error.to_string().starts_with("unknown field `proxy_image`") => bail!(
            "invalid {}: proxy_image is now proxy: {{ image: ... }}, see the README",
            path.display()
        ),
        file => file.with_context(|| format!("invalid {}", path.display()))?,
    };

    if file
        .tag_prefix
        .as_deref()
        .is_some_and(|prefix| !is_valid_tag_prefix(prefix))
    {
        bail!(
            "invalid {}: tag_prefix must be a lowercase image name, e.g. sndbx",
            path.display()
        );
    }

    if file
        .proxy
        .as_ref()
        .is_some_and(|proxy| !is_valid_image(&proxy.image))
    {
        bail!(
            "invalid {}: proxy.image must be an image like ubuntu/squid",
            path.display()
        );
    }

    Ok(Settings { file })
}

/// The default tag, `<tag_prefix>:<name>`
pub fn tag(tag_prefix: &str, name: &str) -> String {
    format!("{tag_prefix}:{name}")
}

/// Only the first document, the default when there's none or it's empty
fn first_document<T: DeserializeOwned + Default>(content: &str) -> Result<T, serde_norway::Error> {
    let Some(document) = serde_norway::Deserializer::from_str(content).next() else {
        return Ok(T::default());
    };
    Ok(Option::<T>::deserialize(document)?.unwrap_or_default())
}

fn parse(content: &str) -> Result<File, serde_norway::Error> {
    first_document(content)
}

/// No whitespace, and no leading `-` that would make it an option
fn is_valid_image(image: &str) -> bool {
    !image.is_empty() && !image.starts_with('-') && !image.contains(char::is_whitespace)
}

/// Lowercase alphanumeric words joined by single `.`, `_`, `-` or `/`, like `sndbx` or `my.org/sndbx`
fn is_valid_tag_prefix(prefix: &str) -> bool {
    prefix.split(['.', '_', '-', '/']).all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_prefixes() {
        for valid in ["sndbx", "my.org/sndbx", "a-b_c", "x1"] {
            assert!(is_valid_tag_prefix(valid), "{valid}");
        }
        for invalid in ["", "Not Valid", "a--b", "-a", "a/", "Sndbx"] {
            assert!(!is_valid_tag_prefix(invalid), "{invalid}");
        }
    }

    #[test]
    fn example_parses() {
        let example = parse(EXAMPLE).unwrap();
        assert_eq!(
            example.docker_args,
            ["--security-opt=no-new-privileges", "--pids-limit=4096"]
        );
        assert_eq!(example.images.unwrap()[0].name, "claude");
    }

    #[test]
    fn empty_settings() {
        for empty in ["", "# comment\n", "~\n", "images: ~\n"] {
            assert!(parse(empty).unwrap().images.is_none(), "{empty:?}");
        }
    }

    #[test]
    fn only_the_first_document() {
        assert_eq!(
            parse("images: []\n---\nimages: 1\n").unwrap().images,
            Some(vec![])
        );
    }

    #[test]
    fn duplicate_keys() {
        let message = parse("images: []\nimages: []\n").err().unwrap().to_string();
        assert!(message.contains("duplicate field `images`"), "{message}");
    }

    #[test]
    fn strings_and_lists() {
        let image = parse("images:\n  - name: a\n    ports: '3000'\n    docker_args: [x, y]\n")
            .unwrap()
            .images
            .unwrap()
            .remove(0);
        assert_eq!(image.ports, ["3000"]);
        assert_eq!(image.docker_args, ["x", "y"]);
    }

    #[test]
    fn proxies() {
        let proxy = |content| parse(content).unwrap().proxy;
        assert_eq!(proxy("images: []\n"), None);
        assert_eq!(proxy("proxy: ~\n"), None);
        assert_eq!(
            proxy("proxy:\n  image: squid\n"),
            Some(Proxy {
                image: "squid".to_string(),
                allow: Allow::Listed
            })
        );
        assert_eq!(
            proxy("proxy:\n  image: squid\n  allow: all\n")
                .unwrap()
                .allow,
            Allow::All
        );
        assert_eq!(
            proxy("proxy:\n  image: squid\n  allow:\n").unwrap().allow,
            Allow::Listed
        );
        let message = parse("proxy:\n  image: squid\n  allow: any\n")
            .err()
            .unwrap()
            .to_string();
        assert!(
            message.contains("proxy.allow: unknown variant `any`, expected `listed` or `all`"),
            "{message}"
        );
        let message = parse("proxy:\n  allow: all\n").err().unwrap().to_string();
        assert!(
            message.contains("proxy: missing field `image`"),
            "{message}"
        );
    }

    #[test]
    fn unknown_keys() {
        let message = parse("image: []\n").err().unwrap().to_string();
        assert!(message.contains("unknown field `image`"), "{message}");
        let message = parse("images:\n  - name: a\n    port: '3000'\n")
            .err()
            .unwrap()
            .to_string();
        assert!(
            message.contains("images[0]: unknown field `port`"),
            "{message}"
        );
    }

    #[test]
    fn only_strings() {
        for invalid in [
            "images:\n  - name: a\n    ports: [3000]\n",
            "images:\n  - name: a\n    docker_args: 8\n",
            "images:\n  - name: a\n    user: 5\n",
            "images:\n  - name: a\n    config_dir: 42\n",
            "images:\n  - name: true\n",
            "tag_prefix: 1\n",
            "docker_args: [8]\n",
            "proxy:\n  image: 1\n",
        ] {
            let message = parse(invalid).err().unwrap().to_string();
            assert!(
                message.contains("expected a string"),
                "{invalid:?}: {message}"
            );
        }
    }
}
