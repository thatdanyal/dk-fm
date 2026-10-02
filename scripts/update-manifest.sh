#!/usr/bin/env bash
# Writes an electron-updater manifest (latest.yml / latest-linux.yml) into out/, so installed
# copies of the old Electron DK.FM find this release and install it.
# Usage: VERSION=x.y.z update-manifest.sh <manifest name> <main file> [other files...]
set -euo pipefail
name=$1
shift
sha() { openssl dgst -sha512 -binary "$1" | base64 | tr -d '\n'; }
size() { wc -c < "$1" | tr -d ' '; }
{
  echo "version: $VERSION"
  echo "files:"
  for f in "$@"; do
    echo "  - url: $(basename "$f")"
    echo "    sha512: $(sha "$f")"
    echo "    size: $(size "$f")"
  done
  echo "path: $(basename "$1")"
  echo "sha512: $(sha "$1")"
  echo "releaseDate: '$(date -u +%Y-%m-%dT%H:%M:%S.000Z)'"
} > "out/$name"
cat "out/$name"
