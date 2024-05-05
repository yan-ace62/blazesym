#![allow(clippy::let_unit_value)]

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::create_dir_all;
use std::fs::hard_link;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Result;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

fn crate_root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
}

fn data_dir() -> PathBuf {
    crate_root().join("data")
}

/// Retrieve the system's page size.
fn page_size() -> Result<usize> {
    // SAFETY: `sysconf` is always safe to call.
    let rc = unsafe { libc::sysconf(libc::_SC_PAGE_SIZE) };
    if rc < 0 {
        return Err(Error::new(
            ErrorKind::Other,
            format!("failed to retrieve page size: {}", Error::last_os_error()),
        ))
    }
    Ok(usize::try_from(rc).unwrap())
}


/// Format a command with the given list of arguments as a string.
fn format_command<C, A, S>(command: C, args: A) -> String
where
    C: AsRef<OsStr>,
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter().fold(
        command.as_ref().to_string_lossy().into_owned(),
        |mut cmd, arg| {
            cmd += " ";
            cmd += arg.as_ref().to_string_lossy().deref();
            cmd
        },
    )
}

/// Run a command with the provided arguments.
fn run<C, A, S>(command: C, args: A) -> Result<()>
where
    C: AsRef<OsStr>,
    A: IntoIterator<Item = S> + Clone,
    S: AsRef<OsStr>,
{
    let instance = Command::new(command.as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .env_clear()
        .envs(env::vars().filter(|(k, _)| k == "PATH"))
        .args(args.clone())
        .output()
        .map_err(|err| {
            Error::new(
                ErrorKind::Other,
                format!(
                    "failed to run `{}`: {err}",
                    format_command(command.as_ref(), args.clone())
                ),
            )
        })?;

    if !instance.status.success() {
        let code = if let Some(code) = instance.status.code() {
            format!(" ({code})")
        } else {
            " (terminated by signal)".to_string()
        };

        let stderr = String::from_utf8_lossy(&instance.stderr);
        let stderr = stderr.trim_end();
        let stderr = if !stderr.is_empty() {
            format!(": {stderr}")
        } else {
            String::new()
        };

        Err(Error::new(
            ErrorKind::Other,
            format!(
                "`{}` reported non-zero exit-status{code}{stderr}",
                format_command(command, args)
            ),
        ))
    } else {
        Ok(())
    }
}

fn adjust_mtime(path: &Path) -> Result<()> {
    // Note that `OUT_DIR` is only present at runtime.
    let out_dir = env::var("OUT_DIR").unwrap();
    // The $OUT_DIR/output file is (in current versions of Cargo [as of
    // 1.69]) the file containing the reference time stamp that Cargo
    // checks to determine whether something is considered outdated and
    // in need to be rebuild. It's an implementation detail, yes, but we
    // don't rely on it for anything essential.
    let output = Path::new(&out_dir)
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::Other, "OUT_DIR has no parent"))?
        .join("output");

    if !output.exists() {
        // The file may not exist for legitimate reasons, e.g., when we
        // build for the very first time. If there is not reference there
        // is nothing for us to do, so just bail.
        return Ok(())
    }

    let () = run(
        "touch",
        [
            "-m".as_ref(),
            "--reference".as_ref(),
            output.as_os_str(),
            path.as_os_str(),
        ],
    )?;
    Ok(())
}

enum ArgSpec {
    SrcDashODst,
}

/// Invoke `tool` to convert `src` into `dst`.
fn toolize_impl(
    tool: &str,
    arg_spec: ArgSpec,
    src: &Path,
    dst: impl AsRef<OsStr>,
    options: &[&str],
) {
    let dst = dst.as_ref();
    let dst = src.with_file_name(dst);
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", dst.display());

    let args3;
    let args = match arg_spec {
        ArgSpec::SrcDashODst => {
            args3 = [src.as_os_str(), "-o".as_ref(), dst.as_os_str()];
            args3.as_slice()
        }
    };

    let () = run(
        tool,
        options
            .iter()
            .map(OsStr::new)
            .chain(args.iter().map(Deref::deref)),
    )
    .unwrap_or_else(|err| panic!("failed to run `{tool}`: {err}"));

    let () = adjust_mtime(&dst).unwrap();
}

fn toolize_o(tool: &str, src: &Path, dst: impl AsRef<OsStr>, options: &[&str]) {
    toolize_impl(tool, ArgSpec::SrcDashODst, src, dst, options)
}

