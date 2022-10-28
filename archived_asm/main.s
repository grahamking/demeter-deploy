;; seed
;; Helper for Demeter Deploy. The main program will scp this file to remote,
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
	MAX_PATH_LEN: equ 256
	USAGE: db `Usage: seed <dir>\n\0`
	CR: db "",10,0  ; 0 is the terminating null byte
	COLON: db ":",0
	BUF_SIZE: equ 32768 ; read 32k of directory entries at a time
	DT_DIR: equ 4 ; directory
	DT_REG: equ 8 ; regular file
	MAP_SHARED: equ 1 ; for mmap
	PROT_READ: equ 1 ; mmap a file as read only
	SLASH: equ "/" ; path separator
	DOT_DIR: db ".",0

; error messages

	EM_AVX2: db "Need AVX2",10,0
	EM_MISSING_SLASH: db `Path must end in a single /\n\0`
	EM_OPEN_FILE: db "file open err for CRCing: ",0
	EM_OPEN_DIR: db "dir open err for listing: ",0
	EM_FSTAT: db "fstat err: ",0
	EM_GETDENTS64: db "getdents64 err: ",0
	EM_MMAP: db "mmap err: ",0
	EM_MUNMAP: db "munmap err: ",0
	EM_CHDIR: db "chdir err: ",0
	EM_CLOSE: db "close err: ",0

; fd's
	STDOUT: equ 1
	STDERR: equ 2

; syscalls
	SYS_WRITE: equ 1
	SYS_OPEN: equ 2
	SYS_CLOSE: equ 3
	SYS_FSTAT: equ 5
	SYS_MMAP: equ 9
	SYS_MUNMAP: equ 11
	SYS_EXIT: equ 60
	SYS_CHDIR: equ 80
	SYS_GETDENTS64: equ 217

; err codes
	ERR0: db "",0 ; never happens
	ERR1: db "EPERM",10,0 ; Operation not permitted
	ERR2: db "ENOENT",10,0 ; No such file or directory
	ERR3: db "ESRCH",10,0 ; No such process
	ERR4: db "EINTR",10,0 ; Interrupted system call
	ERR5: db "EIO",10,0 ; I/O error
	ERR6: db "ENXIO",10,0 ; No such device or address
	ERR7: db "E2BIG",10,0 ; Argument list too long
	ERR8: db "ENOEXEC",10,0 ; Exec format error
	ERR9: db "EBADF",10,0 ; Bad file number
	ERR10: db "ECHILD",10,0 ; No child processes
	ERR11: db "EAGAIN",10,0 ; Try again
	ERR12: db "ENOMEM",10,0 ; Out of memory
	ERR13: db "EACCES",10,0 ; Permission denied
	ERR14: db "EFAULT",10,0 ; Bad address
	ERR15: db "ENOTBLK",10,0 ; Block device required
	ERR16: db "EBUSY",10,0 ; Device or resource busy
	ERR17: db "EEXIST",10,0 ; File exists
	ERR18: db "EXDEV",10,0 ; Cross-device link
	ERR19: db "ENODEV",10,0 ; No such device
	ERR20: db "ENOTDIR",10,0 ; Not a directory
	ERR21: db "EISDIR",10,0 ; Is a directory
	ERR22: db "EINVAL",10,0 ; Invalid argument
	ERR23: db "ENFILE",10,0 ; File table overflow
	ERR24: db "EMFILE",10,0 ; Too many open files
	ERR25: db "ENOTTY",10,0 ; Not a typewriter
	ERR26: db "ETXTBSY",10,0 ; Text file busy
	ERR27: db "EFBIG",10,0 ; File too large
	ERR28: db "ENOSPC",10,0 ; No space left on device
	ERR29: db "ESPIPE",10,0 ; Illegal seek
	ERR30: db "EROFS",10,0 ; Read-only file system
	ERR31: db "EMLINK",10,0 ; Too many links
	ERR32: db "EPIPE",10,0 ; Broken pipe
	ERR33: db "EDOM",10,0 ;	 Math argument out of domain of func
	ERR34: db "ERANGE",10,0 ; Math result not representable"
	ERR35: db "",10,0 ; custom error, no code or name
	ERRS: dq ERR0, ERR1, ERR2, ERR3, ERR4, ERR5, ERR6, ERR7, ERR8, ERR9, ERR10, ERR11, ERR12, ERR13, ERR14, ERR15, ERR16, ERR17, ERR18, ERR18, ERR20, ERR21, ERR22, ERR23, ERR24, ERR25, ERR26, ERR27, ERR28, ERR29, ERR30, ERR31, ERR32, ERR33, ERR34, ERR35
	ERRS_BYTE_LEN: equ $-ERRS  ; will need to divide by 8 to get num items

