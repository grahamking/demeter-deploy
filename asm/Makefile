main: main.s
	nasm -g -felf64 main.s
	ld -n -N -o rcple-h  main.o

clean:
	rm *.o
	rm rcple-h

.SILENT: main clean
