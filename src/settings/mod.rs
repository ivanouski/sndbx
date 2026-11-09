mod de;
mod edit;

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::paths::{HOME_PREFIX, HOME_SHORTHAND, expand_path};
use crate::{Abort, Result, paths};
use de::{optional_text, or_default, strings, text};
pub use edit::{Added, NewEntry, add_image, comment_out_image};

pub const IMAGES_DIR_SETTING: &str = "images_dir";
const DEFAULT_TAG_PREFIX: &str = "sndbx";
/// What docker allows between the lowercase alphanumeric words of a repository name
const TAG_PREFIX_SEPARATORS: [char; 4] = ['.', '_', '-', '/'];
// The images directory sndbx was built with: SNDBX_IMAGES_DIR at build time, e.g. a package's, or the repo's
const BUILT_IN_IMAGES_DIR: &str = match option_env!("SNDBX_IMAGES_DIR") {
    Some(dir) => dir,
    None => concat!(env!("CARGO_MANIFEST_DIR"), "/images"),
};

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
    /// Absolute, or relative to the user's home
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
                "image '{name}' in {} has a config_dir that isn't a path",
                path().display()
            ),
            Some(dir) => dir,
        };
        let config_target = entry
            .config_target
            .clone()
            .unwrap_or_else(|| default_config_target(name));
        if let Some(problem) = config_target_problem(&config_target) {
            bail!(
                "image '{name}' in {} has a config_target that {problem}",
                path().display()
            );
        }

        Ok(Image {
            name: name.to_string(),
            tag: self.entry_tag(entry),
            config_dir: expand_path(config_dir, Some(&dir())),
            user: entry.user.clone(),
            config_target,
            ports: entry.ports.clone(),
            // Later arguments win in docker, so the image's override the top-level ones
            docker_args: expand_home_in_args(
                self.file.docker_args.iter().chain(&entry.docker_args),
                &paths::home(),
            ),
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
            None => expand_path(BUILT_IN_IMAGES_DIR, None),
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

/// Where a docker run flag's value has a host path
#[derive(Clone, Copy)]
enum PathIn {
    /// The whole value, like `--env-file`'s
    Value,
    /// Before the first `:`, like `-v`'s; without one, the value is a container path
    VolumeSource,
    /// In the `source` or `src` field, like `--mount`'s
    MountSource,
}

fn path_flag(flag: &str) -> Option<PathIn> {
    match flag {
        "--env-file" | "--label-file" => Some(PathIn::Value),
        "-v" | "--volume" => Some(PathIn::VolumeSource),
        "--mount" => Some(PathIn::MountSource),
        _ => None,
    }
}

/// Expands a leading `~/` in the host paths of the flags `path_flag` knows, as `--flag=value` or `--flag value`;
/// others, like `-e FOO=~/x`, may mean the container's home
fn expand_home_in_args<'a>(args: impl IntoIterator<Item = &'a String>, home: &Path) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut value_of = None;
    for arg in args {
        let arg = if let Some(path_in) = value_of.take() {
            expand_home_in_value(arg, path_in, home)
        } else if let Some(path_in) = path_flag(arg) {
            value_of = Some(path_in);
            arg.clone()
        } else if let Some((flag, value)) = arg.split_once('=')
            && let Some(path_in) = path_flag(flag)
        {
            format!("{flag}={}", expand_home_in_value(value, path_in, home))
        } else {
            arg.clone()
        };
        expanded.push(arg);
    }
    expanded
}

fn expand_home_in_value(value: &str, path_in: PathIn, home: &Path) -> String {
    match path_in {
        PathIn::Value => expand_home(value, home),
        PathIn::VolumeSource if value.contains(':') => expand_home(value, home),
        PathIn::VolumeSource => value.to_string(),
        PathIn::MountSource => value
            .split(',')
            .map(|field| match field.split_once('=') {
                Some((key @ ("source" | "src"), path)) => {
                    format!("{key}={}", expand_home(path, home))
                }
                _ => field.to_string(),
            })
            .collect::<Vec<_>>()
            .join(","),
    }
}

fn expand_home(value: &str, home: &Path) -> String {
    match value.strip_prefix(HOME_PREFIX) {
        Some(rest) => home.join(rest).display().to_string(),
        None => value.to_string(),
    }
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

/// In the user's home, where tools keep their config by default
pub fn default_config_target(name: &str) -> String {
    format!(".{name}")
}

/// Why a `config_target` can't be meant, `None` if it's fine
fn config_target_problem(target: &str) -> Option<&'static str> {
    let path = Path::new(target);
    if target.is_empty() {
        Some("isn't a path")
    } else if path.components().eq([Component::RootDir]) {
        Some("is /, which docker can't mount over")
    } else if path.starts_with(HOME_SHORTHAND) {
        Some("starts with ~, drop ~/ as it's relative to the user's home already")
    } else if path.components().any(|part| part == Component::ParentDir) {
        Some("has a .. part")
    } else {
        None
    }
}

/// No whitespace, and no leading `-` that would make it an option
fn is_valid_image(image: &str) -> bool {
    !image.is_empty() && !image.starts_with('-') && !image.contains(char::is_whitespace)
}

/// Lowercase alphanumeric words joined by single `.`, `_`, `-` or `/`, like `sndbx` or `my.org/sndbx`
fn is_valid_tag_prefix(prefix: &str) -> bool {
    prefix.split(TAG_PREFIX_SEPARATORS).all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests;
