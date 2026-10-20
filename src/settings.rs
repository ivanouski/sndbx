use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::paths::{expand_path, join};
use crate::{Abort, Result, paths};

pub const HOME_ROOT: &str = "/home";
const DEFAULT_TAG_PREFIX: &str = "sndbx";
// The images directory of the repo this binary was built from
const REPO_IMAGES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/images");

const EXAMPLE: &str = "\
# tag_prefix: sndbx # optional, images are tagged <tag_prefix>:<name>
# images_dir: ~/src/sndbx/images # optional, where build finds images, defaults to the sndbx repo's images/
# optional, docker run arguments for every image, before the image's own; these two harden the sandbox, see the README
docker_args: ['--security-opt=no-new-privileges', '--pids-limit=4096']
images:
  - name: claude
    user: ubuntu
    # tag: my-claude:latest # optional, defaults to <tag_prefix>:<name>
    # config_dir: ~/.claude_docker # optional, relative paths resolve against ~/.config/sndbx, defaults to ~/.config/sndbx/<name>
    # config_target: .claude # optional, relative to the user's home, defaults to .<name>
    # ports: ['3000'] # optional, always published; quote them
    # docker_args: ['--memory=8g'] # optional, extra docker run arguments, after the top-level ones
";

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
    pub config_target: String,
    pub ports: Vec<String>,
    pub docker_args: Vec<String>,
}

/// The settings file as written, an unknown key is an error, likely a misspelled one
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields, expecting = "a mapping with an images list")]
struct File {
    #[serde(deserialize_with = "optional_text")]
    tag_prefix: Option<String>,
    #[serde(deserialize_with = "optional_text")]
    images_dir: Option<String>,
    /// For every image, before the image's own
    #[serde(deserialize_with = "strings")]
    docker_args: Vec<String>,
    images: Option<Vec<Entry>>,
}

#[derive(Deserialize)]
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
}

/// The settings file, or the defaults where it has none
#[derive(Default)]
pub struct Settings {
    file: File,
}

impl Settings {
    /// Reads the settings, or creates an example and stops when there's no settings file
    pub fn load() -> Result<Self> {
        let path = path();
        if !path.exists() {
            create_example(&path)?;
            return Err(Abort(format!(
                "Created an example config at {}, edit it and run again.",
                path.display()
            ))
            .into());
        }
        read(&path)
    }

    /// Reads the settings, the defaults when there's no settings file
    pub fn load_or_default() -> Result<Self> {
        let path = path();
        if !path.exists() {
            return Ok(Self::default());
        }
        read(&path)
    }

    pub fn image(&self, name: &str) -> Result<Image> {
        let Some(entry) = self.images().iter().find(|entry| entry.name == name) else {
            bail!(
                "unknown image '{name}', expected one of: {}",
                self.image_names().join(", ")
            );
        };

        let settings_path = path();
        let settings_path = settings_path.display();
        let Some(user) = &entry.user else {
            bail!("image '{name}' has no user in {settings_path}");
        };
        let config_dir = match entry.config_dir.as_deref() {
            None => name,
            Some("") => bail!("image '{name}' has a config_dir that isn't a path in {settings_path}"),
            Some(dir) => dir,
        };
        let config_target = entry.config_target.clone().unwrap_or_else(|| format!(".{name}"));

        Ok(Image {
            name: name.to_string(),
            tag: entry
                .tag
                .clone()
                .unwrap_or_else(|| format!("{}:{name}", self.tag_prefix())),
            config_dir: expand_path(config_dir, Some(&dir())),
            config_target: join(&[HOME_ROOT, user, &config_target]),
            ports: entry.ports.clone(),
            // Later arguments win in docker, so the image's override the top-level ones
            docker_args: self
                .file
                .docker_args
                .iter()
                .chain(&entry.docker_args)
                .cloned()
                .collect(),
        })
    }

    pub fn image_names(&self) -> Vec<String> {
        self.images().iter().map(|entry| entry.name.clone()).collect()
    }

    pub fn tag_prefix(&self) -> &str {
        self.file.tag_prefix.as_deref().unwrap_or(DEFAULT_TAG_PREFIX)
    }

    /// The default finds the repo's images as long as it's where the binary was built
    pub fn images_dir(&self) -> PathBuf {
        match self.file.images_dir.as_deref() {
            Some(images_dir) => expand_path(images_dir, Some(&dir())),
            None => expand_path(REPO_IMAGES_DIR, None),
        }
    }

    fn images(&self) -> &[Entry] {
        self.file.images.as_deref().unwrap_or_default()
    }
}

/// Reads and validates the settings file
fn read(path: &Path) -> Result<Settings> {
    let content = std::fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))?;
    let file = parse(&content).with_context(|| format!("invalid {}", path.display()))?;

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

    Ok(Settings { file })
}

/// The first document, empty settings when there's none or it's empty
fn parse(content: &str) -> Result<File, serde_norway::Error> {
    let Some(document) = serde_norway::Deserializer::from_str(content).next() else {
        return Ok(File::default());
    };
    Ok(Option::<File>::deserialize(document)?.unwrap_or_default())
}

/// Lowercase alphanumeric words joined by single `.`, `_`, `-` or `/`, like `sndbx` or `my.org/sndbx`
fn is_valid_tag_prefix(prefix: &str) -> bool {
    prefix
        .split(['.', '_', '-', '/'])
        .all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn create_example(path: &Path) -> Result<()> {
    std::fs::create_dir_all(dir())
        .and_then(|()| std::fs::write(path, EXAMPLE))
        .with_context(|| format!("can't create {}", path.display()))
}

/// A YAML string; `serde_norway` would also turn numbers and booleans into a `String`
struct Text(String);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextVisitor;

        impl Visitor<'_> for TextVisitor {
            type Value = Text;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Text, E> {
                Ok(Text(value.to_string()))
            }
        }

        deserializer.deserialize_any(TextVisitor)
    }
}

fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Text::deserialize(deserializer).map(|Text(text)| text)
}

fn optional_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<Text>::deserialize(deserializer)?.map(|Text(text)| text))
}

/// A list of strings, or one string, none when it's null
fn strings<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    struct StringsVisitor;

    impl<'de> Visitor<'de> for StringsVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or a list of strings")
        }

        fn visit_unit<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Vec<String>, E> {
            Ok(vec![value.to_string()])
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut strings = Vec::new();
            while let Some(Text(string)) = seq.next_element()? {
                strings.push(string);
            }
            Ok(strings)
        }
    }

    deserializer.deserialize_any(StringsVisitor)
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
        assert!(
            parse("images: []\n---\nimages: 1\n")
                .unwrap()
                .images
                .unwrap()
                .is_empty()
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
    fn unknown_keys() {
        let message = parse("image: []\n").err().unwrap().to_string();
        assert!(message.contains("unknown field `image`"), "{message}");
        let message = parse("images:\n  - name: a\n    port: '3000'\n")
            .err()
            .unwrap()
            .to_string();
        assert!(message.contains("images[0]: unknown field `port`"), "{message}");
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
        ] {
            let message = parse(invalid).err().unwrap().to_string();
            assert!(message.contains("expected a string"), "{invalid:?}: {message}");
        }
    }
}
