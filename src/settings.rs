use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::paths::expand_path;
use crate::{Abort, Result, paths};

const DEFAULT_TAG_PREFIX: &str = "sndbx";
// The images directory of the repo this binary was built from
const REPO_IMAGES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/images");

const EXAMPLE: &str = "\
# tag_prefix: sndbx # optional, images are tagged <tag_prefix>:<name>
# images_dir: ~/src/sndbx/images # optional, where build finds images, defaults to the sndbx repo's images/
# optional, docker run arguments for every image, before the image's own; these two harden the sandbox, see the README
docker_args: ['--security-opt=no-new-privileges', '--pids-limit=4096']
# optional, runs sandboxes behind a proxy that lets through only the hosts in ~/.config/sndbx/allowed_hosts, see the README
# proxy_image: ubuntu/squid@sha256:6a097f68bae708cedbabd6188d68c7e2e7a38cedd05a176e1cc0ba29e3bbe029
images:
  - name: claude
    # user: ubuntu # optional, whose home config_target is in, defaults to the user the image runs as
    # tag: my-claude:latest # optional, defaults to <tag_prefix>:<name>
    # config_dir: ~/.claude_docker # optional, relative paths resolve against ~/.config/sndbx, defaults to ~/.config/sndbx/<name>
    # config_target: .claude # optional, relative to the user's home, defaults to .<name>
    # ports: ['3000'] # optional, always published; quote them
    # docker_args: ['--memory=8g'] # optional, extra docker run arguments, after the top-level ones
    # proxy: false # optional, keeps the image off the proxy that proxy_image turns on
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
    /// Overrides the user the image runs as, whose home `config_target` is in
    pub user: Option<String>,
    /// Relative to the user's home
    pub config_target: String,
    pub ports: Vec<String>,
    pub docker_args: Vec<String>,
    /// Whether the image reaches the network through the proxy when `proxy_image` turns it on
    pub proxy: bool,
}

/// The settings file as written, an unknown key is an error, likely a misspelled one
#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default, deny_unknown_fields, expecting = "a mapping with an images list")]
struct File {
    #[serde(deserialize_with = "optional_text")]
    tag_prefix: Option<String>,
    #[serde(deserialize_with = "optional_text")]
    images_dir: Option<String>,
    /// For every image, before the image's own
    #[serde(deserialize_with = "strings")]
    docker_args: Vec<String>,
    /// The proxy's image, which turns the proxy on
    #[serde(deserialize_with = "optional_text")]
    proxy_image: Option<String>,
    images: Option<Vec<Entry>>,
}

#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
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

/// What `add_image` did
pub enum Added {
    /// An entry already runs the image
    Already,
    /// The image's entry runs another tag, this one
    OtherTag(String),
    /// Added an entry to the end of the settings file
    Appended,
    /// Created the settings file, the example with an entry for the image
    Created,
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
        let config_target = entry.config_target.clone().unwrap_or_else(|| format!(".{name}"));

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

    /// The tag the entry runs, its own or its name's default one
    fn entry_tag(&self, entry: &Entry) -> String {
        entry.tag.clone().unwrap_or_else(|| tag(self.tag_prefix(), &entry.name))
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.images().iter().find(|entry| entry.name == name)
    }

    /// The tag the image's entry runs, `None` when it has no entry
    pub fn configured_tag(&self, name: &str) -> Option<String> {
        self.entry(name).map(|entry| self.entry_tag(entry))
    }

