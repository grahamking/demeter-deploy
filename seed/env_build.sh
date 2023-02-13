#!/bin/bash
#
# Source this for 'cargo build'
# Do not source this for 'cargo test'
#
# Adjust target-cpu to match you server. Find it like this:
# `gcc -march=native -Q --help=target | grep march`

export RUSTFLAGS="-Ctarget-cpu=core-avx2 -Clink-args=-nostartfiles -Crelocation-model=static -Clink-args=-Wl,-n,-N,--no-dynamic-linker,--build-id=none,--no-eh-frame-hdr"
