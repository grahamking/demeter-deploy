//
// Part of Demeter Deploy.
//
// Remote helper. This gets uploaded to remote server to calc CRC32 values of remote
// files. We want it as small as possible, and also fast.
// It's no_std so we do all our own syscalls
// Source build_env.sh to be able to build it. Do not source that for testing.
//
// Size notes:
// - opt-level = "z" reduces binary by 40%+.
// - don't use slices, use pointers instead:
//   . x[idx] pulls in panic machinery which pulls in format machinery.
//   . ERRS being padded and &[u8; 8] saves about 200 bytes versus &[u8].
//   . string contants (EM_*, USAGE, CR, etc) being pointers saves another 100 bytes
//     versus them being &str.
//
// BUILD
//
// export RUSTFLAGS="-Ctarget-cpu=core-avx2 -Clink-args=-nostartfiles -Crelocation-model=static -Clink-args=-Wl,-n,-N,--no-dynamic-linker,--no-pie,--build-id=none,--no-eh-frame-hdr"
// cargo build --release
// objcopy -R .eh_frame -R .got.plt target/release/seed target/release/seed-final
//
// Adjust target-cpu to match you server. Find it like this:
// `gcc -march=native -Q --help=target | grep march`
//

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![feature(portable_simd)]
#![feature(maybe_uninit_slice)]
#![feature(maybe_uninit_uninit_array)]
#![feature(maybe_uninit_array_assume_init)]

#[cfg(test)]
mod test;

use core::arch::asm;
#[cfg(not(test))]
use core::arch::global_asm;
use core::arch::x86_64::{
    _mm_cmpistri, _mm_crc32_u64, _mm_loadu_si128, _SIDD_CMP_RANGES, _SIDD_NEGATIVE_POLARITY,
};
use core::ffi::c_char;
use core::mem::{transmute, zeroed, MaybeUninit};
use core::ptr::copy_nonoverlapping;
use core::simd::u64x2;

const USAGE: *const c_char = "Usage: seed <dir>\n\0".as_ptr() as *const c_char;
const CR: *const c_char = "\n\0".as_ptr() as *const c_char;
const COLON: *const c_char = ":\0".as_ptr() as *const c_char;
const BUF_SIZE: u32 = 32768; // read 32k of directory entries at a time
const DT_DIR: u8 = 4; // directory
const DT_REG: u8 = 8; // regular file

const O_RDONLY: i32 = 0;
const O_CLOEXEC: i32 = 0o2000000;
const O_DIRECTORY: i32 = 0o200000;
const PROT_READ: i32 = 1; // mmap a file as read only
const MAP_SHARED: i32 = 1; // for mmap

const EACCES: i32 = -13; // Permission denied

const CRC32: u64 = 0xFFFFFFFF;

// error messages
const EM_AVX2: *const c_char = "Need AVX2\n\0".as_ptr() as *const c_char;
const EM_MISSING_SLASH: *const c_char = "Path must end in a single /\n\0".as_ptr() as *const c_char;
const EM_OPEN_FILE: *const c_char = "file open err for CRCing: \0".as_ptr() as *const c_char;
const EM_OPEN_DIR: *const c_char = "dir open err for listing: \0".as_ptr() as *const c_char;
const EM_FSTAT: *const c_char = "fstat err: \0".as_ptr() as *const c_char;
const EM_GETDENTS64: *const c_char = "getdents64 err: \0".as_ptr() as *const c_char;
const EM_MMAP: *const c_char = "mmap err: \0".as_ptr() as *const c_char;
const EM_MUNMAP: *const c_char = "munmap err: \0".as_ptr() as *const c_char;
const EM_CHDIR: *const c_char = "chdir err: \0".as_ptr() as *const c_char;
const EM_CLOSE: *const c_char = "close err: \0".as_ptr() as *const c_char;

// fd's
const STDOUT: u32 = 1;
const STDERR: u32 = 2;

