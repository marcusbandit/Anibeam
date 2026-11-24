#!/usr/bin/env bash
# Compatibility wrapper for tools that still invoke .github/local-ci.sh.
# The canonical script lives at scripts/local-ci.sh.
exec "$(dirname "$0")/../scripts/local-ci.sh" "$@"
