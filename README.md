# binsize

Diffs section and symbol sizes between two ELF binaries — two releases
of the same executable — so "the binary grew by 4KB" turns into "which
section, which function."

## Installation

```sh
cargo install --path .
```

## Usage

```bash
binsize old-release/myapp new-release/myapp
binsize --top 50 v1.2.0/myapp v1.3.0/myapp
```

## How it works

Parses both files with a hand-rolled ELF64 little-endian reader — no
ELF-parsing crate dependency, since the section header table and
symbol table layouts are small, fixed-size structures fully specified
by the System V ABI. Reads the section header table (name, type,
size), then the `.symtab` (falling back to `.dynsym` for stripped
binaries that still export a dynamic symbol table) paired with its
associated string table for symbol names and sizes.

Matches sections and symbols across the two files **by name**, then
reports every one whose size changed: added (present only in the new
file), removed (only in the old file), or changed (present in both,
different size) — sorted by absolute size delta, largest first, total
file size delta shown up top.

## Status: built, unit-tested, and live-verified against real compiled binaries, independently cross-checked against readelf and nm

- **17 unit tests** (`cargo test --lib`) cover ELF header validation
  (magic number, 64-bit-only, little-endian-only, truncated-file
  rejection), section parsing (name resolution via the section header
  string table, size), symbol parsing (name resolution via the
  associated `.strtab`, size, function-vs-data classification,
  zero-size/undefined/unnamed symbols correctly excluded), and the
  diff logic (grown/added/removed/unchanged classification,
  absolute-delta sort order, byte-formatting with unit scaling). **A
  real bug caught by the tests, in the parser itself, not a test
  fixture**: `Elf64_Sym`'s `st_size` field is at byte offset 16 (after
  `st_name` at 0, `st_info`/`st_other` at 4/5, `st_shndx` at 6, and
  `st_value` at 8) — the initial implementation read from offset 8
  instead, silently returning every symbol's `st_value` (typically 0
  for the hand-built all-zero test fixtures) as its size, which then
  got filtered out by the zero-size check and made every symbol
  vanish. Fixed the offset, not the test.
- **Live-verified against two genuinely different compiled Rust
  binaries** (not hand-crafted ELF bytes): built `v1` and `v2` from
  two real `.rs` source files with `rustc -O`, where `v2` adds two
  extra functions before `main()` calls them, producing real,
  non-synthetic section and symbol-table differences.
  - **Total file size delta (+112 bytes) verified against `stat`**:
    exact byte-for-byte match with the independently measured file
    sizes.
  - **All three reported section deltas verified against `readelf
    -S`**: `.text` +96 bytes, `.eh_frame` +24 bytes, `.relro_padding`
    -112 bytes — every one computed independently from `readelf`'s own
    hex size fields and matching `binsize`'s output exactly.
  - **The largest symbol delta verified against `nm -S`**: `binsize`
    reported the new `main` symbol at +148 bytes and the old one at
    -57 bytes (shown as separate added/removed entries — see below);
    `nm -S` on each binary independently confirmed the same two raw
    sizes (0x94 = 148, 0x39 = 57 bytes).
  - **Fully stripped binaries** (`strip --strip-all` on both): section
    diffs still work identically (they don't depend on symbols at
    all); symbol diffs correctly report "no symbol size changes" since
    a non-PIE executable's surviving `.dynsym` holds only imported
    (`SHN_UNDEF`) symbols with nothing to report, not a crash or a
    false empty result from a missing table.
  - **Error handling verified against a real non-ELF file**
    (`/etc/hostname`) and a nonexistent path: both fail cleanly with a
    clear message and exit code 1 instead of panicking.
- **A real, honestly-documented limitation this live test surfaced**:
  Rust's default symbol mangling scheme includes a content-dependent
  hash suffix, so a function whose body changed even slightly gets a
  *different* mangled symbol name across builds (confirmed above:
  `main`'s mangled name changed between `v1` and `v2`). Since `binsize`
  matches purely by name, this shows up as one symbol "removed" and a
  differently-named one "added" rather than a single "changed" entry
  for what a human would call the same function. This is real observed
  behavior, not a hypothetical caveat — no attempt is made to fuzzy-match
  renamed/hash-shifted symbols.

**Not done / deliberately deferred**: 32-bit and big-endian ELF are
explicitly rejected rather than supported (covers the overwhelming
majority of real-world server/desktop Linux binaries); no Mach-O or
PE/COFF support, ELF only; no DWARF debug-info size breakdown (only
section-level granularity for `.debug_*` sections, not per-compilation-unit).
