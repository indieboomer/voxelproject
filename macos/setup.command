#!/bin/bash
set -eu
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
python3 tools/setup.py "$@"
