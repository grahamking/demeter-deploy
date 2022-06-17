# DEBUG ONLY
# In normal operation `cargo build` will use build.rs (in our root dir) to build the assembly helper.
# This Makefile is only for debugging the assembly helper. It keeps the debugging symbols for gdb.

main: main.s
	nasm -g -felf64 -o main-dbg.o main.s
	ld -n -N -o rcple-dbg main-dbg.o

clean:
	rm main-dbg.o
	rm rcple-dbg

.SILENT: main clean