    /// The images whose entries run the tag
    pub fn names_running(&self, tag: &str) -> Vec<&str> {
        self.images()
            .iter()
            .filter(|entry| self.entry_tag(entry) == tag)
            .map(|entry| entry.name.as_str())
            .collect()
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

    /// The proxy's image, `None` when the proxy is off
    pub fn proxy_image(&self) -> Option<&str> {
        self.file.proxy_image.as_deref()
    }

    fn images(&self) -> &[Entry] {
        self.file.images.as_deref().unwrap_or_default()
    }
}

/// Reads and validates the settings file
fn read(path: &Path) -> Result<Settings> {
    let content = paths::read_file(path)?;
    validate(&content, path)
}

/// The settings in `content`, read from `path`
fn validate(content: &str, path: &Path) -> Result<Settings> {
    let file = parse(content).with_context(|| format!("invalid {}", path.display()))?;

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

    if file.proxy_image.as_deref().is_some_and(|image| !is_valid_image(image)) {
        bail!(
            "invalid {}: proxy_image must be an image like ubuntu/squid",
            path.display()
        );
    }

    Ok(Settings { file })
}

/// The default tag of an image, `<tag_prefix>:<name>`
pub fn tag(tag_prefix: &str, name: &str) -> String {
    format!("{tag_prefix}:{name}")
}

/// The first document, empty settings when there's none or it's empty
fn parse(content: &str) -> Result<File, serde_norway::Error> {
    let Some(document) = serde_norway::Deserializer::from_str(content).next() else {
        return Ok(File::default());
    };
    Ok(Option::<File>::deserialize(document)?.unwrap_or_default())
}

/// Something docker can take as an image: not empty, no whitespace, no `-` first that would make it an option
fn is_valid_image(image: &str) -> bool {
    !image.is_empty() && !image.starts_with('-') && !image.contains(char::is_whitespace)
}

/// Lowercase alphanumeric words joined by single `.`, `_`, `-` or `/`, like `sndbx` or `my.org/sndbx`
fn is_valid_tag_prefix(prefix: &str) -> bool {
    prefix
        .split(['.', '_', '-', '/'])
        .all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn create_example(path: &Path) -> Result<()> {
    paths::write_file(path, EXAMPLE)
}

/// Adds an entry for the image to the settings unless one has it already, appending it to the settings file
/// so the rest of the file stays as written, or creating the example with it when there's no settings file
pub fn add_image(name: &str) -> Result<Added> {
    let path = path();
    let exists = path.exists();
    let content = if exists {
        paths::read_file(&path)?
    } else {
        EXAMPLE.to_string()
    };
    let settings = validate(&content, &path)?;

    let tag = tag(settings.tag_prefix(), name);
    let added = if !settings.names_running(&tag).is_empty() {
        Added::Already
    } else if let Some(other) = settings.configured_tag(name) {
        Added::OtherTag(other)
    } else {
        Added::Appended
    };

    let addition = match added {
        Added::Appended => addition(&settings.file, &content, name)
            .ok_or_else(|| anyhow!("can't tell where the images list in {} ends", path.display()))?,
        _ => String::new(),
    };
    if !exists {
        paths::write_file(&path, &format!("{content}{addition}"))?;
        return Ok(Added::Created);
    }
    if !addition.is_empty() {
        // Appending writes through a symlink and keeps the file's permissions, unlike replacing the file
        OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(addition.as_bytes()))
            .with_context(|| format!("can't write {}", path.display()))?;
    }
    Ok(added)
}

/// An entry for the image as YAML, a list item at the start of the line
pub fn entry_yaml(name: &str) -> String {
    // Quoted, a name like `true` or `123` would be a bool or a number otherwise
    format!("- name: '{}'\n", name.replace('\'', "''"))
}

/// The text to append to `content` that adds an entry for the image to the images list and changes nothing else:
/// the entry indented like a list item in the file, the last first, or by 2 or none, or under a new `images` key.
/// `None` when none does, like for a flow list `[...]`, an `images` key before others, or a second document.
fn addition(file: &File, content: &str, name: &str) -> Option<String> {
    let mut expected = file.clone();
    expected.images.get_or_insert_default().push(Entry {
        name: name.to_string(),
        ..Entry::default()
    });

    let newline = if content.is_empty() || content.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let entry = entry_yaml(name);
    let list_indents = content.lines().rev().filter_map(list_item_indent);
    let mut candidates: Vec<String> = Vec::new();
    for indent in list_indents.chain([2, 0]) {
        let candidate = format!("{}{entry}", " ".repeat(indent));
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates.push(format!("images:\n  {entry}"));

    candidates
        .into_iter()
        .map(|candidate| format!("{newline}{candidate}"))
        .find(|addition| parse(&format!("{content}{addition}")).is_ok_and(|parsed| parsed == expected))
}

/// Comments out the image's entry in the settings file, keeping the rest of the file as written.
/// False when there's no settings file or no entry.
pub fn comment_out_image(name: &str) -> Result<bool> {
    let path = path();
    if !path.exists() {
        return Ok(false);
    }
    let content = paths::read_file(&path)?;
    let settings = validate(&content, &path)?;
    let Some(index) = settings.images().iter().position(|entry| entry.name == name) else {
        return Ok(false);
    };

    let marker = format!("# Removed by sndbx rm on {}:", jiff::Zoned::now().strftime("%F"));
    let Some(commented) = commented_out(&settings.file, &content, index, &marker) else {
        bail!("can't tell where its entry in {} is", path.display());
    };
    paths::write_file(&path, &commented)?;
    Ok(true)
}

/// `content` with the entry at `index` in the images list commented out under `marker`, and nothing else changed:
/// the first list item that, commented out, leaves the settings without that entry. `None` when none does,
/// like in a flow list.
fn commented_out(file: &File, content: &str, index: usize, marker: &str) -> Option<String> {
    let mut expected = file.clone();
    expected.images.as_mut()?.remove(index);
    let expected = without_empty_images(expected);

    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    list_items(&lines)
        .map(|item| comment_out(&lines, &item, marker))
        .find(|commented| parse(commented).is_ok_and(|parsed| without_empty_images(parsed) == expected))
}

/// The lines `start..end` of a list item, which starts at `indent`
struct ListItem {
    start: usize,
    end: usize,
    indent: usize,
}

/// Each list item: from its `- ` line to the next line indented as far as it or less, comments and blank lines
/// aside, without the comments and blank lines at its end
fn list_items<'a>(lines: &'a [&str]) -> impl Iterator<Item = ListItem> + 'a {
    lines.iter().enumerate().filter_map(|(start, line)| {
        let indent = list_item_indent(line)?;
        let rest = &lines[start + 1..];
        let next = rest
            .iter()
            .position(|line| !is_blank_or_comment(line) && indent_of(line) <= indent)
            .unwrap_or(rest.len());
        let content = rest[..next]
            .iter()
            .rposition(|line| !is_blank_or_comment(line))
            .map_or(0, |last| last + 1);
        Some(ListItem {
            start,
            end: start + 1 + content,
            indent,
        })
    })
}

