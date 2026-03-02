use std::collections::BTreeMap;
use std::collections::HashMap;

use dashmap::DashMap;

use crate::file_entry::FileEntry;
use crate::file_entry::Language;

#[derive(Debug)]
pub struct FileTree {
    files: DashMap<String, FileEntry>,
}

impl Default for FileTree {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTree {
    pub fn new() -> Self {
        Self {
            files: DashMap::new(),
        }
    }

    pub fn insert(&self, entry: FileEntry) {
        self.files.insert(entry.rel_path.clone(), entry);
    }

    pub fn remove(&self, rel_path: &str) -> Option<FileEntry> {
        self.files.remove(rel_path).map(|(_, value)| value)
    }

    pub fn get(&self, rel_path: &str) -> Option<FileEntry> {
        self.files.get(rel_path).map(|entry| entry.value().clone())
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn all_paths(&self) -> Vec<String> {
        self.files.iter().map(|entry| entry.key().clone()).collect()
    }

    pub fn all_paths_with_language(&self) -> Vec<(String, Language)> {
        self.files
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().language))
            .collect()
    }

    pub fn language_breakdown(&self) -> Vec<(Language, usize)> {
        let mut counts: HashMap<Language, usize> = HashMap::new();
        for entry in self.files.iter() {
            let language = entry.value().language;
            *counts.entry(language).or_insert(0) += 1;
        }
        let mut out: Vec<(Language, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }

    pub fn render_tree(&self, depth: usize) -> String {
        let mut paths = self.all_paths();
        paths.sort();

        let mut root: BTreeMap<String, TreeNode> = BTreeMap::new();
        for path in paths {
            let parts: Vec<&str> = path.split('/').collect();
            insert_into_tree(&mut root, &parts, 0);
        }

        let mut out = String::new();
        render_tree_node(&root, &mut out, "", depth, 0);
        out
    }
}

enum TreeNode {
    File,
    Dir(BTreeMap<String, TreeNode>),
}

fn insert_into_tree(tree: &mut BTreeMap<String, TreeNode>, parts: &[&str], index: usize) {
    if index >= parts.len() {
        return;
    }

    let name = parts[index].to_string();
    if index == parts.len() - 1 {
        tree.entry(name).or_insert(TreeNode::File);
        return;
    }

    let node = tree
        .entry(name)
        .or_insert_with(|| TreeNode::Dir(BTreeMap::new()));
    if let TreeNode::Dir(children) = node {
        insert_into_tree(children, parts, index + 1);
    }
}

fn render_tree_node(
    tree: &BTreeMap<String, TreeNode>,
    out: &mut String,
    prefix: &str,
    max_depth: usize,
    current_depth: usize,
) {
    if max_depth > 0 && current_depth >= max_depth {
        return;
    }

    let entries: Vec<_> = tree.iter().collect();
    for (idx, (name, node)) in entries.iter().enumerate() {
        let is_last = idx == entries.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        match node {
            TreeNode::File => {
                out.push_str(&format!("{prefix}{connector}{name}\n"));
            }
            TreeNode::Dir(children) => {
                out.push_str(&format!("{prefix}{connector}{name}/\n"));
                render_tree_node(
                    children,
                    out,
                    &format!("{prefix}{child_prefix}"),
                    max_depth,
                    current_depth + 1,
                );
            }
        }
    }
}
