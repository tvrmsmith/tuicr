use super::*;
use std::path::Path;

/// One directory row the file list emits for a file.
pub(in crate::app) struct DirRow {
    /// Full path of the deepest directory the row stands for. Doubles as the
    /// row's `expanded_dirs` key.
    pub path: String,
    /// Rendered text, trailing `/` included. Under `FileTreeMode::Compact`
    /// this spans several path segments.
    pub label: String,
}

/// Which directory rows the active tree mode emits, for one `diff_files`
/// state. Built once and reused across a whole pass, because `Compact` has to
/// see the entire file set before it can tell which chains join.
///
/// The **ungrouped** tree only. The grouped sidebar has no directory rows, so
/// nothing here reaches it (`docs/SIDEBAR_MODEL.md` point 5) — which is also
/// why deciding joins over the whole of `diff_files` is right rather than
/// merely convenient: ungrouped, the whole of `diff_files` is the tree.
pub(in crate::app) struct TreeLayout {
    mode: FileTreeMode,
    /// Directories that merge into their single child instead of taking a
    /// row of their own. Always empty outside `Compact`.
    joined: HashSet<String>,
}

impl TreeLayout {
    fn new(mode: FileTreeMode, diff_files: &[DiffFile]) -> Self {
        let joined = match mode {
            FileTreeMode::Compact => Self::joined_dirs(diff_files),
            FileTreeMode::Nested | FileTreeMode::Flat => HashSet::new(),
        };
        Self { mode, joined }
    }

    /// A directory joins into its child when it has exactly one directory
    /// child and no files of its own — VS Code's "compact folders" rule.
    fn joined_dirs(diff_files: &[DiffFile]) -> HashSet<String> {
        let mut dir_children: HashMap<String, HashSet<String>> = HashMap::new();
        let mut holds_own_files: HashSet<String> = HashSet::new();

        for file in diff_files {
            let mut child: Option<String> = None;
            for dir in ancestors(file.display_path()).into_iter().rev() {
                match child {
                    Some(child) => {
                        dir_children.entry(dir.clone()).or_default().insert(child);
                    }
                    None => {
                        holds_own_files.insert(dir.clone());
                    }
                }
                child = Some(dir);
            }
        }

        dir_children
            .into_iter()
            .filter(|(dir, children)| children.len() == 1 && !holds_own_files.contains(dir))
            .map(|(dir, _)| dir)
            .collect()
    }

    /// Directory rows above `path`, outermost first. The file's depth is the
    /// number of rows returned, so `Flat`'s empty result puts every file at
    /// depth 0.
    pub(in crate::app) fn dir_rows(&self, path: &Path) -> Vec<DirRow> {
        if self.mode == FileTreeMode::Flat {
            return Vec::new();
        }

        let ancestors = ancestors(path);
        let mut rows = Vec::new();
        let mut chain_start = 0;
        for (idx, dir) in ancestors.iter().enumerate() {
            if idx + 1 < ancestors.len() && self.joined.contains(dir) {
                continue;
            }
            let label_start = match chain_start {
                0 => 0,
                start => ancestors[start - 1].len() + 1,
            };
            rows.push(DirRow {
                path: dir.clone(),
                label: format!("{}/", &dir[label_start..]),
            });
            chain_start = idx + 1;
        }
        rows
    }
}

/// Directory ancestors of `path`, outermost first, excluding the repo root.
fn ancestors(path: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent != Path::new("") {
            dirs.push(parent.to_string_lossy().to_string());
        }
        current = parent.parent();
    }
    dirs.reverse();
    dirs
}

impl App {
    pub(in crate::app) fn tree_layout(&self) -> TreeLayout {
        TreeLayout::new(self.file_tree_mode, &self.diff_files)
    }

    pub fn file_list_down(&mut self, n: usize) {
        let visible_items = self.build_visible_items();
        let max_idx = visible_items.len().saturating_sub(1);
        let new_idx = (self.file_list_state.selected() + n).min(max_idx);
        self.file_list_state.select(new_idx);
        self.follow_file_list_in_single_file_view();
    }