/// Compile `src` into `dst` using `cc`.
fn cc(src: &Path, dst: impl AsRef<OsStr>, options: &[&str]) {
    toolize_o("cc", src, dst, options)
}

/// Compile `src` into `dst` using `rustc`.
fn rustc(src: &Path, dst: impl AsRef<OsStr>, options: &[&str]) {
    toolize_o("rustc", src, dst, options)
}

/// Convert debug information contained in `src` into GSYM in `dst` using
/// `llvm-gsymutil`.
fn gsym(src: &Path, dst: impl AsRef<OsStr>) {
    let dst = src.with_file_name(dst);
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", dst.display());
    println!("cargo:rerun-if-env-changed=LLVM_GSYMUTIL");

    let gsymutil = env::var_os("LLVM_GSYMUTIL").unwrap_or_else(|| OsString::from("llvm-gsymutil"));

    let () = run(
        gsymutil,
        ["--convert".as_ref(), src, "--out-file".as_ref(), &dst],
    )
    .expect("failed to run `llvm-gsymutil`");

    let () = adjust_mtime(&dst).unwrap();
}

/// Invoke `strip` on a copy of `src` placed at `dst`.
fn strip(src: &Path, dst: impl AsRef<OsStr>, options: &[&str]) {
    toolize_o("strip", src, dst, options)
}

/// Strip all DWARF information from an ELF binary, in an attempt to
/// leave only ELF symbols in place.
fn elf(src: &Path, dst: impl AsRef<OsStr>) {
    strip(src, dst, &["--strip-debug"])
}

/// Strip all non-debug information from an ELF binary, in an attempt to
/// leave only DWARF remains.
fn dwarf(src: &Path, dst: impl AsRef<OsStr>) {
    strip(src, dst, &["--keep-section=.debug_*"])
}

/// Generate a Breakpad .sym file for the given source.
#[cfg(feature = "dump_syms")]
fn syms(src: &Path, dst: impl AsRef<OsStr>) {
    use dump_syms::dumper;
    use dump_syms::dumper::Config;
    use dump_syms::dumper::FileOutput;
    use dump_syms::dumper::Output;

    let dst = src.with_file_name(dst);

    let mut config = Config::with_output(Output::File(FileOutput::Path(dst)));
    config.check_cfi = false;

    let path = src.to_str().unwrap();
    let () = dumper::single_file(&config, path).unwrap();
}

#[cfg(not(feature = "dump_syms"))]
fn syms(_src: &Path, _dst: impl AsRef<OsStr>) {
    unimplemented!()
}

/// Unpack an xz compressed file.
#[cfg(feature = "xz2")]
fn unpack_xz(src: &Path, dst: &Path) {
    use std::fs::File;
    use std::io::copy;
    use xz2::read::XzDecoder;

    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", dst.display());

    let src_file = File::options().create(false).read(true).open(src).unwrap();
    let mut decoder = XzDecoder::new_multi_decoder(src_file);

    let mut dst_file = File::options()
        .create(true)
        .truncate(true)
        .read(false)
        .write(true)
        .open(dst)
        .unwrap();

    let _bytes = copy(&mut decoder, &mut dst_file).unwrap();
    let () = adjust_mtime(dst).unwrap();
}

#[cfg(not(feature = "xz2"))]
fn unpack_xz(_src: &Path, _dst: &Path) {
    unimplemented!()
}


/// Put files in a zip archive, uncompressed.
#[cfg(feature = "zip")]
fn zip(files: &[PathBuf], dst: &Path) {
    use std::fs::read as read_file;
    use std::fs::File;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    use zip::ZipWriter;

    {
        let dst_file = File::options()
            .create(true)
            .truncate(true)
            .read(false)
            .write(true)
            .open(dst)
            .unwrap();
        let dst_dir = dst.parent().unwrap();

        let page_size = page_size().unwrap();
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            // Ensure that members are page aligned so that they can be
            // mmap'ed directly.
            .with_alignment(page_size.try_into().unwrap());
        let mut zip = ZipWriter::new(dst_file);
        for file in files {
            let contents = read_file(file).unwrap();
            let path = file.strip_prefix(dst_dir).unwrap();
            let () = zip.start_file(path.to_str().unwrap(), options).unwrap();
            let _count = zip.write(&contents).unwrap();
        }
    }

    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    println!("cargo:rerun-if-changed={}", dst.display());
    let () = adjust_mtime(dst).unwrap();
}

