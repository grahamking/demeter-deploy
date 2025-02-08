# Demeter Deploy: Fast Directory Synchronization via SSH

Documentation by Gemini 2.0 Flash. It's very good!

Demeter Deploy, or `de`, is a command-line tool designed for quickly synchronizing local directories to remote servers over SSH.  It prioritizes speed and efficiency, particularly for blog deployments or similar tasks involving frequent file transfers.  `de` aims to provide a user experience similar to `scp` while offering enhanced performance.

## Key Features

*   **Fast Synchronization:** Utilizes concurrent SSH connections for parallel uploads, significantly reducing transfer times.
*   **Checksum-based Comparison:**  Compares files based on CRC32 checksums to identify changes and only transfer necessary files.
*   **Dry Run Mode:**  Simulates a deployment without actually modifying the remote server, allowing you to preview changes.
*   **Hidden File Support:** Option to include hidden files (dotfiles) in the synchronization process.
*   **Progress Reporting:** Provides real-time progress updates during the deployment.
*   **Familiar Syntax:** Uses a syntax similar to `scp` for ease of use.
*   **Efficient Remote Helper:** Uploads a small helper binary to the remote server to facilitate efficient file listing and checksum calculation.
*   **Concurrency:** Uses multiple worker threads to speed up the process.

## Installation

1.  **Clone the repository:**

    ```bash
    git clone https://github.com/grahamking/demeter-deploy.git
    cd demeter-deploy
    ```

2.  **Build the project:**

    ```bash
    ./build.sh  # Builds both the `de` and `seed-final` (helper) binaries
    ```

3.  **Install the binary (optional):**

    You can copy the compiled `de` binary from the `target/release` directory to a location in your `$PATH` for easy access.

    ```bash
    cp target/release/de /usr/local/bin/
    ```

## Usage

The basic usage of `de` is as follows:

```bash
de [OPTIONS] <src_dir> <user@host:remote_dst_dir>
```

*   `<src_dir>`:  The local directory you want to copy *from*. It must end with `/`.
*   `<user@host:remote_dst_dir>`: The remote destination in the format `user@host:/dir/`.  It also must end with `/`.

### Options

`de` supports the following command-line options:

*   `-v`, `--verbose`:  Enables verbose output, providing debug-level information.
*   `-H`, `--hidden`:  Includes hidden files (dotfiles) in the synchronization.  By default, hidden files are excluded.
*   `-w <workers>`, `--workers <workers>`:  Specifies the number of concurrent SSH connections to use for uploading. The default is 4.
*   `--helper-dst <path>`:  Specifies the full path to which the remote helper binary will be uploaded on the remote server.  The default is `/tmp/seed`.
*   `--dry-run`:  Performs a dry run, showing what actions *would* be taken without actually executing them.

### Examples

*   **Basic Synchronization:**

    ```bash
    de /home/graham/myfiles/ graham@myhost.com:/var/www/myfiles/
    ```

    This command synchronizes the contents of the local directory `/home/graham/myfiles/` to the remote directory `/var/www/myfiles/` on `myhost.com`, using the user `graham`.

*   **Dry Run:**

    ```bash
    de --dry-run /home/graham/myfiles/ graham@myhost.com:/var/www/myfiles/
    ```

    This command performs a dry run of the same synchronization, displaying the files that would be uploaded or deleted.

*   **Including Hidden Files:**

    ```bash
    de -H /home/graham/myfiles/ graham@myhost.com:/var/www/myfiles/
    ```

    This command synchronizes the directory, including hidden files and directories (those starting with a `.`).

*   **Specifying the Number of Workers:**

    ```bash
    de -w 8 /home/graham/myfiles/ graham@myhost.com:/var/www/myfiles/
    ```

    This command synchronizes the directory using 8 concurrent SSH connections. Increasing the number of workers can improve performance, but be mindful of server resources.

*   **Specifying the helper destination:**

    ```bash
    de --helper-dst /opt/demeter-helper /home/graham/myfiles/ graham@myhost.com:/var/www/myfiles/
    ```

    This command synchronizes the directory, uploading the helper to `/opt/demeter-helper` instead of the default `/tmp/seed`.

## How It Works

`de` works through the following steps:

1.  **Parsing Arguments:** The command-line arguments are parsed using the `clap` crate.
2.  **Local Checksum Calculation:** A background thread calculates the CRC32 checksum and file size for each file in the source directory.  This includes traversing subdirectories recursively.  Hidden files are excluded unless the `-H` option is specified.
3.  **SSH Connection:**  An SSH connection is established to the remote server using the provided username and hostname.  The `ssh2` crate is used for handling the SSH connection.
4.  **Helper Upload:**  The `seed-final` binary (the "helper") is uploaded to the remote server (default location `/tmp/seed`). This small executable is responsible for efficiently gathering information about the remote directory's contents.
5.  **Remote Checksum Calculation:**  The remote helper is executed on the server. It calculates CRC32 checksums and file sizes of the files in the destination directory.
6.  **Comparison:** The local and remote checksums and file sizes are compared.  A list of files to upload (if checksums differ or files are missing remotely) and files to delete (if they exist remotely but not locally) is generated.
7.  **File Transfer (Uploads):**  Files that need to be uploaded are transferred to the remote server using concurrent SSH connections managed by worker threads.  The file is read in chunks and written to the remote server.
    *   The destination directory structure is created as needed if it doesn't exist.
    *   Real-time progress updates are displayed, showing the number of files transferred, the percentage of bytes transferred, and the active files being uploaded.
8.  **File Deletion (Deletes):** Files that exist on the remote server but not locally are deleted.
9.  **Cleanup:**  The SSH connection is closed. The remote helper is *not* deleted.  This is because `/tmp` gets cleared anyway, and it saves time if you run `de` multiple times.

## Dependencies

`de` relies on the following Rust crates:

*   `anyhow`: Flexible error handling.
*   `clap`: Command-line argument parsing.
*   `crossbeam_channel`:  Concurrency primitives for communication between threads.
*   `ssh2`: Provides SSH client functionality.
*   `std`: The Rust standard library.

## `seed-final` (Remote Helper)

The `seed-final` binary is a small, self-contained executable that is uploaded to the remote server. Its purpose is to efficiently list files in the destination directory and calculate their CRC32 checksums and file sizes. This information is then sent back to the `de` client for comparison.  It is written in Rust for safety, performance, and easy cross-compilation.

**Key functionalities:**

*   Lists files in the target directory.
*   Calculates CRC32 checksums for each file.
*   Outputs filename, and crc32 checksum separated by the `:` character.
*   Handles hidden files based on whether they are in the source list.

## Code Structure

*   `src/main.rs`: The main entry point of the application.  Handles argument parsing, SSH connection setup, file comparison, and orchestrates the upload and delete operations.
*   `src/ssh_manager.rs`: Manages the SSH connections, including the primary connection and the worker threads for concurrent uploads. Implements dry-run functionality by swapping the `SSH` connection with a `MockSSH` connection.
*   `src/ssh.rs`: Implements the SSH connection and file transfer logic using the `ssh2` crate. Includes SFTP functions for secure file transfer.
*   `src/remote.rs`: Defines the `Remote` trait for interacting with remote servers.  This trait is implemented by both the `SSH` and `MockSSH` structs, enabling mocking for dry runs.
*   `src/progress_message.rs`: Defines the `Progress` enum used for sending progress updates from the worker threads to the main thread for display.
*   `src/output.rs`: Handles the display of progress information to the user.
*   `seed/src/main.rs`: The source code for the remote helper binary `seed-final`.

## Error Handling

The `anyhow` crate is used for error handling.  Errors are propagated up the call stack, providing context and information for debugging.  SSH-related errors are handled using `ssh2` specific error codes.

## Threading

`de` uses multi-threading to improve performance, especially for transferring large numbers of files. The main thread handles the high-level logic, such as parsing arguments and comparing files. Worker threads are spawned to handle the actual file uploads concurrently.  Crossbeam channels are used for communication between the main thread and the worker threads.

## Future Improvements

*   **Improved Error Reporting:** More detailed and user-friendly error messages.
*   **Resumable Transfers:** Support for resuming interrupted transfers.
*   **File Permissions:**  Preserve file permissions during transfers.
*   **Configuration File:** Support for storing configuration options in a file.
*   **Password Authentication:** Add support for password-based authentication (currently only supports SSH agent).
*   **Key-based Authentication:** Explicitly specify SSH key.
*   **More sophisticated diff:**  Use rsync algorithm or similar.
*   **Delete Helper:** Delete the remote helper after program runs.

## Contributing

Contributions are welcome!  Please submit pull requests with bug fixes, new features, or improvements to the documentation.
```