    pub fn file_list_up(&mut self, n: usize) {
        let new_idx = self.file_list_state.selected().saturating_sub(n);
        self.file_list_state.select(new_idx);
        self.follow_file_list_in_single_file_view();
    }

    /// In single-file view the diff panel always shows one file at a time,
    /// so navigating the file list with j/k should reveal the highlighted
    /// file immediately instead of waiting for Enter. Skips directories
    /// (jumping there has no diff target) and no-ops outside single-file
    /// view to keep multi-file scrolling exactly as before.
    fn follow_file_list_in_single_file_view(&mut self) {
        if !self.is_single_file_view {
            return;
        }
        if let Some(FileTreeItem::File { file_idx, .. }) = self.get_selected_tree_item() {
            self.jump_to_file(file_idx);
        }
    }

    /// Scroll the file-list viewport down by `lines` without moving the
    /// selection unless it would fall off the top of the viewport.
    pub fn file_list_viewport_scroll_down(&mut self, lines: usize) {
        let total = self.build_visible_items().len();
        let viewport = self.file_list_state.viewport_height.max(1);
        let max_offset = total.saturating_sub(viewport);
        let new_offset = (self.file_list_state.list_state.offset() + lines).min(max_offset);
        *self.file_list_state.list_state.offset_mut() = new_offset;
        if self.file_list_state.selected() < new_offset {
            self.file_list_state.select(new_offset);
        }
    }

    /// Scroll the file-list viewport up by `lines` without moving the
    /// selection unless it would fall off the bottom of the viewport.
    pub fn file_list_viewport_scroll_up(&mut self, lines: usize) {
        let viewport = self.file_list_state.viewport_height.max(1);
        let new_offset = self
            .file_list_state
            .list_state
            .offset()
            .saturating_sub(lines);
        *self.file_list_state.list_state.offset_mut() = new_offset;
        let max_visible = (new_offset + viewport).saturating_sub(1);
        if self.file_list_state.selected() > max_visible {
            self.file_list_state.select(max_visible);
        }
    }

