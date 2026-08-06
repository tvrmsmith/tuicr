#!/usr/bin/env bash
set -e -u -o pipefail

# Configuration - override via environment variables
TUICR_PANE_POSITION="${TUICR_PANE_POSITION:-top}"    # top or bottom
TUICR_PANE_SIZE="${TUICR_PANE_SIZE:-80}"              # percentage of screen

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
  echo -e "${GREEN}[tuicr]${NC} $*"
}

log_warn() {
  echo -e "${YELLOW}[tuicr]${NC} $*"
}

log_error() {
  echo -e "${RED}[tuicr]${NC} $*"
}

usage() {
  cat << EOF
Usage: $(basename "$0") [directory]

Launch tuicr in a tmux split pane to review git changes.

Arguments:
  directory    Git repository directory to review (default: current directory)

Environment variables:
  TUICR_PANE_POSITION   Position of tuicr pane: top or bottom (default: top)
  TUICR_PANE_SIZE       Size of pane as percentage (default: 80)

Examples:
  $(basename "$0")                    # Review changes in current directory
  $(basename "$0") ~/project          # Review changes in ~/project
  TUICR_PANE_SIZE=70 $(basename "$0") # Use 70% of screen
EOF
}

check_tmux() {
  if [[ -z "${TMUX:-}" ]]; then
    return 1
  fi
  return 0
}

check_tuicr() {
  if ! command -v tuicr &> /dev/null; then
    log_error "tuicr not found. Install it first."
    return 1
  fi
  return 0
}

check_tuicr_stdout_support() {
  # Check if tuicr supports --stdout flag
  tuicr --help 2>&1 | grep -q -- '--stdout'
}

check_git_repo() {
  local dir="$1"
  if ! git -C "$dir" rev-parse --git-dir &> /dev/null; then
    log_error "Not a git repository: $dir"
    return 1
  fi
  return 0
}

check_lsof() {
  if ! command -v lsof &> /dev/null; then
    log_error "lsof not found on PATH"
    return 1
  fi
  return 0
}

# True only when a tuicr is already reviewing *this* repository. A machine-wide
# pane scan reports success because of a tuicr in some unrelated repo and sends
# the user hunting for a pane that does not exist here. The spawned pane runs
# tuicr with the repository as its working directory, so that is what is
# matched — the same check the zellij and Orca wrappers make, so all three
# launch paths answer the question the same way.
check_tuicr_running() {
  local target_dir="$1"
  local pid cwd

  while read -r pid; do
    [[ -n "$pid" ]] || continue
    cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p')
    if [[ -z "$cwd" ]]; then
      log_warn "Cannot read the working directory of running tuicr $pid; assuming it is elsewhere"
      continue
    fi
    if [[ "$cwd" == "$target_dir" ]]; then
      return 0
    fi
  done < <(pgrep -x tuicr 2>/dev/null)

  return 1
}