#[cfg(not(feature = "zip"))]
fn zip(_files: &[PathBuf], _dst: &Path) {
    let _page_size = page_size();
    unimplemented!()
}


fn cc_stable_addrs(dst: impl AsRef<OsStr>, options: &[&str]) {
    let data_dir = data_dir();
    let src = data_dir.join("test-stable-addrs.c");
    let src_cu2 = data_dir.join("test-stable-addrs-cu2.c");
    let ld_script = data_dir.join("test-stable-addrs.ld");
    println!("cargo:rerun-if-changed={}", ld_script.display());
    println!("cargo:rerun-if-changed={}", src_cu2.display());

    let args = [
        src_cu2.to_str().unwrap(),
        "-T",
        ld_script.to_str().unwrap(),
        "-O0",
        "-nostdlib",
    ]
    .into_iter()
    .chain(options.iter().map(Deref::deref))
    .collect::<Vec<_>>();

    cc(&src, dst, &args)
}

/// Prepare the various test files.
fn prepare_test_files() {
    let data_dir = data_dir();
    let src = data_dir.join("test.rs");
    rustc(
        &src,
        "test-rs.bin",
        &[
            "--crate-type=bin",
            "-C",
            "panic=abort",
            "-C",
            "link-arg=-nostartfiles",
            "-C",
            "opt-level=0",
            "-C",
            "debuginfo=2",
            // Note that despite us specifying the name mangling scheme
            // here, because we want a stable mangled name the source
            // actually uses a fixed "export name", which really is what
            // is used for the function in question.
            "-C",
            "symbol-mangling-version=v0",
        ],
    );

    let src = data_dir.join("test-so.c");
    cc(
        &src,
        "libtest-so.so",
        &["-shared", "-fPIC", "-m32", "-Wl,--build-id=sha1"],
    );
    cc(
        &src,
        "libtest-so-no-separate-code.so",
        &["-shared", "-fPIC", "-Wl,--build-id=md5,-z,noseparate-code"],
    );
    let src = data_dir.join("libtest-so.so");
    gsym(&src, "libtest-so.gsym");
    strip(&src, "libtest-so-stripped.so", &[]);
    strip(
        &src,
        "libtest-so-partly-stripped.so",
        &["--keep-symbol=the_ignored_answer"],
    );

    let src = data_dir.join("test-exe.c");
    cc(&src, "test-no-debug.bin", &["-g0", "-Wl,--build-id=none"]);
    cc(&src, "test-dwarf-v2.bin", &["-gstrict-dwarf", "-gdwarf-2"]);
    cc(&src, "test-dwarf-v3.bin", &["-gstrict-dwarf", "-gdwarf-3"]);
    cc(&src, "test-dwarf-v4.bin", &["-gstrict-dwarf", "-gdwarf-4"]);
    cc(&src, "test-dwarf-v5.bin", &["-gstrict-dwarf", "-gdwarf-5"]);
    cc(
        &src,
        "test-dwarf-v5-zlib.bin",
        &["-gstrict-dwarf", "-gdwarf-5", "-gz=zlib"],
    );
    if cfg!(feature = "zstd") {
        cc(
            &src,
            "test-dwarf-v5-zstd.bin",
            &["-gstrict-dwarf", "-gdwarf-5", "-gz=zstd"],
        );
    }

    let src = data_dir.join("test-wait.c");
    cc(&src, "test-wait.bin", &[]);

    let src = data_dir.join("test-mnt-ns.c");
    cc(&src, "test-mnt-ns.bin", &[]);

    cc_stable_addrs(
        "test-stable-addrs.bin",
        &["-gdwarf-4", "-Wl,--build-id=none", "-O0"],
    );
    cc_stable_addrs(
        "test-stable-addrs-compressed-debug-zlib.bin",
        &["-gdwarf-4", "-Wl,--build-id=none", "-O0", "-gz=zlib"],
    );
    if cfg!(feature = "zstd") {
        cc_stable_addrs(
            "test-stable-addrs-compressed-debug-zstd.bin",
            &["-gdwarf-4", "-Wl,--build-id=none", "-gz=zstd"],
        );
    }
    cc_stable_addrs(
        "test-stable-addrs-no-dwarf.bin",
        &["-g0", "-Wl,--build-id=none"],
    );
    cc_stable_addrs(
        "test-stable-addrs-lto.bin",
        &[
            // NB: Keep DWARF 4 for this binary. Cross unit references
            //     as this binary aims to produce only seem to appear in
            //     this version.
            "-gdwarf-4",
            "-flto",
        ],
    );

    let src = data_dir.join("test-stable-addrs.bin");
    gsym(&src, "test-stable-addrs.gsym");
    dwarf(&src, "test-stable-addrs-stripped-elf-with-dwarf.bin");
    strip(&src, "test-stable-addrs-stripped.bin", &[]);
    if cfg!(feature = "dump_syms") {
        syms(&src, "test-stable-addrs.sym");
    }

    let src = data_dir.join("kallsyms.xz");
    let mut dst = src.clone();
    assert!(dst.set_extension(""));
    unpack_xz(&src, &dst);

    let () = create_dir_all(data_dir.join("zip-dir")).unwrap();
    let () = hard_link(
        data_dir.join("test-no-debug.bin"),
        data_dir.join("zip-dir").join("test-no-debug.bin"),
    )
    .or_else(|err| {
        if err.kind() == ErrorKind::AlreadyExists {
            Ok(())
        } else {
            Err(err)
        }
    })
    .unwrap();

    let files = [
        data_dir.join("test-stable-addrs-stripped-elf-with-dwarf.bin"),
        data_dir.join("zip-dir").join("test-no-debug.bin"),
        data_dir.join("libtest-so.so"),
        data_dir.join("libtest-so-no-separate-code.so"),
    ];
    let dst = data_dir.join("test.zip");
    zip(files.as_slice(), &dst);
}