/// `lines` with the item's lines commented out at its indent under `marker`, its comments and blank lines as they are
fn comment_out(lines: &[&str], item: &ListItem, marker: &str) -> String {
    let mut commented = lines[..item.start].concat();
    commented.push_str(&" ".repeat(item.indent));
    commented.push_str(marker);
    commented.push('\n');
    for line in &lines[item.start..item.end] {
        if is_blank_or_comment(line) {
            commented.push_str(line);
        } else {
            let (indent, rest) = line.split_at(item.indent);
            commented.push_str(indent);
            commented.push_str("# ");
            commented.push_str(rest);
        }
    }
    // The file's last line may have no newline
    if !commented.ends_with('\n') {
        commented.push('\n');
    }
    commented.push_str(&lines[item.end..].concat());
    commented
}

fn is_blank_or_comment(line: &str) -> bool {
    matches!(line.trim_start().chars().next(), None | Some('#'))
}

/// Without an empty images list, which a list with all its entries commented out is: no list at all
fn without_empty_images(mut file: File) -> File {
    if file.images.as_ref().is_some_and(Vec::is_empty) {
        file.images = None;
    }
    file
}

/// The indent of a line that starts a list item, like `  - name: a`
fn list_item_indent(line: &str) -> Option<usize> {
    let item = line.trim_start_matches(' ').trim_end();
    (item == "-" || item.starts_with("- ")).then(|| indent_of(line))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
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

    fn added(content: &str, name: &str) -> Option<String> {
        addition(&parse(content).unwrap(), content, name).map(|addition| format!("{content}{addition}"))
    }

    #[test]
    fn appends_to_the_images_list() {
        assert_eq!(
            added("images:\n  - name: a\n", "b").unwrap(),
            "images:\n  - name: a\n  - name: 'b'\n"
        );
        assert_eq!(
            added("images:\n- name: a\n  user: x", "b").unwrap(),
            "images:\n- name: a\n  user: x\n- name: 'b'\n"
        );
        assert_eq!(
            added("tag_prefix: x\n", "b").unwrap(),
            "tag_prefix: x\nimages:\n  - name: 'b'\n"
        );
        assert_eq!(
            added("images:\n    - name: a\n      docker_args:\n        - x\n", "b").unwrap(),
            "images:\n    - name: a\n      docker_args:\n        - x\n    - name: 'b'\n"
        );
        assert_eq!(added("", "b").unwrap(), "images:\n  - name: 'b'\n");
        assert_eq!(added("images: []\n# a\n", "b"), None);
        assert_eq!(added("images:\n  - name: a\ndocker_args:\n  - x\n", "b"), None);
        assert_eq!(added("images: ~\n", "b"), None);
        assert_eq!(added("images:\n  - name: a\n---\nx: 1\n", "b"), None);
    }

    #[test]
    fn appends_to_the_example() {
        let example = added(EXAMPLE, "b").unwrap();
        assert!(example.ends_with("\n  - name: 'b'\n"), "{example}");
    }

    #[test]
    fn quotes_names() {
        for name in ["true", "123", "null", "it's"] {
            let content = added("images:\n  - name: a\n", name).unwrap();
            assert_eq!(parse(&content).unwrap().images.unwrap()[1].name, name);
        }
    }

    fn commented(content: &str, index: usize) -> Option<String> {
        commented_out(&parse(content).unwrap(), content, index, "# Removed:")
    }

    #[test]
    fn comments_out_an_entry() {
        assert_eq!(
            commented("images:\n  - name: a\n    ports: ['1']\n  - name: b\n", 0).unwrap(),
            "images:\n  # Removed:\n  # - name: a\n  #   ports: ['1']\n  - name: b\n"
        );
        assert_eq!(
            commented("images:\n- name: a\n- user: x\n  name: b\ndocker_args: [y]\n", 1).unwrap(),
            "images:\n- name: a\n# Removed:\n# - user: x\n#   name: b\ndocker_args: [y]\n"
        );
    }

    #[test]
    fn comments_out_an_entry_with_a_list() {
        let content =
            "images:\n  - name: a\n    docker_args:\n      - x\n\n    # mine\n    ports: ['1']\n  - name: b\n";
        assert_eq!(
            commented(content, 0).unwrap(),
            "images:\n  # Removed:\n  # - name: a\n  #   docker_args:\n  #     - x\n\n    # mine\n  #   ports: ['1']\n  - name: b\n"
        );
        assert_eq!(
            commented(content, 1).unwrap(),
            "images:\n  - name: a\n    docker_args:\n      - x\n\n    # mine\n    ports: ['1']\n  # Removed:\n  # - name: b\n"
        );
    }

    #[test]
    fn comments_out_the_last_entry() {
        assert_eq!(
            commented("images:\n  - name: a\n\n# end", 0).unwrap(),
            "images:\n  # Removed:\n  # - name: a\n\n# end"
        );
        assert_eq!(
            commented("images:\n  - name: a", 0).unwrap(),
            "images:\n  # Removed:\n  # - name: a\n"
        );
    }

    #[test]
    fn comments_out_only_whole_entries() {
        assert_eq!(commented("images: [{name: a}]\n", 0), None);
        assert_eq!(commented("docker_args:\n  - x\nimages: [{name: a}]\n", 0), None);
    }

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
        assert_eq!(parse("images: []\n---\nimages: 1\n").unwrap().images, Some(vec![]));
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
