use super::*;

impl App {
    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Info, Some(MESSAGE_TTL_INFO));
    }

    pub fn set_warning(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Warning, Some(MESSAGE_TTL_WARNING));
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Error, None);
    }

    /// Warning that stays until something else overwrites it. Used for state-tied
    /// messages like the dirty-quit prompt where the visual must outlive any TTL.
    pub fn set_sticky_warning(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Warning, None);
    }

    /// Startup collects warnings from independent sources — a config parse, a
    /// theme, an unknown key, the sparse-checkout backend, the refine arm — and
    /// several can fire on one run. They are shown one at a time in the single
    /// message slot, each for its own TTL, tagged `(i/N)` so a reader knows more
    /// are coming and which one they are on. Nothing is capped or dropped: the
    /// queue drains in arrival order, so the refine warning appended last is
    /// seen even when a config warning arrived first.
    ///
    /// Rejected, for a slot that is one right-aligned span in a fixed-height
    /// status bar:
    /// - **Joining them into one line.** Two full-sentence warnings already
    ///   exceed a normal width, and the span is clipped rather than wrapped, so
    ///   the later warning is lost again — the bug, restated.
    /// - **A stacked block.** Rows for a transient message have to come out of
    ///   the diff, and eight warnings would take the pane.
    /// - **First plus `(+N more)`.** The N are never readable anywhere.
    /// - **A scrollable `:messages` history.** The right home for a long tail,
    ///   but a new surface, and this queue needs no cap without it.
    ///
    /// A message from anything the human then does replaces the queue rather
    /// than waiting behind it: a reply to a keypress outranks startup noise.
    pub fn set_startup_warnings(&mut self, warnings: Vec<String>) {
        let total = warnings.len();
        let mut queued: VecDeque<String> = warnings
            .into_iter()
            .enumerate()
            .map(|(index, warning)| match total {
                1 => warning,
                _ => format!("{warning} ({}/{total})", index + 1),
            })
            .collect();
        let Some(first) = queued.pop_front() else {
            return;
        };
        self.set_warning(first);
        self.queued_warnings = queued;
    }

    fn set_message_inner(
        &mut self,
        msg: impl Into<String>,
        message_type: MessageType,
        ttl: Option<Duration>,
    ) {
        self.queued_warnings.clear();
        self.write_message(msg, message_type, ttl);
    }

    fn write_message(
        &mut self,
        msg: impl Into<String>,
        message_type: MessageType,
        ttl: Option<Duration>,
    ) {
        if self.input_mode == InputMode::MessageDetails {
            if message_type == MessageType::Error {
                self.help_state.scroll_offset = 0;
            } else {
                self.input_mode = self.overlay_return_mode;
            }
        }
        self.message = Some(Message {
            content: msg.into(),
            message_type,
            expires_at: ttl.map(|d| Instant::now() + d),
        });
    }

    /// Returns `true` if the slot changed — a message expired, or the next
    /// queued startup warning took its place — so the main loop can schedule a
    /// redraw.
    pub fn clear_expired_message(&mut self) -> bool {
        let expired = self
            .message
            .as_ref()
            .and_then(|m| m.expires_at)
            .is_some_and(|t| Instant::now() >= t);
        if expired {
            self.message = None;
            if let Some(next) = self.queued_warnings.pop_front() {
                self.write_message(next, MessageType::Warning, Some(MESSAGE_TTL_WARNING));
            }
        }
        expired
    }

    pub fn enter_command_mode(&mut self) {
        self.command_return_mode = self.input_mode;
        self.input_mode = InputMode::Command;
        self.command_buffer.clear();
        self.command_completion = None;
    }

    pub fn exit_command_mode(&mut self) {
        self.input_mode = self.command_return_mode;
        self.command_buffer.clear();
        self.command_completion = None;
    }

    pub fn enter_search_mode(&mut self) {
        self.search_return_mode = self.input_mode;
        self.input_mode = InputMode::Search;
        self.search_buffer.clear();
    }

    pub fn exit_search_mode(&mut self) {
        self.input_mode = self.search_return_mode;
        self.search_buffer.clear();
    }

    pub fn searching_help(&self) -> bool {
        self.input_mode == InputMode::Search && self.search_return_mode == InputMode::Help
    }

    pub fn open_message_details(&mut self) {
        if self
            .message
            .as_ref()
            .is_some_and(|message| message.message_type == MessageType::Error)
        {
            self.overlay_return_mode = self.input_mode;
            self.input_mode = InputMode::MessageDetails;
            self.help_state.scroll_offset = 0;
        } else {
            self.set_message("No current error");
        }
    }

    pub fn toggle_help(&mut self) {
        match self.input_mode {
            InputMode::Help | InputMode::MessageDetails => {
                self.input_mode = self.overlay_return_mode;
            }
            _ => {
                self.overlay_return_mode = self.input_mode;
                self.input_mode = InputMode::Help;
                self.help_state.scroll_offset = 0;
                self.help_state.horizontal_offset = 0;
            }
        }
    }

    pub fn help_scroll_down(&mut self, lines: usize) {
        let max_offset = self
            .help_state
            .total_lines
            .saturating_sub(self.help_state.viewport_height);
        self.help_state.scroll_offset = (self.help_state.scroll_offset + lines).min(max_offset);
    }

    pub fn help_scroll_up(&mut self, lines: usize) {
        self.help_state.scroll_offset = self.help_state.scroll_offset.saturating_sub(lines);
    }

    pub fn help_scroll_right(&mut self, columns: usize) {
        self.help_state.scroll_right(columns);
    }

    pub fn help_scroll_left(&mut self, columns: usize) {
        self.help_state.scroll_left(columns);
    }

    pub fn help_scroll_to_top(&mut self) {
        self.help_state.scroll_offset = 0;
    }

    pub fn help_scroll_to_bottom(&mut self) {
        let max_offset = self
            .help_state
            .total_lines
            .saturating_sub(self.help_state.viewport_height);
        self.help_state.scroll_offset = max_offset;
    }

    pub fn enter_summary_mode(&mut self) {
        self.input_mode = InputMode::Summary;
        self.summary_state.selected_comment = 0;
        self.summary_state.scroll_offset = 0;
        self.summary_state.comment_ranges.clear();
        self.summary_state.targets.clear();
        self.summary_state.selection_needs_scroll = true;
    }

    pub fn exit_summary_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.summary_state.comment_ranges.clear();
        self.summary_state.targets.clear();
        self.summary_state.selection_needs_scroll = false;
    }

    pub(crate) fn update_summary_layout(
        &mut self,
        comment_ranges: Vec<(usize, usize)>,
        targets: Vec<Option<SummaryCommentTarget>>,
        total_lines: usize,
        viewport_height: usize,
    ) {
        debug_assert_eq!(comment_ranges.len(), targets.len());
        let layout_changed = self.summary_state.comment_ranges != comment_ranges
            || self.summary_state.viewport_height != viewport_height;
        self.summary_state.comment_ranges = comment_ranges;
        self.summary_state.targets = targets;
        self.summary_state.total_lines = total_lines;
        self.summary_state.viewport_height = viewport_height;

        let comment_count = self.summary_state.comment_ranges.len();
        if comment_count == 0 {
            self.summary_state.selected_comment = 0;
            self.summary_state.scroll_offset = 0;
            self.summary_state.selection_needs_scroll = false;
            return;
        }

        let clamped_selection = self
            .summary_state
            .selected_comment
            .min(comment_count.saturating_sub(1));
        if clamped_selection != self.summary_state.selected_comment || layout_changed {
            self.summary_state.selection_needs_scroll = true;
        }
        self.summary_state.selected_comment = clamped_selection;

        if self.summary_state.selection_needs_scroll {
            self.ensure_summary_selection_visible();
        } else {
            let max_offset = total_lines.saturating_sub(viewport_height);
            self.summary_state.scroll_offset = self.summary_state.scroll_offset.min(max_offset);
        }
    }

    pub fn summary_select_down(&mut self, comments: usize) {
        let max_selection = self.summary_state.comment_ranges.len().saturating_sub(1);
        self.summary_state.selected_comment = self
            .summary_state
            .selected_comment
            .saturating_add(comments)
            .min(max_selection);
        self.summary_state.selection_needs_scroll = true;
        self.ensure_summary_selection_visible();
    }

    pub fn summary_select_up(&mut self, comments: usize) {
        self.summary_state.selected_comment =
            self.summary_state.selected_comment.saturating_sub(comments);
        self.summary_state.selection_needs_scroll = true;
        self.ensure_summary_selection_visible();
    }

    fn ensure_summary_selection_visible(&mut self) {
        let Some(&(start, end)) = self
            .summary_state
            .comment_ranges
            .get(self.summary_state.selected_comment)
        else {
            return;
        };
        let viewport_height = self.summary_state.viewport_height;
        if viewport_height == 0 {
            return;
        }

        let selected_height = end.saturating_sub(start);
        let viewport_end = self
            .summary_state
            .scroll_offset
            .saturating_add(viewport_height);
        if selected_height >= viewport_height || start < self.summary_state.scroll_offset {
            self.summary_state.scroll_offset = start;
        } else if end > viewport_end {
            self.summary_state.scroll_offset = end.saturating_sub(viewport_height);
        }

        let max_offset = self
            .summary_state
            .total_lines
            .saturating_sub(viewport_height);
        self.summary_state.scroll_offset = self.summary_state.scroll_offset.min(max_offset);
        self.summary_state.selection_needs_scroll = false;
    }

    pub fn summary_scroll_down(&mut self, lines: usize) {
        let max_offset = self
            .summary_state
            .total_lines
            .saturating_sub(self.summary_state.viewport_height);
        self.summary_state.scroll_offset =
            (self.summary_state.scroll_offset + lines).min(max_offset);
        self.sync_summary_selection_to_viewport(true);
    }

    pub fn summary_scroll_up(&mut self, lines: usize) {
        self.summary_state.scroll_offset = self.summary_state.scroll_offset.saturating_sub(lines);
        self.sync_summary_selection_to_viewport(false);
    }

    pub fn summary_select_first(&mut self) {
        self.summary_state.selected_comment = 0;
        self.summary_state.selection_needs_scroll = true;
        self.ensure_summary_selection_visible();
    }

    pub fn summary_select_last(&mut self) {
        self.summary_state.selected_comment =
            self.summary_state.comment_ranges.len().saturating_sub(1);
        self.summary_state.selection_needs_scroll = true;
        self.ensure_summary_selection_visible();
    }

    fn sync_summary_selection_to_viewport(&mut self, scrolling_down: bool) {
        let viewport_start = self.summary_state.scroll_offset;
        let viewport_end = viewport_start.saturating_add(self.summary_state.viewport_height);
        let is_visible =
            |(start, end): &(usize, usize)| *end > viewport_start && *start < viewport_end;
        let current_selection = self.summary_state.selected_comment;
        let current_is_visible = self
            .summary_state
            .comment_ranges
            .get(current_selection)
            .is_some_and(is_visible);
        let visible_selection = if current_is_visible {
            Some(current_selection)
        } else if scrolling_down {
            self.summary_state
                .comment_ranges
                .iter()
                .enumerate()
                .skip(current_selection.saturating_add(1))
                .find(|(_, range)| is_visible(range))
                .map(|(idx, _)| idx)
        } else {
            self.summary_state
                .comment_ranges
                .iter()
                .enumerate()
                .take(current_selection)
                .rev()
                .find(|(_, range)| is_visible(range))
                .map(|(idx, _)| idx)
        }
        .or_else(|| {
            self.summary_state
                .comment_ranges
                .iter()
                .position(is_visible)
        });

        if let Some(selected_comment) = visible_selection {
            self.summary_state.selected_comment = selected_comment;
        } else if let Some((selected_comment, &(start, _))) = self
            .summary_state
            .comment_ranges
            .iter()
            .enumerate()
            .find(|(_, (start, _))| *start >= viewport_start)
        {
            self.summary_state.selected_comment = selected_comment;
            self.summary_state.scroll_offset = start;
        } else if let Some((selected_comment, &(_, end))) = self
            .summary_state
            .comment_ranges
            .iter()
            .enumerate()
            .next_back()
        {
            self.summary_state.selected_comment = selected_comment;
            self.summary_state.scroll_offset =
                end.saturating_sub(self.summary_state.viewport_height);
        }
        self.summary_state.selection_needs_scroll = false;
    }

    pub fn enter_confirm_mode(&mut self, action: ConfirmAction) {
        self.input_mode = InputMode::Confirm;
        self.pending_confirm = Some(action);
    }

    pub fn exit_confirm_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.pending_confirm = None;
    }
}
