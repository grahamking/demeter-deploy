;; rcpl-h
;; Helper for rcpl. The main program will scp this file to remote,
;; where it will gather CRC32 of all the files in given directory.
;; This helps main program know what to upload.

;; syscall (kernel) convention:
;;   syscall number in RAX
;;   IN: RDI, RSI, RDX, R10, R8 and R9
;;  OUT: RAX
;;
;; syscall numbers: /usr/include/asm/unistd_64.h
;; error codes: /usr/include/asm-generic/errno-base.h
;;

;;
;; macros
;;

;; handle error and exit
;; param1: err message
%macro err_check 1
	cmp rax, 0
	mov rdi, %1 ; should be a conditional move, but no immediate for that
	jl err
%endmacro

;; syscall overwrites rcx and r11, and I'm not going to remember every time
%macro safe_syscall 0
	push rcx
	push r11
	syscall
	pop r11
	pop rcx
%endmacro

;;
;; structs
;;

; directory entry
; /usr/include/bits/dirent.h
struc dirent64
	.d_ino resq 1		; 64-bit inode number
    .d_off resq 1		; 64-bit offset to next structure
	.d_reclen resw 1	; Size of this dirent
	.d_type resb 1		; File type - we care about DT_DIR and DT_REG
	.d_name resb 256	; Filename (null-terminated)
endstruc

;;
;; .data
;;
section .data align=16
	MAX_FNAME_LEN: equ 100
	MAX_PATH_LEN: equ 256
	USAGE: db `Usage: rcple-h <dir>\n\0`
	CR: db "",10,0
	SPACE: db " ",0
	BUF_SIZE: equ 32768 ; read 32k of directory entries at a time
	DT_DIR: equ 4 ; directory
	DT_REG: equ 8 ; regular file
	MAP_SHARED: equ 1 ; for mmap
	PROT_READ: equ 1 ; mmap a file as read only
	SLASH: equ "/" ; path separator
	MISSING_SLASH: db `Path must end in a single /\n\0`

; error messages

	EM_OPEN: db "open error: ",0
	EM_FSTAT: db "fstat error: ",0
	EM_GETDENTS64: db "getdents64 error: ",0
	EM_MMAP: db "mmap error: ",0
	EM_MUNMAP: db "munmap error: ",0
	EM_CHDIR: db "fhdir error: ",0

; fd's
	STDIN: equ 0
	STDOUT: equ 1
	STDERR: equ 2

; syscalls
	SYS_READ: equ 0
	SYS_WRITE: equ 1
	SYS_OPEN: equ 2
	SYS_CLOSE: equ 3
	SYS_FSTAT: equ 5
	SYS_MMAP: equ 9
	SYS_MUNMAP: equ 11
	SYS_IOCTL: equ 16
	SYS_MSYNC: equ 26
	SYS_EXIT: equ 60
	SYS_CHDIR: equ 80
	SYS_GETDENTS64: equ 217

