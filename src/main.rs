use std::path::PathBuf;
use std::process::ExitCode;

use binsize::diff::{diff_sections, diff_symbols, format_bytes, format_delta};
use binsize::elf;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "binsize",
    about = "Diffs section and symbol sizes between two ELF binaries"
)]
struct Cli {
    old: PathBuf,
    new: PathBuf,
    /// How many top section/symbol deltas to show (by absolute size change).
    #[arg(long, default_value_t = 20)]
    top: usize,
}

fn load(path: &PathBuf) -> anyhow::Result<elf::ElfInfo> {
    let bytes = std::fs::read(path)?;
    elf::parse(&bytes)
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let old = match load(&cli.old) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("binsize: {}: {e}", cli.old.display());
            return ExitCode::FAILURE;
        }
    };
    let new = match load(&cli.new) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("binsize: {}: {e}", cli.new.display());
            return ExitCode::FAILURE;
        }
    };

    let file_delta = new.file_size as i64 - old.file_size as i64;
    println!(
        "File size: {} -> {} ({})",
        format_bytes(old.file_size),
        format_bytes(new.file_size),
        format_delta(file_delta)
    );

    let section_diffs = diff_sections(&old.sections, &new.sections);
    if section_diffs.is_empty() {
        println!("\nNo section size changes.");
    } else {
        println!(
            "\nSection changes (top {}, by |delta|):",
            cli.top.min(section_diffs.len())
        );
        for d in section_diffs.iter().take(cli.top) {
            println!(
                "  {:<24} {:>8} bytes  [{}]",
                d.name,
                format_delta(d.delta()),
                d.kind()
            );
        }
    }

    let symbol_diffs = diff_symbols(&old.symbols, &new.symbols);
    if symbol_diffs.is_empty() {
        println!("\nNo symbol size changes (or no symbol table found in one/both binaries).");
    } else {
        println!(
            "\nSymbol changes (top {}, by |delta|):",
            cli.top.min(symbol_diffs.len())
        );
        for d in symbol_diffs.iter().take(cli.top) {
            println!(
                "  {:<40} {:>8} bytes  [{}]",
                d.name,
                format_delta(d.delta()),
                d.kind()
            );
        }
    }

    ExitCode::SUCCESS
}
