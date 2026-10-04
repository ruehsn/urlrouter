//! Embeds assets/urlrouter.ico in the Windows .exe as its application icon,
//! which Explorer, the taskbar and Settings → Default apps all show.
//!
//! No build dependencies: the Windows resource (.res) file is assembled here
//! from the .ico. MSVC's linker takes a .res directly. GNU ld can't, so for
//! MinGW builds windres (part of MinGW's binutils) converts it to an object
//! first; if windres is missing the build carries on without an icon rather
//! than failing.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ICO: &str = "assets/urlrouter.ico";
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409;

fn main() {
    println!("cargo:rerun-if-changed={ICO}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let ico = fs::read(ICO).unwrap_or_else(|e| panic!("reading {ICO}: {e}"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let res = out.join("urlrouter.res");
    fs::write(&res, ico_to_res(&ico)).expect("writing the .res file");

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins={}", res.display());
        return;
    }
    let obj = out.join("urlrouter-res.o");
    let windres = env::var("WINDRES").unwrap_or_else(|_| default_windres());
    match windres_to_coff(&windres, &res, &obj) {
        Ok(()) => println!("cargo:rustc-link-arg-bins={}", obj.display()),
        Err(e) => println!("cargo:warning=building without an icon: {e}"),
    }
}

/// Cross-compiling uses the target-prefixed windres, as MinGW installs it.
fn default_windres() -> String {
    let host = env::var("HOST").unwrap_or_default();
    if host.contains("windows") {
        "windres".into()
    } else {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".into());
        format!("{arch}-w64-mingw32-windres")
    }
}

fn windres_to_coff(windres: &str, res: &Path, obj: &Path) -> Result<(), String> {
    let status = Command::new(windres)
        .args(["-J", "res", "-O", "coff", "-i"])
        .arg(res)
        .arg("-o")
        .arg(obj)
        .status()
        .map_err(|e| format!("could not run {windres}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{windres} failed ({status})"))
    }
}

/// Converts an .ico file into a .res file holding one RT_ICON per image plus
/// the RT_GROUP_ICON (ID 1) that ties them together, the layout rc.exe and
/// windres produce for `1 ICON "file.ico"`.
fn ico_to_res(ico: &[u8]) -> Vec<u8> {
    let u16_at = |i: usize| u16::from_le_bytes([ico[i], ico[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([ico[i], ico[i + 1], ico[i + 2], ico[i + 3]]);
    assert!(
        ico.len() >= 6 && u16_at(0) == 0 && u16_at(2) == 1,
        "{ICO} is not an .ico file"
    );
    let count = u16_at(4);

    let mut res = Vec::new();
    // Every 32-bit .res file starts with this empty entry.
    push_resource(&mut res, 0, 0, 0, 0, &[]);

    // GRPICONDIR: reserved, type 1 (icon), count; then one entry per image.
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend(count.to_le_bytes());
    for i in 0..count {
        let entry = 6 + 16 * i as usize;
        let size = u32_at(entry + 8) as usize;
        let offset = u32_at(entry + 12) as usize;
        let id = i + 1;
        // MOVEABLE | DISCARDABLE, as rc.exe marks icon images.
        push_resource(
            &mut res,
            RT_ICON,
            id,
            0x1010,
            LANG_EN_US,
            &ico[offset..offset + size],
        );
        // Same 12 leading bytes as the ICONDIRENTRY (width, height, colours,
        // reserved, planes, bit count, size); the file offset becomes the ID.
        group.extend(&ico[entry..entry + 12]);
        group.extend(id.to_le_bytes());
    }
    // MOVEABLE | PURE | DISCARDABLE.
    push_resource(&mut res, RT_GROUP_ICON, 1, 0x1030, LANG_EN_US, &group);
    res
}

/// Appends one resource with a numeric type and ID: a 32-byte header, then
/// the data padded to a 4-byte boundary.
fn push_resource(res: &mut Vec<u8>, kind: u16, id: u16, flags: u16, lang: u16, data: &[u8]) {
    res.extend((data.len() as u32).to_le_bytes()); // DataSize
    res.extend(32u32.to_le_bytes()); // HeaderSize
    for word in [0xFFFF, kind, 0xFFFF, id] {
        res.extend(word.to_le_bytes()); // TYPE and NAME as ordinals
    }
    res.extend(0u32.to_le_bytes()); // DataVersion
    res.extend(flags.to_le_bytes()); // MemoryFlags
    res.extend(lang.to_le_bytes()); // LanguageId
    res.extend(0u32.to_le_bytes()); // Version
    res.extend(0u32.to_le_bytes()); // Characteristics
    res.extend(data);
    res.resize(res.len().next_multiple_of(4), 0);
}
