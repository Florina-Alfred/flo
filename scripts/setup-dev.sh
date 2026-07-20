#!/usr/bin/env bash
# Install the system dependencies needed to build the `media` feature, which
# wraps GStreamer. The default build has zero system dependencies; this script
# is only required if you intend to compile with `--features media`.
#
# Usage:
#   ./scripts/setup-dev.sh            # auto-detect package manager
#   ./scripts/setup-dev.sh --apt      # force apt (Debian/Ubuntu)
#   ./scripts/setup-dev.sh --brew     # force Homebrew (macOS)
set -euo pipefail

APT_PKGS="libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libx264-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly"
BREW_PKGS="gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly x264"

detect() {
  if [[ "$(uname)" == "Darwin" ]]; then
    echo "brew"
  elif command -v apt-get >/dev/null 2>&1; then
    echo "apt"
  else
    echo "unknown"
  fi
}

PM="${1:-}"
if [[ -n "${PM}" ]]; then
  PM="${PM#--}"
else
  PM="$(detect)"
fi

case "${PM}" in
  apt)
    echo "Installing via apt: ${APT_PKGS}"
    sudo apt-get update
    # shellcheck disable=SC2086
    sudo apt-get install -y ${APT_PKGS}
    ;;
  brew)
    echo "Installing via Homebrew: ${BREW_PKGS}"
    # shellcheck disable=SC2086
    brew install ${BREW_PKGS}
    ;;
  *)
    echo "Could not detect a supported package manager (apt/brew)." >&2
    echo "Install these manually: ${APT_PKGS}" >&2
    exit 1
    ;;
esac

echo
echo "Done. Verify pkg-config resolves the GStreamer packages:"
pkg-config --exists gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0 \
  && echo "  gstreamer-1.0, gstreamer-app-1.0, gstreamer-video-1.0: OK" \
  || echo "  pkg-config could not find GStreamer (set PKG_CONFIG_PATH?)"
