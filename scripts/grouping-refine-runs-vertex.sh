#!/usr/bin/env bash
# Record direct Vertex AI runs of the grouping refine pass (`gd-26r.24`).
#
# The sibling script, `grouping-refine-runs.sh`, shells out to the Claude Code
# CLI. This one calls Vertex over HTTP instead, for two questions that transport
# script cannot ask:
#
#   1. What do Google's models cost and how fast are they on this prompt.
#   2. What does the CLI itself add? `claude-opus-5` answers on both transports,
#      so the same model recorded both ways separates the model's bill from the
#      CLI's preamble, cache write and process overhead.
#
# It writes one envelope per call into the same corpus directories, under the
# same `<shape>-<arm>-<nn>.json` names, so `refine_report` scores these arms
# beside the CLI ones. The envelope is *not* the CLI's shape — it carries the
# raw provider response plus the fields the CLI supplies and Vertex does not
# (wall clock, and a price table, since Vertex bills but does not tell you what
# it billed). `refine::RunRecord::parse` reads both shapes.
#
#   cargo test --test grouping_prototype -- --ignored --nocapture emit_refine_prompts
#   TUICR_REFINE_MODEL=gemini-3-flash-preview TUICR_REFINE_MODEL_SLUG=gemini-3-flash \
#     scripts/grouping-refine-runs-vertex.sh 10
#   cargo test --test grouping_prototype -- --ignored --nocapture refine_report

set -euo pipefail
shopt -s nullglob

runs="${1:-10}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prompts="$repo/target/grouping-refine/prompts"
fixture_dir="${TUICR_GROUPING_FIXTURES:-$HOME/.local/share/tuicr-fixtures}"

project="${TUICR_VERTEX_PROJECT:-}"
# `global` rather than a region on purpose: at the time of recording it was the
# only location serving `claude-opus-5` on this org's projects — `us-east5` and
# `us-central1` both answered 429 RESOURCE_EXHAUSTED — and Gemini 3 pricing
# carries a regional surcharge that `global` does not. A different location is a
# different price and a different queue, so it is recorded in the envelope.
location="${TUICR_VERTEX_LOCATION:-global}"
model="${TUICR_REFINE_MODEL:-}"
slug="${TUICR_REFINE_MODEL_SLUG:-}"
shapes="${TUICR_REFINE_SHAPES:-}"
# How much the model may think, in the units its own API uses: a `thinkingLevel`
# for Gemini 3, a `budget_tokens` for Anthropic, and the literal `off` for
# neither. `gd-26r.24` found reasoning is most of the bill, so this is the arm's
# defining setting and it goes in the slug exactly as `--effort` does on the CLI.
thinking="${TUICR_REFINE_THINKING:-}"

if [[ -z "$project" || -z "$model" || -z "$slug" ]]; then
  echo "set TUICR_VERTEX_PROJECT, TUICR_REFINE_MODEL and TUICR_REFINE_MODEL_SLUG." >&2
  echo "the model decides who answers, the slug decides which corpus the answers" >&2
  echo "are filed under, and one without the other mislabels the recording." >&2
  exit 1
fi

for tool in jq curl gcloud python3; do
  if ! command -v "$tool" &>/dev/null; then
    echo "$tool not found on PATH" >&2
    exit 1
  fi
done

if [[ ! -d "$prompts" ]]; then
  echo "no prompts in $prompts — run emit_refine_prompts first" >&2
  exit 1
fi

# Vertex bills the call and reports the token counts, but unlike the CLI it does
# not report what the call cost. So the price has to be written down, and it is
# written down *here*, in the envelope, per run — not in the scoring code —
# because a published cost column is only as honest as its source and a price
# that moved after a recording must not silently re-price it.
#
# USD per million tokens, standard tier, `global` location, prompts under the
# 200K long-context boundary (ours are ~5-8K, so the boundary never binds).
# Gemini: cloud.google.com/vertex-ai/generative-ai/pricing, read 2026-08-08.
# Reasoning tokens are billed at the output rate — the page's rows are labelled
# "Text output (response and reasoning)" — which is the same accounting the CLI
# arms use, so the two transports' cost columns are comparable.
# Claude: the rate `gd-26r.24` derived from the CLI's own `total_cost_usd` and
# token counts, which reproduces a recorded cost exactly. Anthropic's Vertex
# listing matches its direct listing, so the same rate is applied to both
# transports; if that stops being true this table is where it is wrong.
case "$model" in
  gemini-3-flash-preview)   price_in=0.50; price_out=3.00; price_cached=0.05 ;;
  gemini-3.1-pro-preview)   price_in=2.00; price_out=12.00; price_cached=0.20 ;;
  gemini-2.5-pro)           price_in=1.25; price_out=10.00; price_cached=0.13 ;;
  gemini-2.5-flash)         price_in=0.30; price_out=2.50; price_cached=0.03 ;;
  claude-opus-5)            price_in=5.00; price_out=25.00; price_cached=0.50 ;;
  claude-haiku-4-5*)        price_in=1.00; price_out=5.00; price_cached=0.10 ;;
  *)
    # Guessing a price would publish a cost column nobody can check.
    echo "no price on file for \`$model\` — add it to the table in this script" >&2
    echo "with the source and the date it was read, then re-run." >&2
    exit 1
    ;;
esac

# Which API this model speaks. Vertex fronts both, and they agree on nothing
# below the URL: different publisher, different verb, different request body,
# different usage field names.
case "$model" in
  gemini-*) publisher=google; verb=generateContent ;;
  claude-*) publisher=anthropic; verb=rawPredict ;;
  *) echo "cannot tell which publisher serves \`$model\`" >&2; exit 1 ;;
