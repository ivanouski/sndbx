//! Changes to the settings file that keep the rest of it as written

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;
use serde_norway::{Mapping, Value};

use super::{EXAMPLE, Entry, File, parse, path, validate};
use crate::{Result, paths, unique};

/// The entry build adds for an image: its name, and the keys from the image's directory as written there
pub struct NewEntry {
    entry: Entry,
    keys: String,
}

impl NewEntry {
    /// The entry with the keys in `keys`, all of an entry's but `name`, as read from `path`; none when it's empty
    pub fn new(name: &str, keys: &str, path: &Path) -> Result<Self> {
        let invalid = || format!("invalid {}", path.display());
        let mut mapping = parse_keys(keys).with_context(invalid)?;
        if mapping.contains_key("name") {
            bail!("invalid {}: name is the image directory's", path.display());
        }
        mapping.insert("name".into(), name.into());
        let entry = Entry::deserialize(Value::Mapping(mapping)).with_context(invalid)?;

        let new = Self {
            entry,
            keys: keys.trim_end().to_string(),
        };
        // Indented under the name, flow style or a second document would read differently or not at all
        let parsed: Option<Vec<Entry>> = serde_norway::from_str(&new.item(0)).ok();
        if parsed.as_deref() != Some(std::slice::from_ref(&new.entry)) {
            bail!("invalid {}: write each key at the start of a line", path.display());
        }
        Ok(new)
    }

    pub fn name(&self) -> &str {
        &self.entry.name
    }

    pub fn has_config_target(&self) -> bool {
        self.entry.config_target.is_some()
    }

    /// The entry as a YAML list item at `indent`, its keys indented under its name
    pub fn item(&self, indent: usize) -> String {
        let indent = " ".repeat(indent);
        // Quoted, a name like `true` or `123` would be a bool or a number otherwise
        let mut item = format!("{indent}- name: '{}'\n", self.entry.name.replace('\'', "''"));
        for line in self.keys.lines() {
            if !line.trim().is_empty() {
                item.push_str(&indent);
                item.push_str("  ");
                item.push_str(line);
            }
            item.push('\n');
        }
        item
    }
}

/// The keys in the first document, none when there's none or it's empty
fn parse_keys(keys: &str) -> Result<Mapping, serde_norway::Error> {
    let Some(document) = serde_norway::Deserializer::from_str(keys).next() else {
        return Ok(Mapping::new());
    };
    Ok(Option::<Mapping>::deserialize(document)?.unwrap_or_default())
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

/// Adds an entry for the image to the settings unless one has it already, appending it to the settings file
/// so the rest of the file stays as written, or creating the example with it when there's no settings file
pub fn add_image(new: &NewEntry) -> Result<Added> {
    let name = new.name();
    let path = path();
    let exists = path.exists();
    let content = if exists {
        paths::read_file(&path)?
    } else {
        EXAMPLE.to_string()
    };
    let settings = validate(&content, &path)?;

    let tag = settings.default_tag(name);
    let added = if !settings.names_running(&tag).is_empty() {
        Added::Already
    } else if let Some(other) = settings.configured_tag(name) {
        Added::OtherTag(other)
    } else {
        Added::Appended
    };

    let addition = match added {
        Added::Appended => addition(&settings.file, &content, new)
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

/// The text to append to `content` that adds the entry to the images list and changes nothing else:
/// the entry indented like a list item in the file, the last first, or by 2 or none, or under a new `images` key.
/// `None` when none does, like for a flow list `[...]`, an `images` key before others, or a second document.
fn addition(file: &File, content: &str, new: &NewEntry) -> Option<String> {
    let mut expected = file.clone();
    expected.images.get_or_insert_default().push(new.entry.clone());

    let newline = if content.is_empty() || content.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let list_indents = content.lines().rev().filter_map(list_item_indent);
    let mut candidates: Vec<String> = unique(list_indents.chain([2, 0]))
        .into_iter()
        .map(|indent| new.item(indent))
        .collect();
    candidates.push(format!("images:\n{}", new.item(2)));

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

#[cfg(test)]
mod tests {
    use super::*;

    fn new_entry(name: &str, keys: &str) -> Result<NewEntry> {
        NewEntry::new(name, keys, Path::new("sndbx.yml"))
    }

    fn added_with(content: &str, name: &str, keys: &str) -> Option<String> {
        addition(&parse(content).unwrap(), content, &new_entry(name, keys).unwrap())
            .map(|addition| format!("{content}{addition}"))
    }

    fn added(content: &str, name: &str) -> Option<String> {
        added_with(content, name, "")
    }

    fn error(name: &str, keys: &str) -> String {
        format!("{:#}", new_entry(name, keys).err().unwrap())
    }

    #[test]
    fn appends_the_keys_under_the_name() {
        let keys = "# web UI\nports: ['127.0.0.1:3080:3080']\n\nproxy: false\n\n";
        assert_eq!(
            added_with("images:\n    - name: a\n", "b", keys).unwrap(),
            "images:\n    - name: a\n    - name: 'b'\n      # web UI\n      ports: ['127.0.0.1:3080:3080']\n\n      proxy: false\n"
        );
        assert_eq!(
            added_with("", "b", "docker_args:\n  - x\n").unwrap(),
            "images:\n  - name: 'b'\n    docker_args:\n      - x\n"
        );
        assert_eq!(
            added_with("images:\n  - name: a\n", "b", "# nothing\n").unwrap(),
            "images:\n  - name: a\n  - name: 'b'\n    # nothing\n"
        );
    }

    #[test]
    fn checks_the_keys() {
        assert_eq!(
            error("b", "name: c\n"),
            "invalid sndbx.yml: name is the image directory's"
        );
        assert!(error("b", "port: '1'\n").contains("unknown field `port`"));
        assert!(error("b", "ports: [1]\n").contains("expected a string"));
        assert!(error("b", "- x\n").starts_with("invalid sndbx.yml: "));
        for keys in ["{ports: ['1']}\n", "user: x\n---\ntag: y\n"] {
            assert_eq!(
                error("b", keys),
                "invalid sndbx.yml: write each key at the start of a line"
            );
        }
        assert!(!new_entry("b", "").unwrap().has_config_target());
        assert!(new_entry("b", "config_target: .config/b").unwrap().has_config_target());
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
}
