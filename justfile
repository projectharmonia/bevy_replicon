#!/usr/bin/env just --justfile

raw-coverage $LLVM_PROFILE_FILE=(justfile_directory() / "target/coverage/profile-%p.profraw") $RUSTFLAGS="-C instrument-coverage":
  cargo test 

coverage *ARGS: raw-coverage
  grcov target/coverage \
    --binary-path target/debug/ \
    --source-dir . \
    --excl-start "mod tests" \
    --excl-line "#\[" \
    --ignore "/*" \
    {{ ARGS }}