// syscalls
const SYS_WRITE: u32 = 1;
const SYS_OPEN: u32 = 2;
const SYS_CLOSE: u32 = 3;
const SYS_FSTAT: u32 = 5;
const SYS_MMAP: u64 = 9;
const SYS_MUNMAP: u32 = 11;
const SYS_EXIT: i32 = 60;
const SYS_CHDIR: u32 = 80;
const SYS_GETDENTS64: u32 = 217;

// err codes
// Padding them and using array instead of slice saves about 200 bytes, I don't know why
const ERRS: [&[u8; 8]; 36] = [
    b"NOPE   \0", // never happens
    b"EPERM  \0", // Operation not permitted
    b"ENOENT \0", // No such file or directory
    b"ESRCH  \0", // No such process
    b"EINTR  \0", // Interrupted system call
    b"EIO    \0", // I/O error
    b"ENXIO  \0", // No such device or address
    b"E2BIG  \0", // Argument list too long
    b"ENOEXEC\0", // Exec format error
    b"EBADF  \0", // Bad file number
    b"ECHILD \0", // No child processes
    b"EAGAIN \0", // Try again
    b"ENOMEM \0", // Out of memory
    b"EACCES \0", // Permission denied
    b"EFAULT \0", // Bad address
    b"ENOTBLK\0", // Block device required
    b"EBUSY  \0", // Device or resource busy
    b"EEXIST \0", // File exists
    b"EXDEV  \0", // Cross-device link
    b"ENODEV \0", // No such device
    b"ENOTDIR\0", // Not a directory
    b"EISDIR \0", // Is a directory
    b"EINVAL \0", // Invalid argument
    b"ENFILE \0", // File table overfile
    b"EMFILE \0", // Too many open files
    b"ENOTTY \0", // Not a typewriter
    b"ETXTBSY\0", // Text file busy
    b"EFBIG  \0", // File too large
    b"ENOSPC \0", // No space left on device
    b"ESPIPE \0", // Illegal seek
    b"EROFS  \0", // Read-only file system
    b"EMLINK \0", // Too many links
    b"EPIPE  \0", // Broken pipe
    b"EDOM   \0", // Math argument out of domain of func
    b"ERANGE \0", // Math result not representable
    b"       \0", // custom error, no code or name
];

// /usr/include/bits/dirent.h
#[repr(C)]
struct Dirent64 {
    d_ino: u64,
    d_off: u64,
    d_reclen: u16,
    d_type: u8,
    d_name: [i8; 256],
}

// /usr/include/bits/struct_stat.h
// 144 bytes
#[repr(C)]
struct Stat {
    st_dev: u64,     /* Device.  */
    st_ino: u64,     /* file serial number.	*/
    st_nlink: u64,   /* Link count.  */
    st_mode: u32,    /* File mode.  */
    st_uid: u32,     /* User ID of the file's owner.  */
    st_gid: u32,     /* Group ID of the file's group.  */
    _pad0: u32,      /* switch back to u64 padding */
    st_rdev: u64,    /* Device number, if device.  */
    st_size: u64,    /* Size of file, in bytes.  */
    st_blksize: u64, /* Optimal block size for I/O.  */
    st_blocks: u64,  /* Number 512-byte blocks allocated. */
    _pad1: [u8; 72], /* struct st_atim and __glibc_reserved that we don't use */
}

const MAX_PATH_LEN: usize = 256;

// Rust asm! macro messes with the stack so start here
#[cfg(not(test))]
global_asm!(
    ".global _start",
    "_start:",
    "  pop rdi",        // argc
    "  add rsp, 8",     // skip param 0, program name
    "  mov rsi, [rsp]", // addr of param 0
    "  call enter",
    "  ud2",
);

#[no_mangle]
unsafe fn enter(argc: u32, dir_name: *const c_char) -> ! {
    if !has_avx2() {
        print_avx2_missing();
        exit(2);
    }
    if argc != 2 {
        print_err(USAGE);
        exit(0);
    }

    // check we have a slash at end of dir
    let dir_name_len = strlen_local(dir_name);
    let last_char = dir_name.add(dir_name_len - 1);
    if *last_char != b'/' as i8 {
        print_err(EM_MISSING_SLASH);
        exit(1);
    }

    // chdir so that our paths can be relative, hence shorter
    chdir(dir_name);

    // start in current directory
    handle_dir(b".\0".as_ptr() as *const c_char);

    exit(0);
}