;;
;; .bss: Global variables
;; On x86/64 we want the stack to stay aligned on 64 bits, so there's no point
;; making variables under 8 bytes, we'd have to extend them to push.
;; We often treat them as if they were 4 bytes (mov eax, [file_fd]).
;;

section .bss align=8
	; address of name of directory passed on cmd line
	dir_name_ptr: resb 8
	; length of above
	dir_name_len: resb 8

	; fd of the directory we are looking at
	dir_fd: resb 8
	; bytes of directory entries remaining to process in dir we are looking at
	bytes_to_process: resb 8

	; fd of the file we are CRC-ing
	file_fd: resb 8
	; size of the file we are CRC-ing
	file_size: resb 8
	; address of mmap'ed file, which is how we load a file to CRC it
	mmap_ptr: resb 8

	; length of name of directory we are working on (< MAX_PATH_LEN)
	active_dir_len: resb 8
	; the name of the directory we are working on as [char]
	active_dir: resb MAX_PATH_LEN

	; full path (active_dir + filename) to print to output
	; vmovdqa (AVX2) requires 32 byte (256 bit) alignment
	alignb 32
	full_path: resb MAX_PATH_LEN

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
	; Modern instructions we need: pcmpistri (SSE4_2), vmovdqa (AVX), vpxor (AVX2)
	; cpuid: check for AVX2 (which implies AVX and SSE4.2)
	mov eax, 7
	mov ecx, 0
	cpuid
	shr ebx, 5
	and ebx, 1
	jz missing_avx2

	; number of cmd line arguments is at rsp
	; we want exactly 2, program name, and a directory
	mov al, BYTE [rsp]   ; don't need to clear al, registers start at 0
	cmp al, 2
	jne print_usage

	mov rdi, [rsp + 16] ; address of first cmd line parameter, the directory path

	; we'll need these later to make full paths
	mov [dir_name_ptr], rdi
	call strlen

	; check we have a slash at end of dir
	dec eax ; examine last character
	mov rsi, [dir_name_ptr]		; dir_name_ptr contains an address
	cmp BYTE [rsi + rax], SLASH
	jne missing_slash_err

	; chdir so that our paths can be relative, hence shorter
	mov rdi, rsi
	mov eax, SYS_CHDIR
	syscall
	err_check EM_CHDIR

	; zero the registers we use to clear full_path
	; those are either not modified during the program or reset (xmm0 in strlen)
	; so safe to do just once
	vpxor ymm0, ymm0, ymm0
	vpxor ymm3, ymm3, ymm3

	; start in current directory
	xor eax, eax
	mov ax, [DOT_DIR]
	mov [active_dir], ax
	inc QWORD [active_dir_len]
	call handle_dir

	; end
	jmp exit

;;
;; handle_dir: crc32 all the files in a directory
;; calling itself on sub directories.
;; Expects [active_dir] to contain the bytes of the directory name to crc,
;;  relative to dir passed on cmd line.
;;
handle_dir:
	; open the dir
	mov rdi, active_dir ; rdi now has an address
	mov esi, 0x1000	; flags: O_RDONLY (0) | O_DIRECTORY (octal 0o200000)
	mov eax, SYS_OPEN
	syscall
	cmp eax, -13	; EACCES Permission denied, we won't be able to rcp over it
	je .ret			; so skip it.
	cmp eax, 0
	jl handle_dir_err

	; rax will be fd we just opened
	mov [dir_fd], rax ; save open directory fd

	sub rsp, BUF_SIZE

	; get directory entries