; err codes
	ERR0: db "",0 ; never happens
	ERR1: db "EPERM Operation not permitted",10,0
	ERR2: db "ENOENT No such file or directory",10,0
	ERR3: db "ESRCH No such process",10,0
	ERR4: db "EINTR Interrupted system call",10,0
	ERR5: db "EIO I/O error ",10,0
	ERR6: db "ENXIO No such device or address",10,0
	ERR7: db "E2BIG Argument list too long",10,0
	ERR8: db "ENOEXEC Exec format error",10,0
	ERR9: db "EBADF Bad file number ",10,0
	ERR10: db "ECHILD No child processes",10,0
	ERR11: db "EAGAIN Try again",10,0
	ERR12: db "ENOMEM Out of memory",10,0
	ERR13: db "EACCES Permission denied",10,0
	ERR14: db "EFAULT Bad address",10,0
	ERR15: db "ENOTBLK Block device required",10,0
	ERR16: db "EBUSY Device or resource busy",10,0
	ERR17: db "EEXIST File exists",10,0
	ERR18: db "EXDEV Cross-device link",10,0
	ERR19: db "ENODEV No such device",10,0
	ERR20: db "ENOTDIR Not a directory",10,0
	ERR21: db "EISDIR Is a directory",10,0
	ERR22: db "EINVAL Invalid argument",10,0
	ERR23: db "ENFILE File table overflow",10,0
	ERR24: db "EMFILE Too many open files",10,0
	ERR25: db "ENOTTY Not a typewriter",10,0
	ERR26: db "ETXTBSY Text file busy",10,0
	ERR27: db "EFBIG File too large",10,0
	ERR28: db "ENOSPC No space left on device",10,0
	ERR29: db "ESPIPE Illegal seek",10,0
	ERR30: db "EROFS Read-only file system",10,0
	ERR31: db "EMLINK Too many links",10,0
	ERR32: db "EPIPE Broken pipe",10,0
	ERR33: db "EDOM	 Math argument out of domain of func",10,0
	ERR34: db "ERANGE Math result not representable",10,0
	ERR35: db "",10,0 ; custom error, no code or name
	ERRS: dq ERR0, ERR1, ERR2, ERR3, ERR4, ERR5, ERR6, ERR7, ERR8, ERR9, ERR10, ERR11, ERR12, ERR13, ERR14, ERR15, ERR16, ERR17, ERR18, ERR18, ERR20, ERR21, ERR22, ERR23, ERR24, ERR25, ERR26, ERR27, ERR28, ERR29, ERR30, ERR31, ERR32, ERR33, ERR34, ERR35
	ERRS_BYTE_LEN: equ $-ERRS  ; will need to divide by 8 to get num items

;;
;; .bss: Global variables
;;
section .bss align=64
	path_ptr: resb MAX_PATH_LEN  ; we store file path here, must be 64 byte aligned
	dir_fd_ptr: resb 8
	file_fd_ptr: resb 8
	file_size_ptr: resb 8
	mmap_ptr_ptr: resb 8		; address of mmap'ed file
	dir_name_ptr: resb 8
	dir_name_len_ptr: resb 8
	bytes_to_process_ptr: resb 4

;;
;; .text
;;
section .text

global _start

;;
;; main
;; most of the code is here
;;
_start:

	; number of cmd line arguments is at rsp
	; we want exactly 2, program name, and a directory
	mov al, BYTE [rsp]   ; don't need to clear al, registers start at 0
	cmp al, 2
	jne print_usage

	mov rdi, [rsp + 16] ; address of first cmd line parameter, the directory path

	; we'll need these later to make full paths
	mov [dir_name_ptr], rdi
	call strlen
	mov [dir_name_len_ptr], rax

	; check we have a slash at end of dir
	dec eax ; examine last character
	mov rsi, [dir_name_ptr]		; dir_name_ptr contains an address, so double indirect
	cmp BYTE [rsi + rax], SLASH
	jne missing_slash_err

	; chdir so that our paths can be relative, hence shorter
	mov rdi, rsi
	mov eax, SYS_CHDIR
	syscall
	err_check EM_CHDIR

	mov rdi, [dir_name_ptr]
	call handle_dir

	; end
	jmp exit

;;
;; handle_dir: crc32 all the files in a directory
;; calling itself on sub directories
;; rdi: const char* path, of directory to crc
;;
handle_dir:
	; open the dir
	; rdi already has dir name ptr
	mov esi, 0x1000	; flags: O_RDONLY (0) | O_DIRECTORY (octal 0o200000)
	mov eax, SYS_OPEN
	syscall
	err_check EM_OPEN

	; rax will be fd we just opened
	mov [dir_fd_ptr], rax ; save open directory fd

	sub rsp, BUF_SIZE

	; zeroed registers for zeroing memory later
	vpxord zmm0, zmm0, zmm0
	vpxord zmm3, zmm3, zmm3

	; get directory entries

