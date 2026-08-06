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
# second arm.
slug="${TUICR_REFINE_MODEL_SLUG:-opus}"

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