.next_files_chunk:
	mov rdi, [dir_fd]

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

	mov rdi, [dir_fd]
	mov eax, SYS_CLOSE
	safe_syscall
	err_check EM_CLOSE

.ret:
	ret


;;
;; sub function of handle_dir
;; rdi: address of first record (from getdents64)
;; esi: number of bytes to process (all records)
;;
process_single:

	mov rbx, rdi
	mov [bytes_to_process], rsi ; number of bytes in all records

.process_filenames:
	cmp BYTE [rbx+dirent64.d_type], DT_REG
	je .crc_file

	; it's not a file - is it a directory?
	cmp BYTE [rbx+dirent64.d_type], DT_DIR
	jne .move_to_next_record ; if it's not file or dir, skip

	; it's a directory, should we skip it? ('.' and '..')
	xor edi, edi
	mov di, WORD [rbx+dirent64.d_name] ; filename field of struct
	call is_ignore_dir
	cmp eax, 1
	je .move_to_next_record

	; it's a dir we want to handle, recurse
	push rbx
	push QWORD [dir_fd]
	push QWORD [bytes_to_process]
	push QWORD [active_dir_len]

	; append this dir to current one

	; add a slash separator
	mov rcx, [active_dir_len]
	mov rdi, active_dir		; rdi now has an address
	add rdi, rcx			; increase address by length
	mov BYTE [rdi], SLASH
	inc rcx

	; add this dir
	lea rdi, [rbx+dirent64.d_name]	; filename field of struct
	call strlen ; returns (rax) length of dir name, which is in rdi
	lea rdi, [active_dir + rcx]		; destination
	lea rsi, [rbx+dirent64.d_name]	; source
	mov ecx, eax					; copy rcx many bytes (strlen result)
	inc ecx							;  plus 1 to include the null terminator.
	add QWORD [active_dir_len], rcx
	rep movsb

	call handle_dir

	pop QWORD [active_dir_len]
	pop QWORD [bytes_to_process]
	pop QWORD [dir_fd]
	pop rbx

	; truncate active_dir contents to length before subdir call
	mov rax, [active_dir_len]
	mov BYTE [active_dir + rax], 0

	jmp .move_to_next_record

	; it's a file
.crc_file:

	; zero path memory using AVX instructions
	vmovdqa [full_path], ymm0
	vmovdqa [full_path+32], ymm3
	vmovdqa [full_path+64], ymm0
	vmovdqa [full_path+96], ymm3
	vmovdqa [full_path+128], ymm0
	vmovdqa [full_path+160], ymm3
	vmovdqa [full_path+192], ymm0
	vmovdqa [full_path+224], ymm3

	; copy dir path

	mov rdi, active_dir
	call strlen
	mov rcx, rax

	cld
	mov rdi, full_path		; destination
	mov rsi, active_dir		; source
	rep movsb

	; path separator
	mov BYTE [rdi], SLASH
	inc rdi

	; copy filename after it
	push rdi
	lea rdi, [rbx+dirent64.d_name] ; filename field of struct
	mov rsi, rdi ; source for 'rep movsb', the filename. strlen changes rdi so do first.
	call strlen
	mov rcx, rax
	pop rdi      ; destination, continue after path. source was set earlier
	rep movsb

	mov rdi, full_path   ; full path of file to crc
	call crc_print

	; move to next record
