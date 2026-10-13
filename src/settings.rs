use yaml_rust2::{Yaml, YamlLoader};

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

pub fn dir() -> String {
    join(&[&paths::home(), ".config", "sndbx"])
}

pub fn path() -> String {
    join(&[&dir(), "settings.yml"])
}

pub struct Image {
    pub name: String,
    pub tag: String,
    pub config_dir: String,
    pub config_target: String,
    pub ports: Vec<String>,
    pub docker_args: Vec<String>,
}

#[derive(Default)]
pub struct Settings {
    settings: Option<Yaml>,
}

impl Settings {
    pub fn image(&mut self, name: &str) -> Result<Image> {
        let path = path();
        let Some(image) = self
            .images()?
            .into_iter()
            .find(|entry| entry["name"].as_str() == Some(name))
        else {
            return error(format!(
                "unknown image '{name}', expected one of: {}",
                self.image_names()?.join(", ")
            ));
        };

        let user = match &image["user"] {
            Yaml::String(user) => user.clone(),
            Yaml::BadValue | Yaml::Null => return error(format!("image '{name}' has no user in {path}")),
            _ => return error(format!("image '{name}' has a user that isn't a string in {path}")),
        };

        let Some(ports) = strings(&image["ports"]) else {
            return error(format!(
                "image '{name}' has an unquoted port in {path}; quote it, e.g. \"3000\""
            ));
        };
        let Some(docker_args) = strings(&image["docker_args"]) else {
            return error(format!("image '{name}' has docker_args that aren't strings in {path}"));
        };

        let config_dir = match &image["config_dir"] {
            Yaml::BadValue | Yaml::Null => name.to_string(),
            Yaml::String(dir) if !dir.is_empty() => dir.clone(),
            _ => return error(format!("image '{name}' has a config_dir that isn't a path in {path}")),
        };

        let tag = match optional_string(&image["tag"], name, "tag")? {
            Some(tag) => tag,
            None => format!("{}:{name}", self.tag_prefix()?),
        };
        let config_target =
            optional_string(&image["config_target"], name, "config_target")?.unwrap_or_else(|| format!(".{name}"));

        Ok(Image {
            name: name.to_string(),
            tag,
            config_dir: expand_path(&config_dir, Some(&dir())),
            config_target: join(&[HOME_ROOT, &user, &config_target]),
            ports,
            docker_args,
        })
    }

    pub fn image_names(&mut self) -> Result<Vec<String>> {
        Ok(self
            .images()?
            .iter()
            .filter_map(|entry| entry["name"].as_str().map(ToString::to_string))
            .collect())
    }

    pub fn tag_prefix(&mut self) -> Result<String> {
        Ok(self
            .optional_setting("tag_prefix")?
            .unwrap_or_else(|| DEFAULT_TAG_PREFIX.to_string()))
    }

    /// The default finds the repo's images as long as it's where the binary was built.
    pub fn images_dir(&mut self) -> Result<String> {
        Ok(match self.optional_setting("images_dir")? {
            Some(dir) => expand_path(&dir, Some(&self::dir())),
            None => expand_path(REPO_IMAGES_DIR, None),
        })
    }

    fn images(&mut self) -> Result<Vec<Yaml>> {
        Ok(self.settings()?["images"].as_vec().cloned().unwrap_or_default())
    }

    /// A string setting, validated already, without creating the settings file
    fn optional_setting(&mut self, key: &str) -> Result<Option<String>> {
        if !paths::exists(&path()) {
            return Ok(None);
        }
        Ok(self.settings()?[key].as_str().map(ToString::to_string))
    }

    fn settings(&mut self) -> Result<&Yaml> {
        if self.settings.is_none() {
            let settings = load()?;
            validate(&settings)?;
            self.settings = Some(settings);
        }
        Ok(self.settings.as_ref().expect("settings were just loaded"))
    }
}

/// The first document, an empty mapping when there's none or it's empty
fn load() -> Result<Yaml> {
    let path = path();
    if !paths::exists(&path) {
        create_example()?;
    }

    let content = std::fs::read_to_string(&path).map_err(|e| Error::Message(format!("can't read {path}: {e}")))?;
    let documents = YamlLoader::load_from_str(&content).map_err(|e| Error::Message(format!("invalid {path}: {e}")))?;
    Ok(match documents.into_iter().next() {
        None | Some(Yaml::Null) => Yaml::Hash(yaml_rust2::yaml::Hash::new()),
        Some(settings) => settings,
    })
}

fn validate(settings: &Yaml) -> Result<()> {
    let path = path();

    if settings.as_hash().is_none() {
        return error(format!("invalid {path}: expected a mapping with an images list"));
    }

    let valid_prefix = match &settings["tag_prefix"] {
        Yaml::BadValue | Yaml::Null => true,
        Yaml::String(prefix) => valid_tag_prefix(prefix),
        _ => false,
    };
    if !valid_prefix {
        return error(format!(
            "invalid {path}: tag_prefix must be a lowercase image name, e.g. sndbx"
        ));
    }

    if !matches!(settings["images_dir"], Yaml::BadValue | Yaml::Null | Yaml::String(_)) {
        return error(format!("invalid {path}: images_dir must be a path"));
    }

    let images = match &settings["images"] {
        Yaml::BadValue | Yaml::Null => &[][..],
        Yaml::Array(images) => images.as_slice(),
        _ => return error(format!("invalid {path}: images must be a list")),
    };

    for (number, entry) in images.iter().enumerate() {
        if entry["name"].as_str().is_none() {
            return error(format!(
                "invalid {path}: image {} must be a mapping with a name",
                number + 1
            ));
        }
    }

    Ok(())
}

/// Lowercase alphanumeric words joined by single `.`, `_`, `-` or `/`, like `sndbx` or `my.org/sndbx`
fn valid_tag_prefix(prefix: &str) -> bool {
    let words_valid = prefix
        .split(['.', '_', '-', '/'])
        .all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    words_valid && !prefix.is_empty()
}

fn create_example() -> Result<()> {
    let path = path();
    std::fs::create_dir_all(dir())
        .and_then(|()| std::fs::write(&path, EXAMPLE))
        .map_err(|e| Error::Message(format!("can't create {path}: {e}")))?;

    Err(Error::Abort(format!(
        "Created an example config at {path}, edit it and run again."
    )))
}

/// A string setting of an image, `None` when it's unset
fn optional_string(value: &Yaml, name: &str, key: &str) -> Result<Option<String>> {
    match value {
        Yaml::String(value) => Ok(Some(value.clone())),
        Yaml::BadValue | Yaml::Null => Ok(None),
        _ => error(format!("image '{name}' has a {key} that isn't a string in {}", path())),
    }
}

/// The strings of a list setting, or its one string, none when it's unset, `None` if any isn't a string
fn strings(value: &Yaml) -> Option<Vec<String>> {
    match value {
        Yaml::BadValue | Yaml::Null => Some(Vec::new()),
        Yaml::String(value) => Some(vec![value.clone()]),
        Yaml::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(ToString::to_string))
            .collect(),
        _ => None,
    }
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
        let example = YamlLoader::load_from_str(EXAMPLE).unwrap().remove(0);
        validate(&example).unwrap();
        assert_eq!(example["images"][0]["name"].as_str(), Some("claude"));
    }
}
