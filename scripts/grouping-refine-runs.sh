#!/usr/bin/env bash
# Record agent-CLI runs of the grouping refine pass (`gd-26r.11`).
#
# The refine pass is one agent-CLI call. It is nondeterministic and costs money,
# so no test makes one. This script is the middle of the prototype: it sends
# each emitted prompt N times per shape and records the CLI's own JSON envelope
# — answer, token counts, wall clock — for `refine_report` to replay and score.
#
#   cargo test --test grouping_prototype -- --ignored --nocapture emit_refine_prompts
#   scripts/grouping-refine-runs.sh [runs]
#   cargo test --test grouping_prototype -- --ignored --nocapture refine_report
#
# Runs against the checked-in orca fixture land in tests/fixtures/grouping/refine/
# and are committed as evidence. Runs against the private fixture carry its paths
# in both prompt and answer, so they land beside it under $TUICR_GROUPING_FIXTURES
# and are never committed.
#
# Shelling out to an agent CLI from Rust is `gd-26r.13`, still open and
# unclaimed. This lives in a shell script on purpose: the prototype needs the
# mechanism but must not quietly decide it.

set -euo pipefail
# An empty prompts directory must not send a literal glob to the CLI as a
# filename.
shopt -s nullglob

# Ten per shape: every number in docs/GROUPING_PASSES.md is a property of a
# ten-run corpus, so the documented re-record path has to reproduce one.
runs="${1:-10}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prompts="$repo/target/grouping-refine/prompts"
fixture_dir="${TUICR_GROUPING_FIXTURES:-$HOME/.local/share/tuicr-fixtures}"
# Pinned to the exact model id, not the floating `opus` alias: the harness
# filters the published arm on the model name the envelope records, so a
# re-record under the alias would silently answer as whatever the alias points
# at that week and be filed as the published arm anyway. The check below is what
# catches that; the pin is what stops it happening. Set TUICR_REFINE_MODEL to
# record a second arm; runs are reported per model.
model="${TUICR_REFINE_MODEL:-claude-opus-5}"
# The file-name segment, which is not the model id: the committed corpus is
# named `<shape>-opus-NN.json`, and a re-record has to land on those exact names
# or it writes a parallel corpus beside the one every published number rests on
# instead of resuming it. Set alongside TUICR_REFINE_MODEL when recording a
# second arm — and the two have to move together. Setting only the model leaves
# every target named for the published arm, so all ten slots are already on disk
# and the run reports a re-record that never sent a call; setting only the slug
# files the published model's answers under another arm's name. Either way the
# corpus stops meaning what its file names say.
if [[ -n "${TUICR_REFINE_MODEL:-}" && -z "${TUICR_REFINE_MODEL_SLUG:-}" ]] \
  || [[ -z "${TUICR_REFINE_MODEL:-}" && -n "${TUICR_REFINE_MODEL_SLUG:-}" ]]; then
  echo "TUICR_REFINE_MODEL and TUICR_REFINE_MODEL_SLUG have to be set together:" >&2
  echo "the model decides who answers, the slug decides which corpus the answers" >&2
  echo "are filed under, and one without the other mislabels the recording." >&2
  exit 1
fi
slug="${TUICR_REFINE_MODEL_SLUG:-opus}"

# Reasoning effort. `gd-26r.24` measured that ~73% of a `full` call's output
# tokens are model reasoning rather than answer, which makes this the largest
# lever on both the bill and the wall clock — so it is an arm of the corpus, not
# a setting. Unset means the CLI default, which is what the published `opus` arm
# was recorded under. An effort arm is a different experiment from the published
# one and has to be filed under its own slug, exactly as a second model does,
# because the harness reports arms apart by that slug.
effort="${TUICR_REFINE_EFFORT:-}"
if [[ -n "$effort" && "$slug" == "opus" ]]; then
  echo "TUICR_REFINE_EFFORT changes what the call costs and how well it scores," >&2
  echo "so it needs its own TUICR_REFINE_MODEL_SLUG (e.g. opus-low). Filing it" >&2
  echo "under \`opus\` would mix it into the published arm." >&2
  exit 1
fi

# Which shapes to send, space-separated, default all four. An arm recorded to
# answer one question does not need every shape: `gd-26r.24`'s cost arms are
# about `full`, the shape `gd-26r.11` shipped, and sending the other three would
# triple the bill for rows no verdict reads. Named shapes must exist as prompts.
shapes="${TUICR_REFINE_SHAPES:-}"