.next_files_chunk:
	mov edi, [dir_fd_ptr]

	mov rsi, rsp		; address of space for linux_dirent64 structures
	mov edx, BUF_SIZE	; size of buffer (rsi) in bytes
	mov eax, SYS_GETDENTS64
	safe_syscall
	err_check EM_GETDENTS64

	; eax will be the number of bytes read
	; or 0 if no more directory entries
	; this is how we exit the next_files_chunk loop
	cmp eax, 0
	je .done_read

	mov rdi, rsp ; start of first record
	mov esi, eax ; number of bytes in all records
	call process_single

	jmp .next_files_chunk

.done_read:
	add rsp, BUF_SIZE
	ret


;;
;; sub function of handle_dir
;; rdi: address of first record (from getdents64)
;; esi: number of bytes to process (all records)
;;
process_single:

	mov rbx, rdi
	mov DWORD [bytes_to_process_ptr], esi ; number of bytes in all records

.process_filenames:
	cmp BYTE [rbx+dirent64.d_type], DT_REG
	je .crc_file

	; it's a dir, recurse
	xor edi, edi
	mov di, WORD [rbx+dirent64.d_name] ; filename field of struct
	call is_ignore_dir
	cmp eax, 1
	je .move_to_next_record

.here:
	push rbx
	push QWORD [dir_fd_ptr]
	push QWORD [bytes_to_process_ptr]
	lea rdi, [rbx+dirent64.d_name] ; filename field of struct

	call handle_dir

	pop QWORD [bytes_to_process_ptr]
	pop QWORD [dir_fd_ptr]
	pop rbx

	jmp .move_to_next_record

	; it's a file
.crc_file:
	; zero path memory using AVX-512 instructions
	vmovdqa64 [path_ptr], zmm0
	vmovdqa64 [path_ptr+64], zmm3
	vmovdqa64 [path_ptr+128], zmm0
	vmovdqa64 [path_ptr+192], zmm3

	; TODO this has to include relative dir

	cld
	lea rdi, [rbx+dirent64.d_name] ; filename field of struct
	mov rsi, rdi ; source for 'rep movsb', the filename. strlen changes rdi so do first.
	call strlen
	mov rcx, rax
	mov rdi, path_ptr		; destination
	rep movsb

	; copy dir path
	;cld
	;mov rdi, path_ptr		; destination
	;mov rsi, [dir_name_ptr] ; source
	;mov rcx, [dir_name_len_ptr] ; length
	;rep movsb

	; copy filename after it
	;push rdi
	;lea rdi, [rbx+dirent64.d_name] ; filename field of struct
	;mov rsi, rdi ; source for 'rep movsb', the filename. strlen changes rdi so do first.
	;call strlen
	;mov rcx, rax
	;pop rdi      ; destination, continue after path. source was set earlier
	;rep movsb

	mov rdi, path_ptr   ; full path of file to crc
	call crc_print

	; move to next record
.move_to_next_record:
	mov ax, WORD [rbx+dirent64.d_reclen]
	add rbx, rax

	sub [bytes_to_process_ptr], eax
	jnz .process_filenames
	ret
; end process_filenames


;;
;; rdi: pointer to null terminated filename
;; crc32's the file and outputs: "filename: crc32\n"
crc_print:

	push rax
	push rdx
	push rsi
	push rdi
	push r8
	push r9
	push r10

	; print the filename
	mov r10, rdi ; print does not preserve rdi
	call print

	; print a space
	mov edi, SPACE
	call print

	; next calculate crc32, we print it at end of function

	; open
	mov rdi, r10 ; filename pointer saved earlier
	mov esi, 0x80000 ; flags: O_RDONLY (0) | O_CLOEXEC (octal 0o2000000)
	mov eax, SYS_OPEN
	safe_syscall
	err_check EM_OPEN
	mov [file_fd_ptr], rax

	; space to put stat buffer, on the stack
	sub rsp, 144 ; struct stat in stat/stat.h

	; fstat file to get size
	mov eax, SYS_FSTAT
	mov edi, [file_fd_ptr]
	mov rsi, rsp	; &stat
	safe_syscall
	err_check EM_FSTAT

	mov rax, [rsp + 48] ; stat st_size is 44 bytes into the struct
						; but I guess 4 bytes of padding?
	mov [file_size_ptr], rax
	add rsp, 144		; pop stat buffer

	; mmap it
	mov rsi, rax			; size
	mov eax, SYS_MMAP
	mov edi, 0				; let kernel choose starting address
	mov edx, PROT_READ
	mov r10, MAP_SHARED		; flags
	mov r8, [file_fd_ptr]
	mov r9, 0				; offset in the file to start mapping
	safe_syscall
	err_check EM_MMAP

	; mmap_ptr is **u8. It contains the address of a reserved (.bss) area
	; that reserved area contains the address of the mmap section
	mov [mmap_ptr_ptr], rax ; mmap address

	; close file so we don't run out of descriptors in large folders
	mov rdi, [file_fd_ptr]
	mov eax, SYS_CLOSE
	safe_syscall

	; crc32, which has to happen 8 bytes at a time

	mov eax, 0xFFFFFFFF
	mov rcx, [file_size_ptr]
	mov rsi, [mmap_ptr_ptr]
