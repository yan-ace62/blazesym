use crate::util::Pod;
use crate::SymType;

pub(crate) use goblin::elf::compression_header::compression_header64::CompressionHeader as Elf64_Chdr;
pub(crate) use goblin::elf::header::header64::Header as Elf64_Ehdr;
pub(crate) use goblin::elf::note::Nhdr32 as Elf_Nhdr;
pub(crate) use goblin::elf::program_header::program_header64::ProgramHeader as Elf64_Phdr;
pub(crate) use goblin::elf::section_header::section_header64::SectionHeader as Elf64_Shdr;
pub(crate) use goblin::elf::sym::sym64::Sym as Elf64_Sym;

pub(crate) use goblin::elf::compression_header::ELFCOMPRESS_ZLIB;
pub(crate) use goblin::elf::note::NT_GNU_BUILD_ID;
pub(crate) use goblin::elf::program_header::PT_LOAD;
pub(crate) use goblin::elf::section_header::SHF_COMPRESSED;
pub(crate) use goblin::elf::section_header::SHN_LORESERVE;
pub(crate) use goblin::elf::section_header::SHN_UNDEF;
pub(crate) use goblin::elf::section_header::SHN_XINDEX;
pub(crate) use goblin::elf::section_header::SHT_NOTE;

use goblin::elf::sym::st_type;
use goblin::elf::sym::STT_FUNC;
use goblin::elf::sym::STT_GNU_IFUNC;
use goblin::elf::sym::STT_OBJECT;

// SAFETY: `Elf64_Ehdr` is valid for any bit pattern.
unsafe impl Pod for Elf64_Ehdr {}
// SAFETY: `Elf64_Phdr` is valid for any bit pattern.
unsafe impl Pod for Elf64_Phdr {}
// SAFETY: `Elf64_Shdr` is valid for any bit pattern.
unsafe impl Pod for Elf64_Shdr {}
// SAFETY: `Elf64_Sym` is valid for any bit pattern.
unsafe impl Pod for Elf64_Sym {}
// SAFETY: `Elf64_Chdr` is valid for any bit pattern.
unsafe impl Pod for Elf64_Chdr {}
// SAFETY: `Elf_Nhdr` is valid for any bit pattern.
unsafe impl Pod for Elf_Nhdr {}

impl TryFrom<&Elf64_Sym> for SymType {
    type Error = ();

    fn try_from(other: &Elf64_Sym) -> Result<Self, Self::Error> {
        match st_type(other.st_info) {
            STT_FUNC | STT_GNU_IFUNC => Ok(SymType::Function),
            STT_OBJECT => Ok(SymType::Variable),
            _ => Err(()),
        }
    }
}

pub(crate) fn sym_matches(symbol: &Elf64_Sym, sym_type: SymType) -> bool {
    let elf_ty = st_type(symbol.st_info);
    let is_func = elf_ty == STT_FUNC || elf_ty == STT_GNU_IFUNC;
    let is_var = elf_ty == STT_OBJECT;

    match sym_type {
        SymType::Undefined => is_func || is_var,
        SymType::Function => is_func,
        SymType::Variable => is_var,
    }
}

pub(crate) const PN_XNUM: u16 = 0xffff;

/// zstd algorithm.
pub(crate) const ELFCOMPRESS_ZSTD: u32 = 2;

