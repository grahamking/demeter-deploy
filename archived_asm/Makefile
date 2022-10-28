main: main.s
	nasm -felf64 -o main.o main.s
	ld -s -n -N -o seed main.o

clean:
	rm main.o
	rm seed

.SILENT: main clean
