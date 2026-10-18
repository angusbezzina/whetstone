#!/usr/bin/env bash
# Records a private agreement in this CI checkout: a must rule (no .unwrap())
# and a should rule (no dbg!), both scoped to demo/, each probed first.
set -euo pipefail
inspect=$(wh init --json --request-id demo)
wh init --json --action agree --request-id demo \
  --expected-revision "$(jq -r .expected_revision <<<"$inspect")" \
  --resume "$(jq -r .resume_token <<<"$inspect")" \
  --mission "Demonstrate Whetstone annotations." >/dev/null
rule() {
  local id=$1 strength=$2 method=$3 content=$4
  local query
  if [ "$method" = dbg ]; then
    query='((macro_invocation macro: (identifier) @name) @match (#eq? @name "dbg"))'
  else
    query='((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method "unwrap"))'
  fi
  local def
  def=$(jq -cn --arg s "$strength" --arg q "$query" '{type:"rule",strength:$s,enforcer:{kind:"ast",query:$q,language:"rust"},paths:["demo/**"]}')
  local args=(change --json --request-id "$id" --kind rule --record-id "$id" --content "$content" --rationale "Demo rule." --definition "$def" --probe)
  local first
  first=$(wh "${args[@]}")
  jq -r .data.probe.summary <<<"$first"
  local recorded
  recorded=$(wh "${args[@]}" --expected-revision "$(jq -r .expected_revision <<<"$first")" --resume "$(jq -r .resume_token <<<"$first")")
  wh change --json --request-id "accept-$id" --accept "$(jq -r .data.proposal.id <<<"$recorded")" | jq -r .summary
}
rule rule.no-unwrap must unwrap "Never call .unwrap() in demo code; handle the error."
rule rule.no-dbg should dbg "No dbg! is left in demo code."