// handle_dir: crc32 all the files in a directory
// calling itself on sub directories.
// Expects [active_dir] to contain the bytes of the directory name to crc,
//  relative to dir passed on cmd line.
unsafe fn handle_dir(dir: *const c_char) {
    let dir_fd = match open_dir(dir) {
        None => {
            return;
        }
        Some(fd) => fd,
    };

    let mut buf: [MaybeUninit<u8>; BUF_SIZE as usize] = MaybeUninit::uninit_array();
    let mut bytes_read = get_dir_entries(dir_fd, &mut buf);
    while bytes_read != 0 {
        process_chunk(dir, &MaybeUninit::array_assume_init(buf), bytes_read);
        bytes_read = get_dir_entries(dir_fd, &mut buf);
    }
    close(dir_fd);
}

// sub function of handle_dir
// pass size to avoid using slices which can panic and
//  pull in enormous amounts of format machinery.
unsafe fn process_chunk(dir: *const c_char, buf: &[u8], bytes_read: i32) {
    let mut bytes_processed = 0;
    let dir_len = strlen_local(dir);
    while bytes_processed < bytes_read {
        let dirent =
            transmute::<*const u8, *const Dirent64>(buf.as_ptr().add(bytes_processed as usize));
        let reclen = (*dirent).d_reclen;

        // build full path
        #[allow(unused_assignments)] // read via pointer
        let mut full_path = [0i8; MAX_PATH_LEN];
        full_path = zeroed();
        let mut path_ptr = full_path.as_mut_ptr();
        copy_nonoverlapping(dir, path_ptr, dir_len);
        path_ptr = path_ptr.add(dir_len);
        *path_ptr = '/' as i8;
        // struct Stat is 19 bytes long plus a variable null-terminated filename
        copy_nonoverlapping(
            (*dirent).d_name.as_ptr(),
            path_ptr.add(1),
            (reclen - 19) as usize,
        );

        match (*dirent).d_type {
            // it's a regular file
            DT_REG => {
                crc_print(full_path.as_ptr());
            }
            // it's a directory
            DT_DIR => {
                // it's a directory, should we skip it? ('.' and '..')
                let d_name = (*dirent).d_name.as_ptr();
                if !is_ignore_dir(d_name) {
                    // it's a dir we want to handle, recurse
                    handle_dir(full_path.as_ptr());
                }
            }
            _ => {} // it's not a file or directory, ignore it
        }
        bytes_processed += reclen as i32;
    }
}

// is_ignore_dir: Should we ignore this directory ('.' and '..')
pub(crate) unsafe fn is_ignore_dir(dir: *const i8) -> bool {
    let dir = dir as *const u16;
    let is_dot = *dir == 0x002E; // '.\0'
    let is_dot_dot = *dir == 0x2E2E; // '..'
    is_dot || is_dot_dot
}

// crc32's the file and outputs: "filename: crc32\n"
unsafe fn crc_print(filename: *const c_char) {
    print(filename.add(2)); // skip the "./" path prefix

    // print a character to separate filename and CRC
    // we use a colon for human readiness. a null byte would be more correct.
    print(COLON);

    // next calculate crc32, we print it at end of function
    let fd = match open_file(filename) {
        Some(fd) => fd,
        None => {
            return;
        }
    };
    let mut sb: MaybeUninit<Stat> = MaybeUninit::uninit();
    fstat(fd, &mut sb);
    let sb = sb.assume_init();
    let mut crc = 0; // if the file is empty the crc will be 0
    if sb.st_size != 0 {
        crc = calc_crc(fd, sb.st_size);
    }

    let mut crc_str: [c_char; 8] = [0; 8];
    itoa(crc, crc_str.as_mut_ptr());
    print(crc_str.as_ptr());
    print(CR);

    // close file so we don't run out of descriptors in large folders
    close(fd);
}

