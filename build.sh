#!/bin/bash

cd seed
PREV_RUSTFLAGS=$RUSTFLAGS
source env_build.sh # set RUSTFLAGS, seed is no_std no_main
cargo build --release
objcopy -R .eh_frame -R .got.plt target/release/seed target/release/seed-final
export RUSTFLAGS=$PREV_RUSTFLAGS
cd ..

cd de
cargo build --release
if [[ -d ~/bin/ ]]
then
	ln --force --symbolic $(pwd)/target/release/de ~/bin/de
	echo "Done. Use ~/bin/de"
else
	echo "Done: Use target/release/de"
fi
cd ..