/// Download a multi-part file split into `part_count` pieces.
#[cfg(feature = "reqwest")]
fn download_multi_part(base_url: &reqwest::Url, part_count: usize, dst: &Path) {
    use std::fs::File;
    use std::io::Write as _;

    let mut dst = File::create(dst).unwrap();
    for part in 1..=part_count {
        let url = reqwest::Url::parse(&format!("{}.part{part}", base_url.as_str())).unwrap();
        let response = reqwest::blocking::get(url).unwrap();
        let _count = dst.write(&response.bytes().unwrap()).unwrap();
    }
}

/// Download large benchmark related files for later use.
#[cfg(feature = "reqwest")]
fn download_bench_files() {
    use reqwest::Url;

    let large_file_url =
        Url::parse("https://github.com/danielocfb/blazesym-data/raw/main/").unwrap();
    let file = "vmlinux-5.17.12-100.fc34.x86_64.xz";
    let dst = data_dir().join(file);
    let () = download_multi_part(&large_file_url.join(file).unwrap(), 2, &dst);
    let () = adjust_mtime(&dst).unwrap();
}

#[cfg(not(feature = "reqwest"))]
fn download_bench_files() {
    unimplemented!()
}

/// Prepare benchmark files.
fn prepare_bench_files() {
    let vmlinux_xz = data_dir().join("vmlinux-5.17.12-100.fc34.x86_64.xz");

    let mut vmlinux = vmlinux_xz.clone();
    assert!(vmlinux.set_extension(""));
    unpack_xz(&vmlinux_xz, &vmlinux);

    let mut dst = vmlinux_xz.clone();
    assert!(dst.set_extension("elf"));
    let dst = dst.file_name().unwrap();
    elf(&vmlinux, dst);

    let mut dst = vmlinux_xz.clone();
    assert!(dst.set_extension("gsym"));
    let dst = dst.file_name().unwrap();
    gsym(&vmlinux, dst);

    let mut dst = vmlinux_xz.clone();
    assert!(dst.set_extension("dwarf"));
    let dst = dst.file_name().unwrap();
    dwarf(&vmlinux, dst);

    let mut dst = vmlinux_xz;
    assert!(dst.set_extension("sym"));
    let dst = dst.file_name().unwrap();
    syms(&vmlinux, dst);
}

fn main() {
    if cfg!(feature = "generate-unit-test-files")
        && !cfg!(feature = "dont-generate-unit-test-files")
    {
        prepare_test_files();
    }

    if cfg!(feature = "generate-large-test-files") {
        download_bench_files();
        prepare_bench_files();
    }
}
