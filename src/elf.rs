//! Minimal, hand-rolled ELF64 (little-endian) reader: just enough of
//! the section header table and symbol table to report section and
//! symbol sizes. No dependency on an ELF-parsing crate — the header
//! layouts are small, fixed-size, and well documented in the System V
//! ABI spec.

use anyhow::{bail, Context, Result};

const EI_CLASS: usize = 4;
const EI_DATA: usize = 5;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;

const SHT_SYMTAB: u32 = 2;
const SHT_DYNSYM: u32 = 11;
const SHT_NOBITS: u32 = 8; // .bss — occupies no file bytes, but does occupy runtime memory

const STT_FUNC: u8 = 2;
const SHN_UNDEF: u16 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub sh_type: u32,
    /// In-memory/on-disk size. For SHT_NOBITS (.bss) this is a
    /// virtual size — it contributes nothing to file-on-disk size but
    /// is real runtime memory footprint, so it's reported separately.
    pub size: u64,
    pub is_nobits: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub size: u64,
    pub is_func: bool,
}

#[derive(Debug, Clone)]
pub struct ElfInfo {
    pub file_size: u64,
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
}

fn u16le(b: &[u8], off: usize) -> Result<u16> {
    let s = b
        .get(off..off + 2)
        .context("truncated ELF: u16 read past EOF")?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn u32le(b: &[u8], off: usize) -> Result<u32> {
    let s = b
        .get(off..off + 4)
        .context("truncated ELF: u32 read past EOF")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn u64le(b: &[u8], off: usize) -> Result<u64> {
    let s = b
        .get(off..off + 8)
        .context("truncated ELF: u64 read past EOF")?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn cstr_at(strtab: &[u8], offset: usize) -> String {
    let bytes = &strtab[offset.min(strtab.len())..];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

struct RawShdr {
    sh_name: u32,
    sh_type: u32,
    sh_offset: u64,
    sh_size: u64,
    sh_link: u32,
    sh_entsize: u64,
}

fn read_shdr(b: &[u8], off: usize) -> Result<RawShdr> {
    Ok(RawShdr {
        sh_name: u32le(b, off)?,
        sh_type: u32le(b, off + 4)?,
        sh_offset: u64le(b, off + 24)?,
        sh_size: u64le(b, off + 32)?,
        sh_link: u32le(b, off + 40)?,
        sh_entsize: u64le(b, off + 56)?,
    })
}

/// Parses an ELF64 little-endian file's section headers and symbol
/// table (preferring `.symtab`, falling back to `.dynsym` for stripped
/// binaries that still export a dynamic symbol table).
pub fn parse(bytes: &[u8]) -> Result<ElfInfo> {
    if bytes.len() < 64 {
        bail!("too small to be an ELF file ({} bytes)", bytes.len());
    }
    if &bytes[0..4] != b"\x7FELF" {
        bail!("not an ELF file (missing magic number)");
    }
    if bytes[EI_CLASS] != ELFCLASS64 {
        bail!("only 64-bit ELF is supported (this file is 32-bit)");
    }
    if bytes[EI_DATA] != ELFDATA2LSB {
        bail!("only little-endian ELF is supported (this file is big-endian)");
    }

    let e_shoff = u64le(bytes, 0x28)? as usize;
    let e_shentsize = u16le(bytes, 0x3A)? as usize;
    let e_shnum = u16le(bytes, 0x3C)? as usize;
    let e_shstrndx = u16le(bytes, 0x3E)? as usize;

    if e_shoff == 0 || e_shnum == 0 {
        bail!("ELF file has no section header table (likely stripped down to a bare executable segment layout, not supported)");
    }

    let mut raw_sections = Vec::with_capacity(e_shnum);
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        raw_sections
            .push(read_shdr(bytes, off).with_context(|| format!("reading section header {i}"))?);
    }

    let shstrtab_hdr = raw_sections
        .get(e_shstrndx)
        .context("e_shstrndx out of range")?;
    let shstrtab_start = shstrtab_hdr.sh_offset as usize;
    let shstrtab_end = shstrtab_start + shstrtab_hdr.sh_size as usize;
    let shstrtab = bytes
        .get(shstrtab_start..shstrtab_end)
        .context("section header string table out of range")?;

    let sections: Vec<Section> = raw_sections
        .iter()
        .map(|s| Section {
            name: cstr_at(shstrtab, s.sh_name as usize),
            sh_type: s.sh_type,
            size: s.sh_size,
            is_nobits: s.sh_type == SHT_NOBITS,
        })
        .collect();

    let symtab_idx = raw_sections
        .iter()
        .position(|s| s.sh_type == SHT_SYMTAB)
        .or_else(|| raw_sections.iter().position(|s| s.sh_type == SHT_DYNSYM));

    let mut symbols = Vec::new();
    if let Some(idx) = symtab_idx {
        let symtab = &raw_sections[idx];
        let strtab_hdr = raw_sections
            .get(symtab.sh_link as usize)
            .context("symtab sh_link out of range")?;
        let strtab_start = strtab_hdr.sh_offset as usize;
        let strtab_end = strtab_start + strtab_hdr.sh_size as usize;
        let strtab = bytes
            .get(strtab_start..strtab_end)
            .context("symbol string table out of range")?;

        let entsize = if symtab.sh_entsize == 0 {
            24
        } else {
            symtab.sh_entsize as usize
        };
        let sym_start = symtab.sh_offset as usize;
        let sym_end = sym_start + symtab.sh_size as usize;
        let sym_bytes = bytes
            .get(sym_start..sym_end)
            .context("symbol table out of range")?;

        let count = sym_bytes.len() / entsize;
        for i in 0..count {
            let off = i * entsize;
            let st_name = u32le(sym_bytes, off)?;
            let st_info = sym_bytes[off + 4];
            let st_shndx = u16le(sym_bytes, off + 6)?;
            let st_size = u64le(sym_bytes, off + 16)?; // st_value is at +8, st_size at +16

            if st_shndx == SHN_UNDEF || st_name == 0 || st_size == 0 {
                continue; // undefined/external, unnamed, or zero-size symbols aren't useful for a size diff
            }
            let symbol_type = st_info & 0xf;
            symbols.push(Symbol {
                name: cstr_at(strtab, st_name as usize),
                size: st_size,
                is_func: symbol_type == STT_FUNC,
            });
        }
    }

    Ok(ElfInfo {
        file_size: bytes.len() as u64,
        sections,
        symbols,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad_to(buf: &mut Vec<u8>, offset: usize) {
        if buf.len() < offset {
            buf.resize(offset, 0);
        }
    }

    /// Builds a minimal, valid, hand-assembled ELF64-LE file with one
    /// named section (besides the mandatory null and shstrtab
    /// sections) and, optionally, a symtab+strtab pair.
    fn build_minimal_elf(
        section_name: &str,
        section_size: u64,
        symbols: &[(&str, u64, bool)],
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 64]; // ELF header, filled below
        buf[0..4].copy_from_slice(b"\x7FELF");
        buf[EI_CLASS] = ELFCLASS64;
        buf[EI_DATA] = ELFDATA2LSB;

        // Section header string table content: "\0.shstrtab\0<section_name>\0"
        let mut shstrtab_data = vec![0u8];
        let shstrtab_name_off = shstrtab_data.len();
        shstrtab_data.extend_from_slice(b".shstrtab\0");
        let section_name_off = shstrtab_data.len();
        shstrtab_data.extend_from_slice(section_name.as_bytes());
        shstrtab_data.push(0);
        let strtab_name_off = shstrtab_data.len();
        shstrtab_data.extend_from_slice(b".strtab\0");
        let symtab_name_off = shstrtab_data.len();
        shstrtab_data.extend_from_slice(b".symtab\0");

        let shstrtab_file_off = 64usize;
        buf.extend_from_slice(&shstrtab_data);
        pad_to(&mut buf, shstrtab_file_off + shstrtab_data.len());

        // .strtab content (symbol names): "\0name1\0name2\0..."
        let mut strtab_data = vec![0u8];
        let mut symbol_name_offsets = Vec::new();
        for (name, _, _) in symbols {
            symbol_name_offsets.push(strtab_data.len());
            strtab_data.extend_from_slice(name.as_bytes());
            strtab_data.push(0);
        }
        let strtab_file_off = buf.len();
        buf.extend_from_slice(&strtab_data);

        // .symtab content: one all-zero null entry, then real entries (24 bytes each)
        let symtab_file_off = buf.len();
        buf.extend_from_slice(&[0u8; 24]); // mandatory STN_UNDEF null symbol
        for (i, (_, size, is_func)) in symbols.iter().enumerate() {
            let mut entry = [0u8; 24];
            entry[0..4].copy_from_slice(&(symbol_name_offsets[i] as u32).to_le_bytes());
            entry[4] = if *is_func { STT_FUNC } else { 1 };
            entry[6..8].copy_from_slice(&1u16.to_le_bytes()); // st_shndx = 1 (defined, not SHN_UNDEF)
            entry[8..16].copy_from_slice(&0u64.to_le_bytes()); // st_value
            entry[16..24].copy_from_slice(&size.to_le_bytes());
            buf.extend_from_slice(&entry);
        }
        let symtab_size = 24 + symbols.len() * 24;

        // Section headers: [0]=NULL, [1]=<section_name>, [2]=.shstrtab, [3]=.strtab, [4]=.symtab
        let shoff = buf.len();
        let mut shdrs = Vec::new();
        shdrs.push([0u8; 64]); // NULL section

        let mut real_section = [0u8; 64];
        real_section[0..4].copy_from_slice(&(section_name_off as u32).to_le_bytes());
        real_section[4..8].copy_from_slice(&1u32.to_le_bytes()); // SHT_PROGBITS
        real_section[32..40].copy_from_slice(&section_size.to_le_bytes());
        shdrs.push(real_section);

        let mut shstrtab_hdr = [0u8; 64];
        shstrtab_hdr[0..4].copy_from_slice(&(shstrtab_name_off as u32).to_le_bytes());
        shstrtab_hdr[4..8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
        shstrtab_hdr[24..32].copy_from_slice(&(shstrtab_file_off as u64).to_le_bytes());
        shstrtab_hdr[32..40].copy_from_slice(&(shstrtab_data.len() as u64).to_le_bytes());
        shdrs.push(shstrtab_hdr);

        let mut strtab_hdr = [0u8; 64];
        strtab_hdr[0..4].copy_from_slice(&(strtab_name_off as u32).to_le_bytes());
        strtab_hdr[4..8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
        strtab_hdr[24..32].copy_from_slice(&(strtab_file_off as u64).to_le_bytes());
        strtab_hdr[32..40].copy_from_slice(&(strtab_data.len() as u64).to_le_bytes());
        shdrs.push(strtab_hdr);

        let mut symtab_hdr = [0u8; 64];
        symtab_hdr[0..4].copy_from_slice(&(symtab_name_off as u32).to_le_bytes());
        symtab_hdr[4..8].copy_from_slice(&2u32.to_le_bytes()); // SHT_SYMTAB
        symtab_hdr[24..32].copy_from_slice(&(symtab_file_off as u64).to_le_bytes());
        symtab_hdr[32..40].copy_from_slice(&(symtab_size as u64).to_le_bytes());
        symtab_hdr[40..44].copy_from_slice(&3u32.to_le_bytes()); // sh_link -> .strtab (index 3)
        symtab_hdr[56..64].copy_from_slice(&24u64.to_le_bytes()); // sh_entsize
        shdrs.push(symtab_hdr);

        for shdr in &shdrs {
            buf.extend_from_slice(shdr);
        }

        // Fill in e_shoff, e_shentsize, e_shnum, e_shstrndx in the ELF header
        buf[0x28..0x30].copy_from_slice(&(shoff as u64).to_le_bytes());
        buf[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes());
        buf[0x3C..0x3E].copy_from_slice(&(shdrs.len() as u16).to_le_bytes());
        buf[0x3E..0x40].copy_from_slice(&2u16.to_le_bytes()); // shstrtab is section index 2

        buf
    }

    #[test]
    fn rejects_too_small_file() {
        assert!(parse(&[0u8; 10]).is_err());
    }

    #[test]
    fn rejects_missing_magic() {
        let mut bytes = vec![0u8; 64];
        bytes[0..4].copy_from_slice(b"nope");
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn rejects_32_bit_elf() {
        let mut bytes = build_minimal_elf(".text", 100, &[]);
        bytes[EI_CLASS] = 1; // ELFCLASS32
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn rejects_big_endian_elf() {
        let mut bytes = build_minimal_elf(".text", 100, &[]);
        bytes[EI_DATA] = 2; // ELFDATA2MSB
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn parses_section_name_and_size() {
        let bytes = build_minimal_elf(".text", 4096, &[]);
        let info = parse(&bytes).unwrap();
        let text = info.sections.iter().find(|s| s.name == ".text").unwrap();
        assert_eq!(text.size, 4096);
        assert!(!text.is_nobits);
    }

    #[test]
    fn parses_symbols_with_names_and_sizes() {
        let bytes = build_minimal_elf(
            ".text",
            4096,
            &[("my_function", 128, true), ("my_data", 8, false)],
        );
        let info = parse(&bytes).unwrap();
        assert_eq!(info.symbols.len(), 2);
        let func = info
            .symbols
            .iter()
            .find(|s| s.name == "my_function")
            .unwrap();
        assert_eq!(func.size, 128);
        assert!(func.is_func);
        let data = info.symbols.iter().find(|s| s.name == "my_data").unwrap();
        assert_eq!(data.size, 8);
        assert!(!data.is_func);
    }

    #[test]
    fn file_size_matches_actual_byte_length() {
        let bytes = build_minimal_elf(".text", 100, &[]);
        let info = parse(&bytes).unwrap();
        assert_eq!(info.file_size, bytes.len() as u64);
    }

    #[test]
    fn zero_size_symbols_are_skipped() {
        let bytes = build_minimal_elf(
            ".text",
            100,
            &[("zero_sized", 0, true), ("real_one", 10, true)],
        );
        let info = parse(&bytes).unwrap();
        assert_eq!(info.symbols.len(), 1);
        assert_eq!(info.symbols[0].name, "real_one");
    }
}