launch_tuicr_pane() {
  local target_dir="$1"

  # Get window height and calculate lines (using -l instead of -p to avoid "size missing" error)
  local window_height
  window_height=$(tmux display-message -p '#{window_height}')
  local pane_lines=$(( window_height * TUICR_PANE_SIZE / 100 ))

  # Build the split-window command
  local split_args=()

  # Determine split direction based on position
  if [[ "$TUICR_PANE_POSITION" == "top" ]]; then
    split_args+=(-b)  # Create pane above
  fi
  # For bottom, no -b flag needed (default)

  # Set pane size in lines (not percentage, to work without TTY)
  split_args+=(-l "$pane_lines")

  # Change to target directory
  split_args+=(-c "$target_dir")

  log_info "Launching tuicr in $TUICR_PANE_POSITION pane (${pane_lines} lines, ${TUICR_PANE_SIZE}%)"
  log_info "Directory: $target_dir"

  # Create unique channel for wait-for
  local wait_channel="tuicr-$$"

  # `tmux wait-for` carries no payload, so tuicr's exit status has to travel out
  # of the pane some other way. Removed from a trap: every path out of here can
  # leave it behind otherwise, including a split-window that never spawns.
  status_file=$(mktemp /tmp/tuicr-status.XXXXXX)
  trap 'rm -f "$status_file"' EXIT

  # Check if --stdout is supported and set up output capture
  local output_file=""
  local tuicr_cmd="tuicr"
  local use_stdout=false

  if check_tuicr_stdout_support; then
    output_file=$(mktemp /tmp/tuicr-output.XXXXXX)
    local quoted_output
    printf -v quoted_output '%q' "$output_file"
    tuicr_cmd="tuicr --stdout > $quoted_output"
    use_stdout=true
    log_info "Using --stdout mode (output will be captured)"
  else
    log_warn "tuicr --stdout not supported, output will be copied to clipboard"
  fi

  # Every path interpolated into the pane's shell command goes through `%q`
  # first, so one holding a quote or a space cannot break out of the command it
  # belongs to and run as shell in the new pane.
  local quoted_dir quoted_status quoted_channel
  printf -v quoted_dir '%q' "$target_dir"
  printf -v quoted_status '%q' "$status_file"
  printf -v quoted_channel '%q' "$wait_channel"

  # Create the split pane with tuicr, signal when done
  # Use -d to not switch, -P to print pane info so we can capture the ID
  local new_pane_id
  new_pane_id=$(tmux split-window -d -P -F '#{pane_id}' "${split_args[@]}" \
    "cd $quoted_dir && $tuicr_cmd; echo \$? > $quoted_status; tmux wait-for -S $quoted_channel")

  # Switch focus to the new tuicr pane
  tmux select-pane -t "$new_pane_id"

  log_info "tuicr is running in pane $new_pane_id"
  log_info "Waiting for tuicr to exit..."

  # Block until tuicr exits
  tmux wait-for "$wait_channel"

  # The channel is signalled however tuicr ended, so the status is what says
  # whether it ended well — the same answer the zellij and Orca wrappers return.
  local status
  status=$(cat "$status_file" 2>/dev/null || true)
  [[ "$status" =~ ^[0-9]+$ ]] || status=1

  if [[ "$status" -eq 0 ]]; then
    log_info "tuicr finished"
  else
    log_error "tuicr exited with status $status"
  fi

  # Output captured instructions if --stdout was used
  if [[ "$use_stdout" == true ]] && [[ -f "$output_file" ]]; then
    if [[ -s "$output_file" ]]; then
      echo ""
      echo "=== TUICR INSTRUCTIONS ==="
      cat "$output_file"
      echo "=== END TUICR INSTRUCTIONS ==="
    else
      log_info "No instructions exported from tuicr"
      log_info "If you exported to clipboard, paste the instructions here"
    fi
    rm -f "$output_file"
  else
    log_info "If you exported instructions, they are in your clipboard - paste them here"
  fi

  return "$status"
}

main() {
  # Handle help
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
  fi

  # Check for tuicr
  if ! check_tuicr; then
    exit 1
  fi

  if ! check_lsof; then
    exit 1
  fi

  # Determine target directory. Physical path: lsof reports a process's working
  # directory with symlinks resolved, so a logical `pwd` through a symlinked
  # checkout would never compare equal in check_tuicr_running.
  local target_dir="${1:-.}"
  target_dir=$(cd "$target_dir" && pwd -P)

  # Verify it's a git repo
  if ! check_git_repo "$target_dir"; then
    exit 1
  fi

  # Check if we're in tmux
  if ! check_tmux; then
    log_error "Not running inside tmux!"
    echo ""
    echo "To use tuicr with your coding agent, run that agent inside tmux."
    echo ""
    echo "1. Exit the current agent session."
    echo ""
    echo "2. Restart the agent inside tmux."
    echo ""
    echo "3. Then run /tuicr again."
    exit 1
  fi

  # Check if tuicr is already reviewing this repository
  if check_tuicr_running "$target_dir"; then
    log_error "tuicr is already reviewing $target_dir"
    echo ""
    echo "Switch to its pane with Ctrl-b + arrow keys, or quit it there and run"
    echo "/tuicr again. To review a different repository, pass its directory:"
    echo ""
    echo "  $(basename "$0") <directory>"
    exit 1
  fi

  # Launch tuicr in a split pane, and exit with what tuicr exited with
  launch_tuicr_pane "$target_dir"
}

main "$@"
