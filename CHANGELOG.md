# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Server and client TOML configs now reject unknown fields (`deny_unknown_fields`):
  typos fail fast instead of being silently ignored.
- Migrated the optional `media` feature from `webrtc 0.17` to `webrtc 0.21.0-alpha.1`
  (Sans-I/O API rewrite). Behavior preserved: single H.264 outbound track, host-only
  trickle ICE, receive no-op. Adds `rtc` + `async-trait` as direct deps and drops the
  dormant `gstreamer-video` dependency. Alpha status is a documented risk — tracked
  upstream for a stable 0.21.

## [0.1.4] - unreleased

### Added
- `CHANGELOG.md` and a `keep-a-changelog` discipline for releases.

### Changed
- Bumped crate version to `0.1.4`.
- Added the required `authors` field to `Cargo.toml` (needed for `cargo publish`).

### Fixed
- `CONTRIBUTING.md`: the local `act` example used `act pull_request`, which fails
  locally without a remote base ref; changed to `act push` to match `AGENTS.md`.

> This release aggregates the comprehensive top-to-bottom refinement tracked by
> wayfinder map #165 (the flo-rs 0.1.4 refinement effort). Entries are appended
> here as each fix lands; the `## [Unreleased]` section above collects the next
> cycle's changes.
