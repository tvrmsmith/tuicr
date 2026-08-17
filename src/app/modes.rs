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

    pub fn enter_confirm_mode(&mut self, action: ConfirmAction) {
        self.input_mode = InputMode::Confirm;
        self.pending_confirm = Some(action);
    }

    pub fn exit_confirm_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.pending_confirm = None;
    }
}
