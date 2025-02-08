# Demeter Deploy

Fast blog deployment, alternative to `rsync` and `scp`. I use this to deploy my Hugo blog files. It can be used for any operation that needs to push the contents of a directory to a remote server.

**Requires**: Rust, Linux x86-64. `sshd` on the remote server, with working keys and known_hosts already done.

**Build**: `./build.sh`

**Usage**: `de <src> <remote>`.

This is how I deploy, from the root of my Hugo directory:
```
de public my_server:/var/www/blog/ --dry-run
```

Then again without `--dry-run` to do it for real.

It uploads a very small helper binary (`seed`, because Demeter) which calculates a CRC32 of all the files. Then it uploads the new and modified ones, and deletes the removed ones.

## Notes

The original `seed` helper was in assembler. I kept it for posterity in `archived_asm/`. The Rust version is in `seed/` and **compiles to the same size**! See [A very small Rust binary indeed](https://darkcoding.net/software/a-very-small-rust-binary-indeed/) for how it's done.

## More, better docs

- [de, the main program](de/)
- [seed, the remote helper](seed/)