.crc32_next_8:
	crc32 rax, QWORD [rsi]
	add rsi, 8
	sub rcx, 8
	jg .crc32_next_8 ; jump if rcx above 0

	; convert crc32 to string
	mov edi, eax
	sub rsp, 16		; space to put the string
	mov rsi, rsp
	call itoa

	; print code
	mov rdi, rsi
	call print
	add rsp, 16

	; print carriage return
	mov edi, CR
	call print

	; munmap
	mov eax, SYS_MUNMAP
	mov rdi, [mmap_ptr_ptr]
	mov esi, [file_size_ptr]
	safe_syscall  ; not safe_syscall, no need so late in the program
	err_check EM_MUNMAP

	pop r10
	pop r9
	pop r8
	pop rdi
	pop rsi
	pop rdx
	pop rax

	ret

;;
;; is_ignore_dir: Should we ignore this directory ('.' and '..')
;; IN rdi: address of dir name string
;; OUT ax: 1 if yes ignore it, 0 otherwise
;;
is_ignore_dir:
	push rbx

	xor eax, eax
	xor ebx, ebx

	cmp di, 0x002E ; '.\0'	; is it '.' dir?
	sete al					; set AL to 1 if they are equal, set to 0 otherwise

	cmp di, 0x2E2E ; '..'	; is it '..' dir?
	sete bl

	or eax, ebx				; is it either of them?

	pop rbx
	ret

;;;;;;;;;;;;;;;;;;;;;;;
;; Utility functions ;;
;;;;;;;;;;;;;;;;;;;;;;;

;;
;; print a null terminated string to stdout
;; DOES NOT PRESERVE rdi
;; rdi: str addr
;;
print:
	push rsi
	mov esi, STDOUT
	call fprint
	pop rsi
	ret

;;
;; print a null terminated string to stderr
;; rdi: str addr
;;
print_err:
	push rsi
	mov esi, STDERR
	call fprint
	pop rsi
	ret

;;
;; strlen: Length of null-terminated string with addr in rdi
;; length returned in rax
;;
strlen:
	push rcx
	push rdx

	xor eax, eax
	mov edx, 0xFF01		; range(01..FF), i.e. everything except null byte
	movd xmm0, edx		;  this is the range we are looking for
	sub eax, 16
	sub rdi, 16
.next:
	add eax, 16
	add rdi, 16
	pcmpistri xmm0, [rdi], 0x14	; Packed CMPare Implicit (\0 terminator) STRing
								;  returning Index.
								; 0x14 is control byte 1 01 00
								; 00: src is unsigned bytes
								; 01: range match
								; 1: negate the result (so match not in the range, i.e match \0)
	jnz .next
	add eax, ecx

	pop rdx
	pop rcx
	ret

