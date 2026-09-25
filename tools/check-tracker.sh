#!/usr/bin/env bash
# check-tracker.sh — validate PhaseFlow tracker scaffolding.
# Usage: tools/check-tracker.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

errors=0
warn() { echo "WARN: $*"; }
die()  { echo "ERROR: $*"; errors=$((errors+1)); }

# 1. plan.md exists and has a phase table
[[ -f plan.md ]] || die "plan.md missing"
grep -q '^## Phases' plan.md || warn "plan.md has no '## Phases' section"

# 2. every referenced phase file exists
for f in phases/phase-*.md; do
  [[ -e "$f" ]] || { warn "no phase files found"; break; }
done

# 3. outputs/phase-N/.phase must exist for each phase and contain a known state
declare -A STATES=( [pending]=1 [in_progress]=1 [completed]=1 [reviewed]=1 [requires_fix]=1 [blocked]=1 [error]=1 )
for f in phases/phase-*.md; do
  num="${f#phases/phase-}"; num="${num%.md}"
  p="outputs/phase-$num/.phase"
  if [[ ! -f "$p" ]]; then die "missing $p for $(basename "$f")"; continue; fi
  state="$(cat "$p")"
  [[ -n "${STATES[$state]:-}" ]] || warn "$p has unknown state '$state'"
done

# 4. CURRENT_PLAN.md must have the HITO map and a free-ID line
[[ -f CURRENT_PLAN.md ]] || die "CURRENT_PLAN.md missing"
grep -q '## Mapa HITO ↔ Fases' CURRENT_PLAN.md || warn "CURRENT_PLAN.md missing HITO map"
grep -qi 'Siguiente ID libre' CURRENT_PLAN.md || warn "CURRENT_PLAN.md missing 'Siguiente ID libre'"

# 5. DECISIONS.md exists
[[ -f DECISIONS.md ]] || die "DECISIONS.md missing"

if [[ $errors -gt 0 ]]; then
  echo "Tracker validation FAILED ($errors error(s))."
  exit 1
fi
echo "Tracker OK."
