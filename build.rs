//! Downloads the official Stockfish release for the target platform, verifies its checksum and
//! extracts the engine binary into OUT_DIR, where `src/bundled.rs` embeds it.
//!
//! Set `STOCKFISH_ARCHIVE=<path>` to use an already downloaded release archive (offline builds).

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

const TAG: &str = "sf_19";
const BASE_URL: &str = "https://github.com/official-stockfish/Stockfish/releases/download";

struct Asset {
    /// Archive file name without extension; also the binary's name inside the archive.
    stem: &'static str,
    zip: bool,
    sha256: &'static str,
}

fn asset_for(os: &str, arch: &str, target_env: &str) -> Option<Asset> {
    let (stem, zip, sha256) = match (os, arch, target_env) {
        ("windows", "x86_64", _) => (
            "stockfish-windows-x86-64-universal",
            true,
            "3c8bf1f9ea66a09350a40df4f632288285ac206d99f33ab5842c408fc30b48a7",
        ),
        ("windows", "aarch64", _) => (
            "stockfish-windows-arm64-universal",
            true,
            "8372ad3f0d7276deb2c70f801f541ec7db463219fc6d9c7592864e542aa4f401",
        ),
        ("linux", "x86_64", "gnu") => (
            "stockfish-linux-x86-64-universal",
            false,
            "9defc0d4e55d49c65a6d042f3e571a39fcea499ade6dbe741b53b8c65e03611f",
        ),
        ("linux", "aarch64", "gnu") => (
            "stockfish-linux-arm64-universal",
            false,
            "fe26cfd1d9db4c8af3d21e24d9ff34cacb31c1f940085a7583da11796f2bac01",
        ),
        ("macos", "x86_64" | "aarch64", _) => (
            "stockfish-macos-universal",
            false,
            "a1f0e3bcc5a6927a11fe6fc8e54a779754645f3c2bae2cf13420fd1957adaa77",
        ),
        _ => return None,
    };
    Some(Asset { stem, zip, sha256 })
}

impl Asset {
    fn file_name(&self) -> String {
        let ext = if self.zip { "zip" } else { "tar.gz" };
        format!("{}.{ext}", self.stem)
    }

    fn entry_name(&self) -> String {
        let exe = if self.zip { ".exe" } else { "" };
        format!("stockfish/{}{exe}", self.stem)
    }
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=STOCKFISH_ARCHIVE");
    println!("cargo::rustc-check-cfg=cfg(bundled_stockfish)");

    let var = |name: &str| env::var(name).unwrap_or_default();
    let (os, arch, target_env) = (
        var("CARGO_CFG_TARGET_OS"),
        var("CARGO_CFG_TARGET_ARCH"),
        var("CARGO_CFG_TARGET_ENV"),
    );
    let Some(asset) = asset_for(&os, &arch, &target_env) else {
        println!(
            "cargo::warning=No Stockfish build for {arch}-{os}-{target_env}; only the built-in engine is available"
        );
        return;
    };

    let archive = load_archive(&asset).unwrap_or_else(|e| {
        panic!(
            "Could not get Stockfish {TAG} ({}): {e}\n\
             Download it manually and set STOCKFISH_ARCHIVE=<path to the archive> to build offline.",
            asset.file_name()
        )
    });
    let actual = sha256_hex(&archive);
    if actual != asset.sha256 {
        panic!(
            "Checksum mismatch for {}: expected {}, got {actual}",
            asset.file_name(),
            asset.sha256
        );
    }

    let binary = extract(&asset, &archive).unwrap_or_else(|e| {
        panic!(
            "Could not extract {} from {}: {e}",
            asset.entry_name(),
            asset.file_name()
        )
    });
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    fs::write(Path::new(&out_dir).join("stockfish.bin"), binary).expect("write stockfish.bin");

    println!("cargo::rustc-cfg=bundled_stockfish");
    println!("cargo::rustc-env=TUICHESS_STOCKFISH_TAG={TAG}");
}

fn load_archive(asset: &Asset) -> io::Result<Vec<u8>> {
    if let Ok(path) = env::var("STOCKFISH_ARCHIVE") {
        return fs::read(path);
    }
    let url = format!("{BASE_URL}/{TAG}/{}", asset.file_name());
    // The OS trust store, not bundled roots, so TLS-inspecting proxies and antivirus work.
    // Integrity does not depend on TLS: the archive's sha256 is pinned.
    let agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent();
    let response = agent.get(&url).call().map_err(io::Error::other)?;
    let mut bytes = Vec::new();
    response.into_body().into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn extract(asset: &Asset, archive: &[u8]) -> io::Result<Vec<u8>> {
    let wanted = asset.entry_name();
    let mut binary = Vec::new();
    if asset.zip {
        let mut zip = zip::ZipArchive::new(io::Cursor::new(archive)).map_err(io::Error::other)?;
        zip.by_name(&wanted)
            .map_err(io::Error::other)?
            .read_to_end(&mut binary)?;
        return Ok(binary);
    }
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(&wanted) {
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "entry not in archive",
    ))
}
