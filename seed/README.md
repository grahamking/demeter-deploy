# Demeter Deploy Remote Helper (seed)

Documentation by Gemini 2.0 Flash. Thanks!

This document provides a comprehensive overview of the `seed` program, a remote helper tool designed for Demeter Deploy.  It is specifically crafted for efficiency and minimal size, optimized to be uploaded to remote servers for calculating CRC32 checksums of remote files.  This tool operates in a `no_std` environment, necessitating custom implementations for system calls and core functionalities.

## Overview

The `seed` program is a crucial component of the Demeter Deploy system, responsible for:

*   **Remote Checksum Calculation:**  Generating CRC32 checksums of files on the remote server.
*   **Lightweight Design:** Minimized binary size for fast uploads and execution on resource-constrained remote environments.
*   **No Standard Library Dependence:**  `no_std` environment ensures a smaller footprint and greater control over dependencies.
*   **AVX2 Optimization:**  Utilizes AVX2 instructions (if available) for accelerated CRC32 calculation.
*   **Directory Traversal:** Recursively traverses directories, calculating checksums for all files within.

## Building

Before building `seed`, you **must** source the `build_env.sh` script. **Do not source the script for testing.** This script sets up the necessary environment variables to facilitate the build process.

```bash
source build_env.sh
```

The `build_env.sh` script should set `RUSTFLAGS` to a value similar to this:

```bash
export RUSTFLAGS="-Ctarget-cpu=core-avx2 -Clink-args=-nostartfiles -Crelocation-model=static -Clink-args=-Wl,-n,-N,--no-dynamic-linker,--no-pie,--build-id=none,--no-eh-frame-hdr"
```

Adjust the `-Ctarget-cpu` flag to reflect your server's processor. You can determine the appropriate value using:

```bash
gcc -march=native -Q --help=target | grep march
```

Once the environment is configured, build the program using Cargo:

```bash
cargo build --release
```

Post-build, strip unnecessary sections from the binary to further reduce its size:

```bash
objcopy -R .eh_frame -R .got.plt target/release/seed target/release/seed-final
```

The resulting binary, `target/release/seed-final`, is the deployable artifact.

## Core Functionality

The program's operation can be broken down into the following key steps:

1.  **AVX2 Check:** Verifies the availability of AVX2 instructions on the target system.  If AVX2 is unavailable, an error message is printed to `stderr`, and the program exits with code 2.
2.  **Argument Parsing:**  Validates the command-line arguments.  The program expects a single argument: the directory to process. Exits with an error message if insufficient arguments are provided, exiting with code 0.
3.  **Path Validation:**  Ensures that the provided directory path ends with a forward slash (`/`). If the trailing slash is missing, it prints an error message to `stderr` and exits with code 1.
4.  **Directory Change:** Changes the current working directory to the specified directory using `chdir`.  This allows for relative path handling, reducing path lengths in the output.
5.  **Recursive Directory Traversal:** Initiates the recursive directory traversal process, starting from the current directory (`.`). The `handle_dir` function handles this.
6.  **Checksum Calculation and Output:**  For each file encountered during the traversal, the program calculates its CRC32 checksum using AVX2 instructions and prints the filename and checksum to `stdout` in the format `filename: crc32\n`.

## Code Structure

### `src/main.rs`

This file constitutes the main source code of the `seed` program. It is responsible for the core logic of the program including the `enter` function, which is the actual entrypoint after the `_start` assembly code.

#### Global Assembly (`_start`)

The `_start` label is the actual program entry point. It retrieves the command-line arguments (argc and argv) and calls the `enter` function.

```assembly
.global _start
_start:
  pop rdi       ; argc
  add rsp, 8    ; skip param 0, program name
  mov rsi, [rsp] ; addr of param 0
  call enter
  ud2
```

#### `enter(argc: u32, dir_name: *const c_char) -> !`

This function is the logical entry point of the Rust code. It performs initial checks and initiates the directory traversal.

*   **AVX2 Check:** Calls `has_avx2()` to verify AVX2 support.
*   **Argument Validation:**  Checks if the correct number of arguments (2: program name and directory) is provided.
*   **Path Validation:** Checks for the trailing slash.
*   **`chdir` Call:**  Changes the current directory to the input `dir_name`.
*   **`handle_dir` Call:** Initiates the recursive directory traversal starting from the current directory.
*   **`exit` Call:** Exits the program.

#### `handle_dir(dir: *const c_char)`

This function handles the directory processing:

*   **`open_dir`:** Opens the specified directory using `open` syscall.
*   **`get_dir_entries`:** Reads directory entries in chunks of `BUF_SIZE` using the `getdents64` syscall.
*   **`process_chunk`:** Processes each chunk of directory entries.
*   **`close`:** Closes the directory file descriptor.

#### `process_chunk(dir: *const c_char, buf: &[u8], bytes_read: i32)`

This function processes a chunk of directory entries read by `get_dir_entries`:

*   Iterates through the buffer, extracting directory entries (represented by `Dirent64` structs).
*   Constructs the full path for each entry.
*   Determines if the entry is a regular file (`DT_REG`) or a directory (`DT_DIR`).
*   If it's a regular file, calls `crc_print`.
*   If it's a directory (and not `.` or `..`), recursively calls `handle_dir`.

#### `crc_print(filename: *const c_char)`

Calculates and prints the CRC32 checksum of a file:

*   **`open_file`:** Opens the file in read-only mode.
*   **`fstat`:** Retrieves file statistics (size) using `fstat` syscall.
*   **`calc_crc`:** Calculates the CRC32 checksum using the `_mm_crc32_u64` intrinsic (AVX2).
*   **`itoa`:** Converts the CRC32 checksum to a string.
*   **Prints filename, a colon, the checksum, and a newline to `stdout`.**
*   **`close`:** Closes the file descriptor.

#### `calc_crc(fd: i32, size: u64) -> u32`

Calculates the CRC32 checksum of the file:

*   **`mmap`:** Memory-maps the file using `mmap` syscall.
*   Calculates the CRC32 checksum using `_mm_crc32_u64` (AVX2) in 8-byte chunks.
*   **`munmap`:** Unmaps the file.

#### System Call Wrappers (`open`, `close`, `fstat`, `mmap`, `munmap`, `get_dir_entries`, `chdir`)

These functions wrap the raw system calls to provide a more Rust-friendly interface and error handling.  They use inline assembly (`asm!`) to execute the system calls.

#### Utility Functions (`itoa`, `strlen_local`, `print`, `print_err`, `error`, `is_ignore_dir`)

These functions provide utility operations:

*   **`itoa`:** Converts an unsigned 32-bit integer to a string.
*   **`strlen_local`:** Calculates the length of a null-terminated string using SSE4.2 instructions (`_mm_cmpistri`).
*   **`print`:** Prints a string to `stdout`.
*   **`print_err`:** Prints a string to `stderr`.
*   **`error`:** Prints an error message and exits if the error code is negative.
*   **`is_ignore_dir`:** Checks if a directory name should be ignored (i.e., "." or "..").

#### AVX2 Detection (`has_avx2`)

This function detects the availability of AVX2 instructions on the CPU using the `cpuid` instruction.

#### `memcpy` and `memset`

Standard memory manipulation functions, implemented in assembly for performance and `no_std` compatibility.

#### Panic Handler (`my_panic`)

A basic panic handler for `no_std` environments, preventing the program from unwinding on panic.  It simply enters an infinite loop.

### `src/test.rs`

This file contains unit tests for various functions, including `itoa` and `is_ignore_dir`.

## Data Structures

*   **`Dirent64`:** Represents a directory entry (matches `linux_dirent64` structure).
*   **`Stat`:** Represents file statistics (matches the `stat` structure).

## Constants

The code makes extensive use of constants for system call numbers, file flags, error codes, and messages to avoid memory allocation and improve performance. Notably:

*   **`SYS_*`:** System call numbers.
*   **`O_*`:** File open flags.
*   **`DT_*`:** Directory entry types.
*   **`EM_*`:** Error messages.
*   **`ERRS`:** Array of error string constants.
*   **`BUF_SIZE`:** Size of buffer for reading directory entries.
*   **`MAX_PATH_LEN`:** Maximum path length allowed.

## Optimization Strategies

The `seed` program employs several optimization strategies to minimize its binary size and maximize performance:

*   **`no_std` Environment:** Avoids the overhead of the standard library.
*   **Inline Assembly:**  Uses inline assembly for system calls and performance-critical sections (e.g., `strlen_local`, `memcpy`, `memset`).
*   **Static Linking:** Links statically to avoid runtime dependencies.
*   **Code Size Optimization:**  Uses `opt-level = "z"` in `Cargo.toml` for maximum code size optimization.
*   **AVX2 Intrinsics:** Utilizes AVX2 instructions (`_mm_crc32_u64`) for fast CRC32 calculation.
*   **Pointer Arithmetic:** Prefers raw pointers over slices to avoid panicking code and format machinery.
*   **String Constants as Pointers:** Defines string constants as `*const c_char` instead of `&str` to save space.
*   **Padding and Array Usage:** Uses padded arrays (e.g., `ERRS`) to minimize memory usage compared to slices.

## Error Handling

The program handles errors by:

*   Checking return values of system calls.
*   Printing error messages to `stderr` using `print_err`.
*   Exiting with a non-zero exit code using `exit`.
*   Using an array of pre-defined error messages (`ERRS`) to avoid dynamic memory allocation.

## Testing

The `src/test.rs` file contains unit tests for some of the utility functions.  However, due to the `no_std` environment and the reliance on system calls, comprehensive testing can be challenging.  Tests are included for `itoa` and `is_ignore_dir`.

```bash
cargo test
```

## Limitations

*   **AVX2 Requirement (Soft):** While the program attempts to detect AVX2, it doesn't provide a fallback if AVX2 is not available. This severely limits the target environments.
*   **Error Handling:**  Error handling is basic and focuses on minimizing code size.  More robust error reporting could be beneficial in some scenarios.
*   **Limited Testing:**  The testing is minimal due to the `no_std` environment and the reliance on system calls.
*   **Path Length Limit:** The `MAX_PATH_LEN` constant limits the maximum length of file paths that can be processed.

## Potential Improvements

*   **Fallback for Non-AVX2 CPUs:** Implement a non-AVX2 CRC32 calculation method (e.g., using a software-based algorithm) to support older CPUs.
*   **More Robust Error Handling:**  Improve error reporting by including more context in error messages.
*   **Expanded Testing:** Explore options for more comprehensive testing, perhaps using a mock system call library.
*   **Configuration Options:**  Add command-line options to control behavior, such as specifying the number of threads to use for CRC32 calculation.

This documentation provides a detailed explanation of the `seed` program's functionality, code structure, optimization strategies, and limitations. It serves as a valuable resource for developers who need to understand, maintain, or extend this crucial tool.