.move_to_next_record:
	mov ax, WORD [rbx+dirent64.d_reclen]
	add rbx, rax

	sub [bytes_to_process], eax
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
	add rdi, 2   ; skip the "./" path prefix
	call print

	; print a character to separate filename and CRC
	; we use a colon for human readiness. a null byte would be more correct.
	mov edi, COLON
	call print

	; next calculate crc32, we print it at end of function

	; open
	mov rdi, r10 ; filename pointer saved earlier
	mov esi, 0x80000 ; flags: O_RDONLY (0) | O_CLOEXEC (octal 0o2000000)
	mov eax, SYS_OPEN
	safe_syscall
	cmp eax, -13 ; EACCES Permission denied - skip this file
	je .ret
	err_check EM_OPEN_FILE

	mov [file_fd], rax

	; space to put stat buffer, on the stack
	sub rsp, 144 ; struct stat in stat/stat.h

	; fstat file to get size
	mov eax, SYS_FSTAT
	mov edi, [file_fd]
	mov rsi, rsp	; &stat
	safe_syscall
	err_check EM_FSTAT

	mov rax, [rsp + 48] ; stat st_size is 44 bytes into the struct
						; but I guess 4 bytes of padding?
	mov [file_size], rax
	add rsp, 144		; pop stat buffer

	; if the file is empty the crc will be 0, so skip straight to output.
	; once we get to that label the crc is expected to be in rax, so
	; we can leave the 0 it already has.
	cmp rax, 0
	je .got_crc

	; mmap it
	mov rsi, rax			; size
	mov eax, SYS_MMAP
	mov edi, 0				; let kernel choose starting address, page aligned
	mov edx, PROT_READ
	mov r10, MAP_SHARED		; flags
	mov r8, [file_fd]
	mov r9, 0				; offset in the file to start mapping
	safe_syscall
	err_check EM_MMAP

	; mmap_ptr is **u8. It contains the address of a reserved (.bss) area
	; that reserved area contains the address of the mmap section
	mov [mmap_ptr], rax ; mmap address

	; crc32, which has to happen 8 bytes at a time

	mov eax, 0xFFFFFFFF
	mov rcx, [file_size]
	mov rsi, [mmap_ptr]

	; - Optimization:
	; https://github.com/htot/crc32c/blob/master/crc32c/crc_iscsi_v_pcl.asm
	; Split area into 3, do 3 crc32 at a time. Three cache lines, and crc32 latency is 3
	; finally combine with three crc32's with pclmulqdq.
	; Only if file_size > ~200 bytes, otherwise this version is faster.
	; - What if size not a multiple of 8?
.crc32_next_8:
	crc32 rax, QWORD [rsi]
	add rsi, 8
	sub rcx, 8
	jg .crc32_next_8 ; jump if rcx above 0

	; munmap
	push rax
	mov eax, SYS_MUNMAP
	mov rdi, [mmap_ptr]
	mov rsi, [file_size]
	safe_syscall
	err_check EM_MUNMAP
	pop rax

.got_crc:

	; convert crc32 to string
	mov edi, eax	; crc32 value is in rax
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

	; close file so we don't run out of descriptors in large folders
	mov edi, [file_fd]
	mov eax, SYS_CLOSE
	safe_syscall
	err_check EM_CLOSE

.ret:
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

;;
;; err handling for handle_dir. it's special so that we can print the
;; dir we failed to open.
;;
;; rax: err code from syscall
;; rdi: the dir we tried to open
handle_dir_err:

	; print the dir we tried to open
	call print
	mov edi, CR
	call print

	; err code from syscall is still in rax
	mov rdi, EM_OPEN_DIR
	jmp err

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
;; OVERWRITES xmm0/ymm0/zmm0
;;
strlen:
	push rcx
	push rdx
	sub rsp, 16
	movdqu [rsp], xmm0

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

	movdqu xmm0, [rsp]
	add rsp, 16
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

	mov rdi, [ERRS+rax*8]
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
	mov rdi, EM_MISSING_SLASH
	call print

	mov edi, 1  ; return code
	mov eax, SYS_EXIT
	syscall

;; print missing AVX2 message and exit, that means very old CPU on the server
;; don't use strlen for printing because that needs sse4.2
missing_avx2:
	mov rsi, EM_AVX2
	mov edx, 10 ; length of EM_AVX2
	mov rdi, STDERR
	mov eax, SYS_WRITE
	syscall

	mov edi, 2  ; return code
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

