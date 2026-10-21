//! Changes to the settings file that keep the rest of it as written

use std::fs::OpenOptions;
use std::io::Write;

use anyhow::{Context, anyhow, bail};

use super::{EXAMPLE, Entry, File, parse, path, tag, validate};
use crate::{Result, paths};

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
}
