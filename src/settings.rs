use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::paths::{expand_path, join};
use crate::{Error, Result, error, paths};

pub const HOME_ROOT: &str = "/home";
const DEFAULT_TAG_PREFIX: &str = "sndbx";
// The images directory of the repo this binary was built from
const REPO_IMAGES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/images");

const EXAMPLE: &str = "\
# tag_prefix: sndbx # optional, images are tagged <tag_prefix>:<name>
images:
  - name: claude
    user: ubuntu
    # tag: my-claude:latest # optional, defaults to <tag_prefix>:<name>
    # config_dir: ~/.claude_docker # optional, relative paths resolve against ~/.config/sndbx, defaults to ~/.config/sndbx/<name>
    # config_target: .claude # optional, relative to the user's home, defaults to .<name>
    # ports: ['3000'] # optional, always published; quote them
    # docker_args: ['--memory=8g'] # optional, extra docker run arguments
# images_dir: ~/src/sndbx/images # optional, where build finds images, defaults to the sndbx repo's images/
";

pub fn dir() -> PathBuf {
    paths::home().join(".config/sndbx")
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

/// The settings file as written, unknown keys are ignored
#[derive(Deserialize, Default)]
#[serde(default, expecting = "a mapping with an images list")]
struct File {
    #[serde(deserialize_with = "optional_text")]
    tag_prefix: Option<String>,
    #[serde(deserialize_with = "optional_text")]
    images_dir: Option<String>,
    images: Option<Vec<Entry>>,
}

#[derive(Deserialize)]
#[serde(expecting = "a mapping with a name")]
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

#[derive(Default)]
pub struct Settings {
    settings: Option<File>,
}

impl Settings {
    pub fn image(&mut self, name: &str) -> Result<Image> {
        let path = path();
        let path = path.display();
        let tag_prefix = self.tag_prefix()?;
        let Some(entry) = self.images()?.iter().find(|entry| entry.name == name) else {
            return error(format!(
                "unknown image '{name}', expected one of: {}",
                self.image_names()?.join(", ")
            ));
        };

        let Some(user) = &entry.user else {
            return error(format!("image '{name}' has no user in {path}"));
        };
        let config_dir = match entry.config_dir.as_deref() {
            None => name,
            Some("") => return error(format!("image '{name}' has a config_dir that isn't a path in {path}")),
            Some(dir) => dir,
        };
        let config_target = entry.config_target.clone().unwrap_or_else(|| format!(".{name}"));

        Ok(Image {
            name: name.to_string(),
            tag: entry.tag.clone().unwrap_or_else(|| format!("{tag_prefix}:{name}")),
            config_dir: expand_path(config_dir, Some(&dir())),
            config_target: join(&[HOME_ROOT, user, &config_target]),
            ports: entry.ports.clone(),
            docker_args: entry.docker_args.clone(),
        })
    }

    pub fn image_names(&mut self) -> Result<Vec<String>> {
        Ok(self.images()?.iter().map(|entry| entry.name.clone()).collect())
    }

    /// Doesn't create the settings file
    pub fn tag_prefix(&mut self) -> Result<String> {
        Ok(self
            .existing()?
            .and_then(|settings| settings.tag_prefix.clone())
            .unwrap_or_else(|| DEFAULT_TAG_PREFIX.to_string()))
    }

    /// The default finds the repo's images as long as it's where the binary was built.
    /// Doesn't create the settings file
    pub fn images_dir(&mut self) -> Result<PathBuf> {
        let images_dir = self.existing()?.and_then(|settings| settings.images_dir.as_deref());
        Ok(match images_dir {
            Some(dir) => expand_path(dir, Some(&self::dir())),
            None => expand_path(REPO_IMAGES_DIR, None),
        })
    }

    fn images(&mut self) -> Result<&[Entry]> {
        Ok(self.settings()?.images.as_deref().unwrap_or_default())
    }

    /// The settings without creating the file, `None` when there's none
    fn existing(&mut self) -> Result<Option<&File>> {
        if !path().exists() {
            return Ok(None);
        }
        self.settings().map(Some)
    }

    fn settings(&mut self) -> Result<&File> {
        if self.settings.is_none() {
            self.settings = Some(load()?);
        }
        Ok(self.settings.as_ref().expect("settings were just loaded"))
    }
}

/// Reads and validates the settings, creating an example when there's none
fn load() -> Result<File> {
    let path = path();
    if !path.exists() {
        create_example(&path)?;
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| Error::Message(format!("can't read {}: {e}", path.display())))?;
    let path = path.display();
    let settings = parse(&content).map_err(|e| Error::Message(format!("invalid {path}: {e}")))?;

    if settings
        .tag_prefix
        .as_deref()
        .is_some_and(|prefix| !valid_tag_prefix(prefix))
    {
        return error(format!(
            "invalid {path}: tag_prefix must be a lowercase image name, e.g. sndbx"
        ));
    }

    Ok(settings)
}

/// The first document, empty settings when there's none or it's empty
fn parse(content: &str) -> std::result::Result<File, serde_norway::Error> {
    let Some(document) = serde_norway::Deserializer::from_str(content).next() else {
        return Ok(File::default());
    };
    Ok(Option::<File>::deserialize(document)?.unwrap_or_default())
}

/// Lowercase alphanumeric words joined by single `.`, `_`, `-` or `/`, like `sndbx` or `my.org/sndbx`
fn valid_tag_prefix(prefix: &str) -> bool {
    let words_valid = prefix
        .split(['.', '_', '-', '/'])
        .all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    words_valid && !prefix.is_empty()
}

fn create_example(path: &Path) -> Result<()> {
    std::fs::create_dir_all(dir())
        .and_then(|()| std::fs::write(path, EXAMPLE))
        .map_err(|e| Error::Message(format!("can't create {}: {e}", path.display())))?;

    Err(Error::Abort(format!(
        "Created an example config at {}, edit it and run again.",
        path.display()
    )))
}

/// A YAML string; `serde_norway` would also turn numbers and booleans into a `String`
struct Text(String);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct TextVisitor;

        impl Visitor<'_> for TextVisitor {
            type Value = Text;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Text, E> {
                Ok(Text(value.to_string()))
            }
        }

        deserializer.deserialize_any(TextVisitor)
    }
}

