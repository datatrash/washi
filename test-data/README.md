# test-data

Test fixtures for `washi`'s integration-style unit tests.

Each test case under `src/main.rs` reads its inputs and (when applicable) its
expected outputs from this folder, using the convention:

- `<test_name>.input.wgsl`     — the WGSL source that gets minified
- `<test_name>.expected.wgsl`  — the expected minified WGSL (whitespace-insensitive)
- `<test_name>.expected.map`   — the expected `washi.map` file contents (where applicable)

Keeping the inputs/outputs as real files makes them easy to read, edit, and diff.