esac

host="https://aiplatform.googleapis.com"
if [[ "$location" != global ]]; then
  host="https://$location-aiplatform.googleapis.com"
fi
endpoint="$host/v1/projects/$project/locations/$location/publishers/$publisher/models/$model:$verb"

# One token for the whole recording. It outlives a corpus that takes under an
# hour, and re-minting it per call would put a gcloud round trip inside the wall
# clock this script exists to measure.
token="$(gcloud auth print-access-token)"

# Build the request body for one prompt file. Written by python3 rather than by
# string-pasting into JSON: the prompts contain quotes, newlines and backslashes,
# and a shell-quoted body would corrupt the very text being measured.
request_body() {
  TUICR_PROMPT_FILE="$1" TUICR_PUBLISHER="$publisher" TUICR_THINKING="$thinking" python3 - <<'PY'
import json, os

prompt = open(os.environ["TUICR_PROMPT_FILE"], encoding="utf-8").read()
thinking = os.environ["TUICR_THINKING"]

if os.environ["TUICR_PUBLISHER"] == "google":
    body = {"contents": [{"role": "user", "parts": [{"text": prompt}]}]}
    if thinking and thinking != "off":
        body["generationConfig"] = {"thinkingConfig": {"thinkingLevel": thinking}}
    elif thinking == "off":
        # Gemini 3 cannot be told to stop thinking outright; the floor is `low`.
        body["generationConfig"] = {"thinkingConfig": {"thinkingLevel": "low"}}
else:
    # `max_tokens` has to cover the answer *and* the thinking, and it is a hard
    # stop: too small truncates the JSON answer and the run scores as a parse
    # failure rather than as the model being wrong. The largest recorded answer
    # is ~8K tokens, so this leaves room for a long think on top.
    body = {
        "anthropic_version": "vertex-2023-10-16",
        "max_tokens": 32000,
        "messages": [{"role": "user", "content": prompt}],
    }
    if thinking and thinking != "off":
        body["thinking"] = {"type": "enabled", "budget_tokens": int(thinking)}

print(json.dumps(body))
PY
}

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
    if [[ -n "$shapes" && " $shapes " != *" $shape "* ]]; then
      continue
    fi
    body="$(request_body "$prompt")"
    for run in $(seq 1 "$runs"); do
      target="$out/$shape-$slug-$(printf '%02d' "$run").json"
      slots=$((slots + 1))
      if [[ -f "$target" ]]; then
        echo "have  $fixture/$shape run $run"
        continue
      fi
      sent=$((sent + 1))
      echo "call  $fixture/$shape run $run"

      # Wall clock is one of the two figures this whole ticket reports, so it is
      # measured around the request alone — token minting, body building and
      # envelope writing all sit outside it. Milliseconds via python3 because
      # BSD `date` has no %N.
      started="$(python3 -c 'import time; print(int(time.time() * 1000))')"
      status="$(curl -sS -o "$target.body" -w '%{http_code}' -X POST "$endpoint" \
        -H "Authorization: Bearer $token" \
        -H 'Content-Type: application/json' \
        --data-binary "$body" || echo 000)"
      elapsed="$(python3 -c "import time; print(int(time.time() * 1000) - $started)")"

      if [[ "$status" != 200 ]]; then
        echo "  HTTP $status; leaving $target.body for inspection" >&2
        failed=$((failed + 1))
        continue
      fi

      # The envelope. The provider response goes in whole and unedited — it is
      # the evidence — and everything around it is what Vertex does not supply:
      # what was asked for, how long it took, and the prices the cost column is
      # derived from.
      jq -n \
        --arg model "$model" \
        --arg publisher "$publisher" \
        --arg location "$location" \
        --arg thinking "${thinking:-default}" \
        --argjson duration "$elapsed" \
        --argjson pin "$price_in" \
        --argjson pout "$price_out" \
        --argjson pcached "$price_cached" \
        --slurpfile response "$target.body" \
        '{transport: "vertex", model: $model, publisher: $publisher,
          location: $location, thinking: $thinking, duration_ms: $duration,
          price_usd_per_mtok: {input: $pin, output: $pout, cached: $pcached},
          response: $response[0]}' > "$target.partial"

      # A 200 is not an answer. Gemini stops on MAX_TOKENS or a safety filter
      # with an empty `parts`, and Anthropic returns `stop_reason: max_tokens`
      # with a truncated body — both of which would be recorded as evidence and
      # then scored as the model failing at the task rather than as the recorder
      # cutting it off.
      if ! jq -e '
            .response
            | if .candidates then
                (.candidates[0].finishReason == "STOP")
                and ((.candidates[0].content.parts // []) | length > 0)
              else
                .stop_reason == "end_turn"
              end' "$target.partial" >/dev/null 2>&1; then
        reason="$(jq -rc '.response | .candidates[0].finishReason // .stop_reason // "unknown"' "$target.partial")"
        echo "  the call did not finish its answer (\`$reason\`); leaving $target.partial" >&2
        failed=$((failed + 1))
        continue
      fi

      rm -f "$target.body"
      mv "$target.partial" "$target"
    done
  done
done

if (( slots == 0 )); then
  echo "no prompts under $prompts — run emit_refine_prompts first" >&2
  exit 1
fi

if (( failed > 0 )); then
  echo "$failed of $sent calls failed; the corpus is incomplete" >&2
  exit 1
fi

echo
echo "recorded. now: cargo test --test grouping_prototype -- --ignored --nocapture refine_report"
