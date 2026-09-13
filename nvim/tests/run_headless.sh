#!/usr/bin/env bash
# Run every headless spec in this directory. Later tasks append their specs
# (spec_card.lua, spec_trans.lua, ...) to the list.
set -e
cd "$(dirname "$0")"

SPECS=(
  spec_skeleton.lua
  spec_card.lua
  spec_trans.lua
)

for spec in "${SPECS[@]}"; do
  echo "== $spec =="
  nvim --headless -u minimal_init.lua -l "$spec"
done
