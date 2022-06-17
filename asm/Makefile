main: main.s
	nasm -g -felf64 main.s
	# the -s strips debug info, remove for gdb
	ld -s -n -N -o rcple-h  main.o

clean:
	rm *.o
	rm rcple-h

.SILENT: main clean