if [[ ! -d "$prompts" ]]; then
  echo "no prompts in $prompts — run emit_refine_prompts first" >&2
  exit 1
fi

# Used to check that the answer was billed against the model we asked for.
if ! command -v jq &>/dev/null; then
  echo "jq not found on PATH; it is needed to verify each recorded run's model" >&2
  exit 1
fi

# Strip the CLI's project context: no CLAUDE.md, no settings, no tools. The
# refine call is a pure text transformation, and letting the agent read the
# repo it is grouping would measure something the shipped pass cannot do.
common=(--print --output-format json --setting-sources '' --tools '' --max-turns 1 --model "$model")
if [[ -n "$effort" ]]; then
  common+=(--effort "$effort")
fi

# `slots` is prompt x run, `sent` is calls actually made. Reporting failures
# against the slot count would understate the failure rate of a re-record that
# mostly hit the cache.
slots=0
sent=0
failed=0

for fixture_prompts in "$prompts"/*/; do
  fixture="$(basename "$fixture_prompts")"
  if [[ "$fixture" == orca-* ]]; then
    out="$repo/tests/fixtures/grouping/refine/$fixture"
  else
    out="$fixture_dir/refine/$fixture"
  fi
  mkdir -p "$out"

  for prompt in "$fixture_prompts"*.txt; do
    shape="$(basename "$prompt" .txt)"
    # A shape the filter excludes is not a slot: counting it would report a
    # complete arm as a mostly-cached re-record.
    if [[ -n "$shapes" && " $shapes " != *" $shape "* ]]; then
      continue
    fi
    for run in $(seq 1 "$runs"); do
      target="$out/$shape-$slug-$(printf '%02d' "$run").json"
      slots=$((slots + 1))
      if [[ -f "$target" ]]; then
        echo "have  $fixture/$shape run $run"
        continue
      fi
      sent=$((sent + 1))
      echo "call  $fixture/$shape run $run"
      # cd to a scratch directory so the CLI cannot pick up repo context. A
      # failed mktemp would leave `cd ""` succeeding in the repo being grouped
      # and record the contaminated answer as a valid run, so it is fatal.
      scratch="$(mktemp -d)" || exit 1
      if ! (cd "$scratch" && claude "${common[@]}" < "$prompt") > "$target.partial"; then
        echo "  failed; leaving $target.partial for inspection" >&2
        failed=$((failed + 1))
        rm -rf "$scratch"
        continue
      fi
      rm -rf "$scratch"
      # The harness filters the published arm on the envelope's `modelUsage`
      # key, so a CLI that resolved the request to some other model must not be
      # filed under this one — that mislabels the corpus every published number
      # rests on. A call may bill more than one model; the harness labels the run
      # by the one that wrote the answer, so this reads the same key rather than
      # every key, which would never compare equal.
      billed="$(jq -r '.modelUsage | to_entries | max_by(.value.outputTokens) | .key' \
        "$target.partial" 2>/dev/null || true)"
      if [[ "$billed" != "$model" ]]; then
        echo "  answered by \`${billed:-unknown}\`, not \`$model\`; leaving $target.partial" >&2
        failed=$((failed + 1))
        continue
      fi
      # The CLI exits 0 on its own failures — a turn limit, an execution error —
      # and still fills in `modelUsage`, so the model check above passes and an
      # envelope carrying no answer would be committed as evidence. The resume
      # guard then skips that slot forever and the hole surfaces much later as a
      # parse failure against a checked-in fixture.
      if ! jq -e '.is_error == false and .subtype == "success"' \
        "$target.partial" >/dev/null 2>&1; then
        echo "  the CLI reported an error for this run; leaving $target.partial" >&2
        failed=$((failed + 1))
        continue
      fi
      mv "$target.partial" "$target"
    done
  done
done

# nullglob turns an empty prompts tree into a loop that never runs, so a script
# that recorded nothing would otherwise print success and exit 0 on the one path
# that reproduces the corpus every published number rests on.
if (( slots == 0 )); then
  echo "no prompts under $prompts — run emit_refine_prompts first" >&2
  exit 1
fi

# A failed call prints and continues so the remaining calls still get made, but
# any hole in the corpus is fatal here: this is the path every published number
# rests on, and a partial re-record must not exit 0.
if (( failed > 0 )); then
  echo "$failed of $sent calls failed; the corpus is incomplete" >&2
  exit 1
fi

echo
echo "recorded. now: cargo test --test grouping_prototype -- --ignored --nocapture refine_report"
