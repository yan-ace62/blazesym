#![allow(clippy::let_and_return, clippy::let_unit_value)]

mod args;

use anyhow::Context;
use anyhow::Result;

use blazesym::normalize;
use blazesym::normalize::Normalizer;
use blazesym::symbolize;
use blazesym::symbolize::Symbolizer;

use clap::Parser as _;

use tracing::subscriber::set_global_default as set_global_subscriber;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::FmtSubscriber;


fn format_build_id_bytes(build_id: &[u8]) -> String {
    build_id
        .iter()
        .fold(String::with_capacity(build_id.len() * 2), |mut s, b| {
            let () = s.push_str(&format!("{b:02x}"));
            s
        })
}

fn format_build_id(build_id: Option<&[u8]>) -> String {
    if let Some(build_id) = build_id {
        format!(" (build ID: {})", format_build_id_bytes(build_id))
    } else {
        String::new()
    }
}

fn normalize(normalize: args::Normalize) -> Result<()> {
    let normalizer = Normalizer::new();
    match normalize {
        args::Normalize::User(args::User { pid, addrs }) => {
            let norm_addrs = normalizer
                .normalize_user_addrs(addrs.as_slice(), pid)
                .context("failed to normalize addresses")?;
            for (addr, (norm_addr, meta_idx)) in addrs.iter().zip(&norm_addrs.addrs) {
                print!("{addr:#016x}: ");

                let meta = &norm_addrs.meta[*meta_idx];
                match meta {
                    normalize::UserAddrMeta::ApkElf(normalize::ApkElf {
                        apk_path,
                        elf_path,
                        elf_build_id,
                        ..
                    }) => {
                        let build_id = format_build_id(elf_build_id.as_deref());
                        println!(
                            "{norm_addr:#x} @ {} in {}{build_id}",
                            elf_path.display(),
                            apk_path.display()
                        )
                    }
                    normalize::UserAddrMeta::Elf(normalize::Elf { path, build_id, .. }) => {
                        let build_id = format_build_id(build_id.as_deref());
                        println!("{norm_addr:#x} @ {}{build_id}", path.display())
                    }
                    normalize::UserAddrMeta::Unknown(normalize::Unknown { .. }) => {
                        println!("<unknown>")
                    }
                    // This is a bug and should be reported as such.
                    _ => panic!("encountered unsupported user address meta data: {meta:?}"),
                }
            }
        }
    }
    Ok(())
}

/// The handler for the 'symbolize' command.
fn symbolize(symbolize: args::Symbolize) -> Result<()> {
    let symbolizer = Symbolizer::new();
    let (src, addrs) = match symbolize {
        args::Symbolize::Elf(args::Elf { path, addrs }) => {
            let src = symbolize::Source::from(symbolize::Elf::new(path));
            (src, addrs)
        }
        args::Symbolize::Process(args::Process { pid, addrs }) => {
            let src = symbolize::Source::from(symbolize::Process::new(pid));
            (src, addrs)
        }
    };

    let syms = symbolizer
        .symbolize(&src, &addrs)
        .context("failed to symbolize addresses")?;

    let addr_width = 16;
    let mut prev_addr_idx = None;

    for (sym, addr_idx) in syms {
        if let Some(idx) = prev_addr_idx {
            // Print a line for all addresses that did not get symbolized.
            for input_addr in addrs.iter().take(addr_idx).skip(idx + 1) {
                println!("{input_addr:#0width$x}: <no-symbol>", width = addr_width)
            }
        }

        let symbolize::Sym {
            name,
            addr,
            offset,
            code_info,
            ..
        } = &sym;

        let src_loc = if let Some(code_info) = code_info {
            let path = code_info.to_path();
            let path = path.display();

            match (code_info.line, code_info.column) {
                (Some(line), Some(col)) => format!(" {path}:{line}:{col}"),
                (Some(line), None) => format!(" {path}:{line}"),
                (None, _) => format!(" {path}"),
            }
        } else {
            String::new()
        };

        if prev_addr_idx != Some(addr_idx) {
            // If the address index changed we reached a new symbol.
            println!(
                "{input_addr:#0width$x}: {name} @ {addr:#x}+{offset:#x}{src_loc}",
                input_addr = addrs[addr_idx],
                width = addr_width
            );
        } else {
            // Otherwise we are dealing with an inlined call.
            println!(
                "{:width$}  {name} @ {addr:#x}+{offset:#x}{src_loc}",
                " ",
                width = addr_width
            );
        }

        prev_addr_idx = Some(addr_idx);
    }
    Ok(())
}


fn main() -> Result<()> {
    let args = args::Args::parse();
    let level = match args.verbosity {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        2 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_span_events(FmtSpan::FULL)
        .with_timer(SystemTime)
        .finish();

    let () =
        set_global_subscriber(subscriber).with_context(|| "failed to set tracing subscriber")?;

    match args.command {
        args::Command::Normalize(normalize) => self::normalize(normalize),
        args::Command::Symbolize(symbolize) => self::symbolize(symbolize),
    }
}