;; Print null terminated string to file descriptor
;; DOES NOT PRESERVE rdi/rsi
;; rdi: str addr
;; rsi: open file descriptor
fprint:
	push rax
	push rdx

	push rdi
	push rsi

	call strlen
	mov edx, eax ; strlen now in edx

	; write syscall
	mov eax, SYS_WRITE
	; swap rdi/rsi from earlier push
	pop rdi  ; file descriptor now in rdi
	pop rsi  ; rsi now points at str addr
	safe_syscall

	pop rdx
	pop rax
	ret

;;
;; err: prints an error include error code and exits
;; Unusual ABI!
;; rax: err code, because it's already in there
;; rdi: err msg address
;;
err:
	call abs_rax
	call print_err

	mov ecx, ERRS_BYTE_LEN
	shr ecx, 3 ; divide by 8
	cmp eax, ecx
	jge .err_numeric

	mov rdi, QWORD [ERRS+rax*8]
	call print_err
	jmp exit

.err_numeric:
	; err code (rax) isn't in our table, print the code itself

	; convert code to string
	mov edi, eax
	sub rsp, 8
	mov rsi, rsp
	call itoa

	; print code
	mov rdi, rsi
	call print_err
	add rsp, 8

	; print carriage return
	mov rdi, CR
	call print_err

	jmp exit

;;
;; abs_rax: Absolute value ("abs" is reserved)
;; Unusual ABI!
;; rax: Number to convert. Is replaced with it's absolute value.
;;
global abs_rax
abs_rax:

	mov r11, rdx	; push rdx, faster. r11 is always fair game.
	; does the actual abs
	cqo ; fill rdx with sign of rax, so rdx will be 0 or -1 (0xFF..)
	xor eax, edx
	sub eax, edx
	mov rdx, r11	; pop rdx

	; MMX - 2x slower
	;pinsrw xmm0, eax, 0
	;pabsw xmm1, xmm0
	;pextrw eax, xmm1, 0

	; FPU - at least 5x slower, must go via memory
	;push rax			; can't copy directly x86 reg -> x87 reg, need to go via memory
	;fild qword [rsp]   ; copy to x87 register stack
	;fabs				; abs(top of FPU stack)
	;fistp qword [rsp]  ; copy from x87 register stack
	;pop rax			; rax now has abs value

	ret

;;
;; itoa: Convert number to string
;; rdi: number to convert
;; rsi: address to put converted number. Must have space.
;;
itoa:
	; prologue
	push rax
	push rbx
	push rcx
	push rdx
	push rsi
	push rdi
	push r8
	push rbp
	mov rbp, rsp
	sub rsp, 8    ; we only handle up to 8 digit numbers

	xor ecx, ecx
	mov rax, rdi  ; rax is numerator
	mov ebx, 10   ; 10 is denominator
	mov r8, rbp

.itoa_next_digit:
	; divide rax by 10 to get split digits into rax:rdx
	xor edx, edx  ; rdx to 0, it is going to get remainder
	div rbx
	add edx, 0x30	; convert to ASCII
	inc cl
	dec r8
	mov [r8], BYTE dl	; digits are in reverse order, so work down memory
							; this must be dl, a byte, so that 'movsb' can
							; move bytes later.
	test eax, eax			; do we have more digits?
	jg .itoa_next_digit

	; now copy them from stack into memory, they will be in correct order
	cld					; clear direction flag, so we walk up
	mov rdi, rsi		; rsi had desination address
	mov rsi, r8			; source is stack
						; rcx already has string length
	rep movsb			; repeat rcx times: copy rsi++ to rdi++
	mov [rdi], BYTE 0	; null byte to terminate string

	; epilogue
	add rsp, 8
	pop rbp
	pop r8
	pop rdi
	pop rsi
	pop rdx
	pop rcx
	pop rbx
	pop rax

	ret

;;
;; print usage and exit
;;
print_usage:
	mov rdi, USAGE
	call print
	call exit

;;
;; print missing slash error and exit
;;
missing_slash_err:
	mov rdi, MISSING_SLASH
	call print

	mov edi, 1  ; return code
	mov eax, SYS_EXIT
	syscall

;;
;; exit
;; never returns
;;
exit:
	mov edi, 0  ; return code 0
	mov eax, SYS_EXIT
	syscall

