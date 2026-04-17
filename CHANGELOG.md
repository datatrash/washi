# Changelog

All notable changes to this project are documented in this file.

## [0.3.0] - 2026-04-17

### Added
- Split identifier renaming into separate global and local flows.
- Added lexical local scope tracking so local declarations and lookups resolve correctly in nested blocks and `for` initializer scopes.
- Added a global-name reservation set so local variables never collide with renamed global/function identifiers.

### Fixed
- Excluded function-local names (including parameters and local declarations) from `washi.map` output.

## [0.2.1] - 2026-03-13

### Added
- When minifying multiple shaders with map generation, write `washi.map` to the root-most folder shared by matched files.

### Changed
- Documentation updates.

## [0.2.0] - 2026-03-13

### Added
- Added multi-file minification via glob patterns.
- Added optional map-file generation support.

## [0.1.2] - 2026-03-10

### Changed
- Replaced `naga` with `wgsl-parse`.
- Switched to manual traversal/minification across the full WGSL AST.

## [0.1.1] - 2026-03-09

### Changed
- Improved identifier generation.

## [0.1.0] - 2026-03-09

### Added
- Initial release.

