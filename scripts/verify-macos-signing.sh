#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-}"
if [[ -z "$TARGET" ]]; then
  echo "Usage: $0 <tauri-target>" >&2
  exit 2
fi

if [[ "$TARGET" != *apple-darwin ]]; then
  echo "Skipping macOS signing verification for target: $TARGET"
  exit 0
fi

BUNDLE_DIR="target/$TARGET/release/bundle/macos"
if [[ ! -d "$BUNDLE_DIR" ]]; then
  echo "macOS bundle directory not found: $BUNDLE_DIR" >&2
  exit 1
fi

APP_COUNT=0
while IFS= read -r app; do
  APP_COUNT=$((APP_COUNT + 1))
  echo "Verifying macOS app bundle: $app"

  if [[ ! -f "$app/Contents/_CodeSignature/CodeResources" ]]; then
    echo "Missing signature resource seal: $app/Contents/_CodeSignature/CodeResources" >&2
    exit 1
  fi

  codesign --verify --deep --strict --verbose=4 "$app"

  service="$app/Contents/Resources/resources/clash-verge-service"
  if [[ -f "$service" ]]; then
    codesign --verify --strict --verbose=4 "$service"

    if strings "$service" | grep -q '/var/run/clash-verge-service/service.sock'; then
      echo "Incompatible service IPC path embedded in $service: /var/run/clash-verge-service/service.sock" >&2
      exit 1
    fi

    if ! strings "$service" | grep -q '/tmp/verge/clash-verge-service.sock'; then
      echo "Expected service IPC path not found in $service: /tmp/verge/clash-verge-service.sock" >&2
      exit 1
    fi
  fi
done < <(find "$BUNDLE_DIR" -maxdepth 1 -type d -name '*.app' | sort)

if [[ "$APP_COUNT" -eq 0 ]]; then
  echo "No .app bundle found in $BUNDLE_DIR" >&2
  exit 1
fi
