use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const CHUNK_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a GGUF artifact")]
    NotGguf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u8,
    pub model_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub chunk_size: usize,
    pub chunk_hashes: Vec<String>,
    pub merkle_root: String,
}

pub fn scan(path: impl AsRef<Path>) -> Result<Manifest, Error> {
    let path = path.as_ref();
    let meta = fs::metadata(path)?;
    let mut file = File::open(path)?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(Error::NotGguf);
    }
    let mut full = Hasher::new();
    let mut hashes = Vec::new();
    let mut buffer = vec![0; CHUNK_SIZE];
    file = File::open(path)?;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        full.update(&buffer[..n]);
        hashes.push(blake3::hash(&buffer[..n]).to_hex().to_string());
    }
    Ok(Manifest {
        version: 1,
        model_id: full.finalize().to_hex().to_string(),
        file_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        file_size: meta.len(),
        chunk_size: CHUNK_SIZE,
        merkle_root: merkle_root(&hashes),
        chunk_hashes: hashes,
    })
}

pub fn write_atomic(manifest: &Manifest, dir: impl AsRef<Path>) -> Result<PathBuf, Error> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir)?;
    let target = dir.join(format!("{}.json", manifest.model_id));
    let tmp = dir.join(format!(".{}.tmp", manifest.model_id));
    let mut out = File::create(&tmp)?;
    out.write_all(serde_json::to_vec_pretty(manifest)?.as_slice())?;
    out.sync_all()?;
    fs::rename(tmp, &target)?;
    Ok(target)
}

pub fn merkle_root(hashes: &[String]) -> String {
    if hashes.is_empty() {
        return blake3::hash(&[]).to_hex().to_string();
    }
    let mut level: Vec<[u8; 32]> = hashes
        .iter()
        .map(|h| {
            let raw = hex::decode(h).expect("chunk hash must be valid hex");
            <[u8; 32]>::try_from(raw.as_slice()).expect("chunk hash must be 32 bytes")
        })
        .collect();
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|p| {
                let mut h = Hasher::new();
                h.update(&p[0]);
                h.update(p.get(1).unwrap_or(&p[0]));
                *h.finalize().as_bytes()
            })
            .collect();
    }
    hex::encode(level[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_stable() {
        let x = vec![
            blake3::hash(b"a").to_hex().to_string(),
            blake3::hash(b"b").to_hex().to_string(),
        ];
        assert_eq!(merkle_root(&x), merkle_root(&x));
    }

    #[test]
    fn root_of_single_chunk_is_the_chunk_hash() {
        let hash = blake3::hash(b"chunk").to_hex().to_string();
        assert_eq!(merkle_root(std::slice::from_ref(&hash)), hash);
    }

    #[test]
    fn root_combines_raw_digests() {
        let a = blake3::hash(b"a");
        let b = blake3::hash(b"b");
        let mut expected = Hasher::new();
        expected.update(a.as_bytes());
        expected.update(b.as_bytes());
        let root = merkle_root(&[a.to_hex().to_string(), b.to_hex().to_string()]);
        assert_eq!(root, expected.finalize().to_hex().to_string());
    }
}