fn text<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<String, D::Error> {
    Text::deserialize(deserializer).map(|Text(text)| text)
}

fn optional_text<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Option<String>, D::Error> {
    Ok(Option::<Text>::deserialize(deserializer)?.map(|Text(text)| text))
}

/// A list of strings, or one string, none when it's null
fn strings<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Vec<String>, D::Error> {
    struct StringsVisitor;

    impl<'de> Visitor<'de> for StringsVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or a list of strings")
        }

        fn visit_unit<E: de::Error>(self) -> std::result::Result<Vec<String>, E> {
            Ok(Vec::new())
        }

        fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Vec<String>, E> {
            Ok(vec![value.to_string()])
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Vec<String>, A::Error> {
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
            assert!(valid_tag_prefix(valid), "{valid}");
        }
        for invalid in ["", "Not Valid", "a--b", "-a", "a/", "Sndbx"] {
            assert!(!valid_tag_prefix(invalid), "{invalid}");
        }
    }

    #[test]
    fn example_parses() {
        let example = parse(EXAMPLE).unwrap();
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
        let image = parse("images:\n  - name: a\n    ports: '3000'\n    docker_args: [x, y]\n    z: 1\n")
            .unwrap()
            .images
            .unwrap()
            .remove(0);
        assert_eq!(image.ports, ["3000"]);
        assert_eq!(image.docker_args, ["x", "y"]);
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
        ] {
            let message = parse(invalid).err().unwrap().to_string();
            assert!(message.contains("expected a string"), "{invalid:?}: {message}");
        }
    }
}
