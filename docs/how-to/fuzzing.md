# Fuzzing

Parsers that read untrusted bytes (FILE_NOTIFY buffers, USN records, filter
globs, settings, export files) are fuzzed nightly with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) 0.13.2 on Linux. libFuzzer
does not support Windows, so fuzz targets only use OS-independent code:
byte-slice inputs, no Win32 types.

## Layout

`fuzz/` is its own cargo-fuzz crate, outside the workspace members:

```text
fuzz/
  Cargo.toml          # [workspace] of its own, depends on peekbuddy-core / -win
  fuzz_targets/
    file_notify.rs    # one file per target
  corpus/<target>/    # cached between nightly runs
  artifacts/<target>/ # crashes, uploaded by CI
```

The nightly `fuzz` job runs every file in `fuzz/fuzz_targets/` for 20 minutes.
No target exists yet; the first one arrives with the FILE_NOTIFY parser (#44).

## Adding a target

1. Write the harness with [bolero](https://crates.io/crates/bolero) 0.13.7 so
   one harness serves as a property test on PRs and as a fuzz target here:

   ```rust
   #[test]
   fn file_notify_never_panics() {
       bolero::check!().for_each(|bytes: &[u8]| {
           let _ = peekbuddy_win::notify::Records::new(bytes).count();
       });
   }
   ```

2. Add `fuzz/fuzz_targets/<name>.rs` that calls the same function:

   ```rust
   #![no_main]
   libfuzzer_sys::fuzz_target!(|bytes: &[u8]| {
       let _ = peekbuddy_win::notify::Records::new(bytes).count();
   });
   ```

3. Register it in `fuzz/Cargo.toml` (`[[bin]]` with `test = false`, `doc = false`).
4. Seed `fuzz/corpus/<name>/` with a few real inputs.

## Reproducing a crash

Download the `fuzz-artifacts` artefact from the failing nightly run, then:

```sh
rustup toolchain install nightly
cargo install --locked cargo-fuzz@0.13.2
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-<hash>
cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/crash-<hash>   # minimise
```

Turn the minimised input into a regular unit test before fixing the bug, so
it stays fixed.