unsafe fn calc_crc(fd: i32, size: u64) -> u32 {
    let mmap_ptr = mmap(fd, size);
    let mut checksum = CRC32;
    let mut pos = 0;
    while pos < size {
        // read fd 8 bytes at a time, treating those 8 bytes as a u64
        checksum = _mm_crc32_u64(checksum, *(mmap_ptr.add(pos as usize) as *const u64));
        pos += 8;
    }

    munmap(mmap_ptr, size);
    (checksum & CRC32) as u32
}

unsafe fn mmap(fd: i32, size: u64) -> *const u8 {
    let mut ret: i64;
    asm!("syscall",
        inout("rax") SYS_MMAP => ret,
        in("rdi") 0, // let kernel choose starting address, page aligned
        in("rsi") size,
        in("rdx") PROT_READ,
        in("r10") MAP_SHARED,
        in("r8") fd,
        in("r9") 0, // offset in the file to start mapping
    );
    if ret <= 0 {
        error(ret as i32, EM_MMAP);
    }
    ret as *const u8
}

unsafe fn munmap(ptr: *const u8, size: u64) {
    let ret: i32;
    asm!("syscall",
         inout("eax") SYS_MUNMAP => ret,
         in("rdi") ptr,
         in("rsi") size,
    );
    error(ret, EM_MUNMAP);
}

unsafe fn fstat(fd: i32, sb: &mut MaybeUninit<Stat>) {
    let mut ret: i32;
    asm!("syscall",
         inout("eax") SYS_FSTAT => ret,
         in("edi") fd,
         in("rsi") sb as *mut MaybeUninit<Stat>,
    );
    error(ret, EM_FSTAT);
}

unsafe fn close(fd: i32) {
    let mut ret: i32;
    asm!("syscall",
         inout("eax") SYS_CLOSE => ret,
         in("edi") fd,
         options(nostack, nomem),
    );
    error(ret, EM_CLOSE);
}

// List directory entries.
// Returns the number of bytes read or 0 if no more directory entries
unsafe fn get_dir_entries(dir_fd: i32, buf: &mut [MaybeUninit<u8>]) -> i32 {
    let mut ret: i32;
    asm!("syscall",
        inout("eax") SYS_GETDENTS64 => ret,
        in("edi") dir_fd,
        // address of space for linux_dirent64 structures
        in("rsi") buf.as_mut_ptr(),
        in("edx") BUF_SIZE,
        options(nostack, nomem),
    );
    error(ret, EM_GETDENTS64);
    ret
}

unsafe fn open_file(filename: *const c_char) -> Option<i32> {
    open(filename, O_RDONLY | O_CLOEXEC, EM_OPEN_FILE)
}

unsafe fn open_dir(active_dir: *const c_char) -> Option<i32> {
    open(active_dir, O_RDONLY | O_DIRECTORY, EM_OPEN_DIR)
}

unsafe fn open(path: *const c_char, flags: i32, err_msg: *const c_char) -> Option<i32> {
    let mut result: i32;
    asm!("syscall",
        inout("eax") SYS_OPEN => result,
        in("rdi") path,
        in("esi") flags,
        options(nostack, nomem)
    );
    if result == EACCES {
        // EACCES Permission denied, we won't be able to rcp over it
        return None;
    } else if result < 0 {
        // print the path we tried to open
        print_err(path);
        print_err(CR);
        error(result, err_msg);
    }
    Some(result)
}

unsafe fn chdir(dir: *const c_char) {
    let mut err_code;
    asm!("syscall",
     in("rdi") dir,
     inout("eax") SYS_CHDIR => err_code,
     options(nostack, nomem)
    );
    error(err_code, EM_CHDIR);
}

pub(crate) unsafe fn itoa(num: u32, dest: *mut c_char) {
    if num == 0 {
        *dest = '0' as c_char;
        *dest.add(1) = 0;
        return;
    }
    let mut dest_idx = 0;
    let mut remain = num;
    let mut next_c: i8;
    // ASCIIify
    while remain > 0 {
        (next_c, remain) = ((remain % 10) as i8, remain / 10);
        *(dest.add(dest_idx)) = next_c + '0' as i8; // convert to ASCII
        dest_idx += 1;
    }
    // Reverse
    if dest_idx > 1 {
        let mut i = 0;
        let mid = dest_idx / 2 - 1;
        while i <= mid {
            let l = dest.add(i);
            let r = dest.add(dest_idx - (i + 1));
            (*l, *r) = (*r, *l);
            i += 1;
        }
    }
    // Add \0 terminator
    *(dest.add(dest_idx)) = 0;
}