    pub fn file_list_idx_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.file_list_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        let idx = self.file_list_state.list_state.offset() + rel;
        let total = self.build_visible_items().len();
        (idx < total).then_some(idx)
    }

    pub fn toggle_diff_view_mode(&mut self) {
        if self.is_pristine_mode {
            // Side-by-side has nothing to show in pristine mode: there is no
            // diff, so the two panes would render identical content. Keep
            // the view unified and tell the user why the toggle did nothing.
            self.set_message("side-by-side not available in pristine mode");
            return;
        }
        self.diff_view_mode = match self.diff_view_mode {
            DiffViewMode::Unified => DiffViewMode::SideBySide,
            DiffViewMode::SideBySide => DiffViewMode::Unified,
        };
        let mode_name = match self.diff_view_mode {
            DiffViewMode::Unified => "unified",
            DiffViewMode::SideBySide => "side-by-side",
        };
        self.set_message(format!("Diff view mode: {mode_name}"));
        self.rebuild_annotations();
    }

    pub fn toggle_file_list(&mut self) {
        self.show_file_list = !self.show_file_list;
        if !self.show_file_list
            && matches!(
                self.focused_panel,
                FocusedPanel::FileList | FocusedPanel::Comments
            )
        {
            self.focused_panel = FocusedPanel::Diff;
        }
        let status = if self.show_file_list {
            "visible"
        } else {
            "hidden"
        };
        self.set_message(format!("File list: {status}"));
    }

    /// Toggle single-file view. When on, the diff panel renders only the
    /// currently focused file instead of the full continuous-scroll
    /// concatenation. Annotations, navigation, and export work the same
    /// way on the rendered subset.
    pub fn toggle_single_file_view(&mut self) {
        self.is_single_file_view = !self.is_single_file_view;
        // `calculate_file_scroll_offset` changes meaning across modes
        // (single-file stops at review-comments header, all-files
        // accumulates), so re-snap the viewport to the current file.
        let start = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
        self.diff_state.scroll_offset = start;
        self.diff_state.cursor_line = start;
        let status = if self.is_single_file_view {
            "single file"
        } else {
            "all files"
        };
        self.set_message(format!("View: {status}"));
        self.rebuild_annotations();
    }

    /// Re-establishes `diff_files` order. **The one place it is established**,
    /// which is why every reorder path calls it and why grouping hooks in here
    /// rather than at each of the seventeen call sites.
    ///
    /// Grouped, the order is the grouping's: the commit-message pseudo-file,
    /// then groups in reading order, then each group's files in the engine's
    /// within-group order. Ungrouped, it is today's directory sort, unchanged.
    pub(in crate::app) fn sort_files_by_directory(&mut self, reset_position: bool) {
        // Both the line-count cache and the gap maps are keyed by `file_idx`,
        // a position in `diff_files`, so reordering invalidates all of them.
        self.clear_expanded_gaps();

        let current_path = if !reset_position {
            self.current_file_path().cloned()
        } else {
            None
        };

        if self.grouping_enabled {
            self.order_files_by_group();
        } else {
            self.order_files_by_directory();
        }

        if let Some(path) = current_path
            && let Some(idx) = self
                .diff_files
                .iter()
                .position(|f| f.display_path() == &path)
        {
            self.jump_to_file(idx);
            return;
        }

        // Start at the overview position (review comments header)
        // so the diff title shows total stats on launch.
        self.diff_state.cursor_line = 0;
        self.diff_state.scroll_offset = 0;
        self.diff_state.current_file_idx = 0;
    }

    pub(in crate::app) fn order_files_by_directory(&mut self) {
        use std::collections::BTreeMap;

        let mut dir_map: BTreeMap<Vec<String>, Vec<DiffFile>> = BTreeMap::new();
        let mut commit_msg_files: Vec<DiffFile> = Vec::new();

        for file in self.diff_files.drain(..) {
            if file.is_commit_message {
                commit_msg_files.push(file);
                continue;
            }
            // Key on the parent's components, not on the joined parent
            // string. `build_visible_items` emits a directory header only the
            // first time it sees a directory, so every subtree has to come out
            // of here contiguous. A flat string sort breaks that whenever a
            // sibling shares a prefix, because '.' and '-' sort before '/':
            // "ChronoStream.BuildTests" lands between "ChronoStream" and
            // "ChronoStream/Subdir", and the orphaned half then renders
            // indented under the sibling.
            let dir: Vec<String> = file
                .display_path()
                .parent()
                .map(|parent| {
                    parent
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                        .collect()
                })
                .unwrap_or_default();

            dir_map.entry(dir).or_default().push(file);
        }

        self.diff_files.extend(commit_msg_files);
        for (_dir, files) in dir_map {
            self.diff_files.extend(files);
        }
    }

    /// Opens everything in **both** sidebars: every directory row of the
    /// ungrouped tree, and every group row when there is a grouping.
    ///
    /// Both, not just the one on screen, because this is the fresh-start seed —
    /// startup and a load that answers a new target — and after `<leader>g` the
    /// other sidebar is one keypress away. Seeding only the active one would
    /// hand the first toggle a wholly collapsed tree that no reader collapsed.
    /// The sets are disjoint by construction, so filling both costs nothing but
    /// the walk.
    pub fn expand_all_dirs(&mut self) {
        self.expand_all_dir_keys();
        self.expand_all_group_keys();
        self.ensure_valid_tree_selection();
    }

    /// The ungrouped tree's keys, seeded from the tree mode's directory rows.
    fn expand_all_dir_keys(&mut self) {
        let layout = self.tree_layout();
        self.expanded_dirs = self
            .diff_files
            .iter()
            .flat_map(|file| layout.dir_rows(file.display_path()))
            .map(|row| row.path)
            .collect();
    }

    /// The grouped sidebar's keys: one per group, and nothing else. Empty when
    /// the session has no grouping to key on.
    pub(in crate::app) fn expand_all_group_keys(&mut self) {
        self.expanded_groups = self
            .grouping
            .as_ref()
            .map(|grouping| grouping.groups().iter().map(Self::group_row_key).collect())
            .unwrap_or_default();
    }

    /// The sidebar keys after a reload of the *same* review, as opposed to a
    /// load that answers a new target.
    ///
    /// Grouped, the keys are kept: incremental assignment leaves every group a
    /// reader had open exactly where it was, and re-expanding everything would
    /// throw away the collapsed overview they had arranged for the sake of one
    /// file appearing. Keys naming a group that no longer exists are dropped
    /// here rather than remapped — `expanded_dirs` is keyed on the opaque
    /// group id precisely so that a group which did not survive is simply gone
    /// (`docs/REGROUPING_STATE.md`). A group opened by incremental assignment
    /// starts collapsed, like the unranked group at the bottom of the list
    /// that it is.
    ///
    /// Ungrouped this re-seeds the directory keys as [`App::expand_all_dirs`]
    /// does: directory keys are path-derived, so there is no identity to
    /// preserve and nothing to lose. Either way the *other* sidebar's keys are
    /// left alone — a reload of the same review is no reason to rearrange a
    /// view the reader is not even looking at.
    pub(in crate::app) fn reseed_expanded_dirs(&mut self) {
        match self.active_grouping() {
            Some(grouping) => {
                let live: HashSet<String> =
                    grouping.groups().iter().map(Self::group_row_key).collect();
                self.expanded_groups.retain(|key| live.contains(key));
            }
            None => self.expand_all_dir_keys(),
        }
        self.ensure_valid_tree_selection();
    }

    /// Collapses every row of the sidebar on screen. The other sidebar's keys
    /// stay: `O` collapses what the reader can see, and a `:regroup` landing
    /// (`src/app/regroup.rs`) says nothing about the directory tree.
    pub fn collapse_all_dirs(&mut self) {
        match self.active_grouping() {
            Some(_) => self.expanded_groups.clear(),
            None => self.expanded_dirs.clear(),
        }
        self.ensure_valid_tree_selection();
    }

    /// Toggles the row `dir_path` keys: a group id while grouping is on, a
    /// directory path while it is off. The two never reach the same set.
    pub fn toggle_directory(&mut self, dir_path: &str) {
        let grouped = self.active_grouping().is_some();
        let keys = if grouped {
            &mut self.expanded_groups
        } else {
            &mut self.expanded_dirs
        };
        if keys.remove(dir_path) {
            if let Some(tree_idx) = self
                .build_visible_items()
                .iter()
                .position(|item| match item {
                    FileTreeItem::Directory { path, .. } => path == dir_path,
                    FileTreeItem::Group { id, .. } => id == dir_path,
                    FileTreeItem::File { .. } => false,
                })
            {
                self.file_list_state.select(tree_idx);
            } else {
                self.ensure_valid_tree_selection();
            }
        } else if grouped {
            self.expanded_groups.insert(dir_path.to_string());
        } else {
            self.expanded_dirs.insert(dir_path.to_string());
        }
    }

    /// Expand whatever hides `file_idx` and put the tree cursor on its row.
    ///
    /// Under grouping the file's group id is the only key that reveals
    /// anything — there are no in-group directory rows — and ungrouped it is
    /// every directory row on the path. Every reveal site goes through here,
    /// so the two key spaces cannot drift apart.
    pub(in crate::app) fn reveal_file(&mut self, file_idx: usize) {
        let Some(file) = self.diff_files.get(file_idx) else {
            return;
        };
        let path = file.display_path().clone();
        if self.active_grouping().is_some() {
            if let Some(key) = self.group_key_of_file(&path) {
                self.expanded_groups.insert(key);
            }
        } else {
            for row in self.tree_layout().dir_rows(&path) {
                self.expanded_dirs.insert(row.path);
            }
        }

        if let Some(tree_idx) = self.file_idx_to_tree_idx(file_idx) {
            self.file_list_state.select(tree_idx);
        }
    }

    pub(in crate::app) fn ensure_valid_tree_selection(&mut self) {
        let visible_items = self.build_visible_items();
        if visible_items.is_empty() {
            self.file_list_state.select(0);
            return;
        }

        let current_file_idx = self.diff_state.current_file_idx;
        let file_visible = visible_items.iter().any(|item| {
            matches!(item, FileTreeItem::File { file_idx, .. } if *file_idx == current_file_idx)
        });

        if file_visible {
            if let Some(tree_idx) = self.file_idx_to_tree_idx(current_file_idx) {
                self.file_list_state.select(tree_idx);
            }
        } else {
            if let Some(file) = self.diff_files.get(current_file_idx) {
                // Under grouping the only thing that can have hidden the file
                // is its group row; ungrouped it is the innermost directory
                // row still on screen. Same discriminant as `reveal_file`, so
                // the two cannot disagree about which sidebar is on screen.
                if self.active_grouping().is_some() {
                    if let Some(key) = self.group_key_of_file(file.display_path()) {
                        for (tree_idx, item) in visible_items.iter().enumerate() {
                            if let FileTreeItem::Group { id, .. } = item
                                && *id == key
                            {
                                self.file_list_state.select(tree_idx);
                                return;
                            }
                        }
                    }
                } else {
                    let dir_rows = self.tree_layout().dir_rows(file.display_path());
                    for row in dir_rows.iter().rev() {
                        for (tree_idx, item) in visible_items.iter().enumerate() {
                            if let FileTreeItem::Directory { path, .. } = item
                                && *path == row.path
                            {
                                self.file_list_state.select(tree_idx);
                                return;
                            }
                        }
                    }
                }
            }
            self.file_list_state.select(0);
        }
    }

    pub fn build_visible_items(&self) -> Vec<FileTreeItem> {
        match self.active_grouping() {
            Some(grouping) => self.build_grouped_items(grouping),
            None => self.build_directory_items(),
        }
    }

    /// The grouped sidebar: the commit-message pseudo-file pinned above
    /// everything, then a collapsible row per group with the group's files
    /// listed directly beneath it (`docs/SIDEBAR_MODEL.md`).
    ///
    /// There are no directory rows inside a group. A group's members are
    /// scattered across directories by construction, so directory chrome cost
    /// 130 rows on the fixture to say what the full path already says, and it
    /// cut across the within-group ordering (`gd-26r.31`). The tree mode
    /// governs the ungrouped tree only: grouping renders as though `Flat` were
    /// set, whatever the config says, and leaves the mode alone.
    fn build_grouped_items(&self, grouping: &crate::grouping::Grouping) -> Vec<FileTreeItem> {
        let mut items = Vec::new();

        // Outside the partition, so it is emitted before the group loop and
        // is never hidden by a collapsed group (`docs/TOTAL_COVERAGE.md`).
        for (file_idx, file) in self.diff_files.iter().enumerate() {
            if file.is_commit_message && self.file_passes_filter(file) {
                items.push(FileTreeItem::File {
                    file_idx,
                    label: FileTreeMode::Flat.file_label(file.display_path()),
                    depth: 0,
                });
            }
        }

        let by_path = self.file_indices_by_path();
        debug_assert!(
            self.group_runs_are_contiguous(grouping),
            "diff_files must be contiguous by group: the sidebar reads each \
             group as one run and a split group loses rows"
        );

        // One pass over the assignments rather than a scan of them per group
        // and another per file: this runs on every frame, and the scans are
        // each `groups × files`. Assignments are in file reading order, which
        // is the order `Grouping::files_in` would have yielded.
        let mut members_by_group: HashMap<&crate::grouping::GroupId, Vec<usize>> = HashMap::new();
        let mut assigned: HashSet<&Path> = HashSet::new();
        for assignment in grouping.assignments() {
            let path = assignment.path.as_path();
            assigned.insert(path);
            let Some(&file_idx) = by_path.get(path) else {
                continue;
            };
            if !self.file_passes_filter(&self.diff_files[file_idx]) {
                continue;
            }
            members_by_group
                .entry(&assignment.group_id)
                .or_default()
                .push(file_idx);
        }

        for group in grouping.groups() {
            // A group whose every file the filter hides disappears with them,
            // exactly as a directory does.
            let Some(members) = members_by_group.get(&group.id) else {
                continue;
            };

            let group_key = Self::group_row_key(group);
            let expanded = self.expanded_groups.contains(&group_key);
            items.push(FileTreeItem::Group {
                id: group_key,
                label: group.name.clone(),
                reviewed: members
                    .iter()
                    .filter(|file_idx| {
                        self.session
                            .is_file_reviewed(self.diff_files[**file_idx].display_path())
                    })
                    .count(),
                total: members.len(),
                expanded,
            });
            if !expanded {
                continue;
            }

            // Every member at depth 1, in the engine's within-group order,
            // labelled with its full relative path. The group row is the only
            // thing that collapses, so there is nothing here that can hide a
            // file the way a collapsed in-group directory row could.
            for &file_idx in members {
                items.push(FileTreeItem::File {
                    file_idx,
                    label: FileTreeMode::Flat.file_label(self.diff_files[file_idx].display_path()),
                    depth: 1,
                });
            }
        }

        // A file the grouping does not mention still gets a row, at top level
        // after the groups — the same place `order_files_by_group` sorts it.
        // The partition is total today, so this emits nothing; it is here
        // because losing a file from the review is the worse failure, and a
        // defence the renderer cancels is no defence.
        for (file_idx, file) in self.diff_files.iter().enumerate() {
            if file.is_commit_message
                || assigned.contains(file.display_path().as_path())
                || !self.file_passes_filter(file)
            {
                continue;
            }
            items.push(FileTreeItem::File {
                file_idx,
                label: FileTreeMode::Flat.file_label(file.display_path()),
                depth: 0,
            });
        }

        items
    }

    fn file_indices_by_path(&self) -> HashMap<&Path, usize> {
        self.diff_files
            .iter()
            .enumerate()
            .map(|(file_idx, file)| (file.display_path().as_path(), file_idx))
            .collect()
    }

    /// Whether every group occupies one unbroken run of `diff_files`, starting
    /// after the commit-message rows the hoist keeps at the front.
    fn group_runs_are_contiguous(&self, grouping: &crate::grouping::Grouping) -> bool {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut current: Option<&str> = None;
        for file in &self.diff_files {
            if file.is_commit_message {
                continue;
            }
            let Some(id) = grouping.group_of(file.display_path()) else {
                // An ungrouped file breaks the run it sits in: the group cannot
                // resume after it without the rows being split.
                current = None;
                continue;
            };
            if current == Some(id.as_str()) {
                continue;
            }
            if !seen.insert(id.as_str()) {
                return false;
            }
            current = Some(id.as_str());
        }
        true
    }

    /// The plain directory tree, unchanged: one global `seen_dirs`, which is
    /// sound because `order_files_by_directory` leaves `diff_files` contiguous
    /// by directory.
    fn build_directory_items(&self) -> Vec<FileTreeItem> {
        let layout = self.tree_layout();
        let mut items = Vec::new();
        let mut seen_dirs: HashSet<String> = HashSet::new();

        for (file_idx, file) in self.diff_files.iter().enumerate() {
            // Filtered-out files contribute no row and no ancestor
            // directories, so a directory whose children are all hidden
            // disappears with them.
            if !self.file_passes_filter(file) {
                continue;
            }
            let path = file.display_path();
            let dir_rows = layout.dir_rows(path);

            let mut visible = true;
            for (depth, row) in dir_rows.iter().enumerate() {
                if visible && seen_dirs.insert(row.path.clone()) {
                    items.push(FileTreeItem::Directory {
                        path: row.path.clone(),
                        label: row.label.clone(),
                        depth,
                        expanded: self.expanded_dirs.contains(&row.path),
                    });
                }

                if !self.expanded_dirs.contains(&row.path) {
                    visible = false;
                }
            }

            if visible {
                items.push(FileTreeItem::File {
                    file_idx,
                    label: self.file_tree_mode.file_label(path),
                    depth: dir_rows.len(),
                });
            }
        }

        items
    }

    pub fn get_selected_tree_item(&self) -> Option<FileTreeItem> {
        let visible_items = self.build_visible_items();
        let selected_idx = self.file_list_state.selected();
        visible_items.get(selected_idx).cloned()
    }
}
