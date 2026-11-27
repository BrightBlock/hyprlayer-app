// gen <key>            : cargo-packager keypair (no password)
// sign <key> <file>    : cargo-packager signer -> <file>.sig
// tauri-verify <file> <sig> <pubfile> : tauri-plugin-updater 2.10.1's verify_signature, copied verbatim
// updater-download <file> <sig> <pubfile> <url> : cargo-packager-updater 0.2.3 Update::download against a served file
use base64::Engine as _;
use cargo_packager::sign::SigningConfig;
use minisign_verify::{PublicKey, Signature};
use std::fs;

fn b64s(s: &str) -> String {
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(s.trim()).expect("b64")).expect("utf8")
}

fn tauri_verify(data: &[u8], sig: &str, pubkey: &str) -> Result<(), String> {
    let public_key = PublicKey::decode(&b64s(pubkey)).map_err(|e| e.to_string())?;
    let signature = Signature::decode(&b64s(sig)).map_err(|e| e.to_string())?;
    public_key.verify(data, &signature, true).map_err(|e| e.to_string())
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a[0].as_str() {
        "gen" => {
            let kp = cargo_packager::sign::generate_key(Some(String::new())).unwrap();
            cargo_packager::sign::save_keypair(&kp, &a[1], true).unwrap();
        }
        "sign" => {
            let key = fs::read_to_string(&a[1]).unwrap();
            let cfg = SigningConfig::new().private_key(key).password("");
            println!("{}", cargo_packager::sign::sign_file(&cfg, &a[2]).unwrap().display());
        }
        "tauri-verify" => {
            let r = tauri_verify(&fs::read(&a[1]).unwrap(), &fs::read_to_string(&a[2]).unwrap(), &fs::read_to_string(&a[3]).unwrap());
            println!("tauri-verify: {r:?}");
            std::process::exit(r.is_err() as i32);
        }
        "updater-download" => {
            let update = cargo_packager_updater::Update {
                config: cargo_packager_updater::Config {
                    endpoints: vec![],
                    pubkey: fs::read_to_string(&a[3]).unwrap().trim().to_string(),
                    windows: None,
                },
                body: None,
                current_version: "0.0.0".into(),
                version: "0.0.1".into(),
                date: None,
                target: "linux".into(),
                extract_path: std::env::temp_dir(),
                download_url: a[4].parse().unwrap(),
                signature: fs::read_to_string(&a[2]).unwrap().trim().to_string(),
                timeout: None,
                headers: Default::default(),
                format: cargo_packager_updater::UpdateFormat::App,
            };
            let r = update.download().map(|b| b.len());
            println!("updater-download: {r:?}");
            std::process::exit(r.is_err() as i32);
        }
        _ => unreachable!(),
    }
}