// Print missing AVX2 message and exit, that means very old CPU on the server
// don't use strlen for printing because that needs sse4.2
unsafe fn print_avx2_missing() {
    asm!("
	mov edx, 10
	syscall", // 10 is length of AVX2
    in("rsi") EM_AVX2,
    in("rdi") STDERR,
    in("eax") SYS_WRITE,
    options(nostack, nomem)
    );
}

// Modern instructions we need: pcmpistri (SSE4_2), vmovdqa (AVX), vpxor (AVX2)
// cpuid: check for AVX2 (which implies AVX and SSE4.2)
// Note we can't use `cfg!` because that's compile time.
unsafe fn has_avx2() -> bool {
    let mut answer: u8;
    asm!("
	mov eax, 7
	mov ecx, 0
	cpuid
	shr ebx, 5
	and ebx, 1
    mov eax, ebx
    ", // output eax instead of ebx because LLVM uses ebx internally apparently
    out("al") answer,
    options(nostack, nomem)
    );
    answer != 0
}

unsafe fn exit(exit_code: i32) -> ! {
    asm!("syscall",
        in("eax") SYS_EXIT,
        in("edi") exit_code,
        options(nostack, nomem, noreturn)
    )
}

// print to stdout
unsafe fn print(s: *const c_char) {
    asm!("syscall",
        in("eax") SYS_WRITE,
        in("edi") STDOUT,
        in("rsi") s,
        in("edx") strlen_local(s) as u32,
        options(nostack, nomem),
    );
}

// print to STDERR
unsafe fn print_err(s: *const c_char) {
    asm!("syscall",
        in("eax") SYS_WRITE,
        in("edi") STDERR,
        in("rsi") s,
        in("edx") strlen_local(s) as u32,
        options(nostack, nomem),
    );
}

unsafe fn error(err_code: i32, msg: *const c_char) {
    if err_code >= 0 {
        return;
    }
    print_err(msg);

    let err_code = err_code.abs() as u32;
    if err_code < ERRS.len() as u32 {
        print_err(ERRS.get_unchecked(err_code as usize).as_ptr() as *const c_char);
    } else {
        // numeric err
        let mut code_str: [c_char; 8] = [0; 8];
        itoa(err_code, code_str.as_mut_ptr());
        print(code_str.as_ptr());
    }
    print_err(CR);
    exit(err_code as i32);
}

//
// Rust core relies on these existing.
// They are in crate compiler-builtins which stdlib imports.
//

// s must be 16 byte aligned
unsafe fn strlen_local(s: *const c_char) -> usize {
    let mut haystack = s as u64;
    let mut idx = 16; // pcmpistri instruction uses 16 for 'not found'
    let mut offset: i32 = -16;
    while idx == 16 {
        offset += 16;
        idx = _mm_cmpistri(
            u64x2::from_array([0xFF01, 0]).into(),
            _mm_loadu_si128(haystack as *const _),
            _SIDD_CMP_RANGES | _SIDD_NEGATIVE_POLARITY,
        );
        haystack += 16;
    }
    (offset + idx) as usize
}

#[no_mangle]
unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    asm!(
        "rep movsb",
        inout("rcx") n => _,
        inout("rdi") dest => _,
        inout("rsi") src => _,
        options(nostack, preserves_flags)
    );
    dest
}

#[no_mangle]
unsafe extern "C" fn memset(dest: *mut u8, c: i32, count: usize) -> *mut u8 {
    asm!(
        "rep stosb",
        inout("rcx") count => _,
        inout("rdi") dest => _,
        inout("al") c as u8 => _,
        options(nostack, preserves_flags)
    );
    dest
}

//
// machinery
//

#[cfg(not(test))]
#[panic_handler]
fn my_panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
