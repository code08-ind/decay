//! Issuing Decay licenses by hand (the purchase webhook, P4.3, does the same):
//!
//! ```text
//! decay-license-tool keygen <secret-file>            new signing key (0600); prints the public key
//! decay-license-tool issue <secret-file> <email> [id]  prints a license key
//! decay-license-tool verify <key>                     checks a key against the app's public key
//! ```
//!
//! The secret file holds the 32 private bytes as hex. Keep it out of the repository and
//! backed up: without it no new key can be issued for installed copies.

use std::process::ExitCode;

use decay_license::{License, issue, public_key_of, verify};

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn read_secret(path: &str) -> Result<[u8; 32], String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let text = text.trim();
    if text.len() != 64 {
        return Err(format!("{path}: expected 64 hex characters"));
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(out)
}

fn keygen(path: &str) -> Result<(), String> {
    if std::path::Path::new(path).exists() {
        return Err(format!("{path} exists: refusing to replace a signing key"));
    }
    let mut secret = [0u8; 32];
    let mut random = std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    std::io::Read::read_exact(&mut random, &mut secret).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path).map_err(|e| format!("{path}: {e}"))?;
    std::io::Write::write_all(&mut file, format!("{}\n", hex(&secret)).as_bytes())
        .map_err(|e| e.to_string())?;
    let public = public_key_of(&secret);
    let list: Vec<String> = public.iter().map(|b| format!("0x{b:02x}")).collect();
    println!("[{}]", list.join(", "));
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [cmd, path] if cmd == "keygen" => keygen(path),
        [cmd, path, email, rest @ ..] if cmd == "issue" && rest.len() <= 1 => {
            let secret = read_secret(path)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?;
            let id = rest.first().cloned().unwrap_or_else(|| format!("manual-{}", now.as_secs()));
            let license = License {
                v: 1,
                email: email.clone(),
                tier: "pro".into(),
                issued_at: i64::try_from(now.as_millis()).unwrap_or(i64::MAX),
                license_id: id,
            };
            println!("{}", issue(&license, &secret));
            Ok(())
        }
        [cmd, key] if cmd == "verify" => {
            let license = verify(key).map_err(|e| e.to_string())?;
            println!("{license:?}");
            Ok(())
        }
        _ => Err(
            "usage: decay-license-tool keygen <file> | issue <file> <email> [id] | verify <key>"
                .into(),
        ),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
