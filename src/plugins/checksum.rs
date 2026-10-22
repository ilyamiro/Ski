//! Checksums in Properties: MD5, SHA-1, SHA-256 and SHA-512 of a file, computed in one pass on a
//! worker thread, compared with a hash on the clipboard and with the checksum file that sits
//! beside the download (`name.sha256`, `SHA256SUMS`, ...). A checksum file can also be verified
//! as a whole from the context menu.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;

use super::{Job as AppJob, MenuItem, Plugin, PropertyGroup, Reaction};
use crate::fs::engine::{self, Outcome, Worker};
use crate::fs::file_item::{FileItem, format_bytes};
use crate::i18n::{tr, trf};
use crate::ui::icons;
use crate::ui::material;
use crate::ui::theme::SerpantinumTheme;

/// A worker gives up when Properties has not asked for its file this long, so closing the dialog
/// stops the hashing of a multi-gigabyte image.
const ABANDON_AFTER_MS: u64 = 1500;

pub struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
    0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
    0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
    0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
    0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            buffer: Vec::with_capacity(64),
            length: 0,
        }
    }

    fn block(state: &mut [u32; 8], chunk: &[u8]) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        let state = &mut self.state;
        update_blocks(&mut self.buffer, data, |block| Self::block(state, block));
    }

    pub fn finish(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        tail.resize(1 + (119 - (self.length as usize % 64)) % 64, 0);
        tail.extend_from_slice(&bits.to_be_bytes());
        self.update(&tail);
        self.state.iter().map(|w| format!("{w:08x}")).collect()
    }
}

/// Feeds `data` to `block` in 64-byte pieces, keeping the remainder in `buffer`.
fn update_blocks(buffer: &mut Vec<u8>, mut data: &[u8], mut block: impl FnMut(&[u8])) {
    if !buffer.is_empty() {
        let take = (64 - buffer.len()).min(data.len());
        buffer.extend_from_slice(&data[..take]);
        data = &data[take..];
        if buffer.len() == 64 {
            block(buffer);
            buffer.clear();
        }
    }
    while data.len() >= 64 {
        let (head, rest) = data.split_at(64);
        block(head);
        data = rest;
    }
    buffer.extend_from_slice(data);
}

const H512: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
];

const K512: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
    0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

pub struct Sha512 {
    state: [u64; 8],
    buffer: Vec<u8>,
    length: u128,
}

impl Sha512 {
    pub fn new() -> Self {
        Self { state: H512, buffer: Vec::with_capacity(128), length: 0 }
    }

    fn block(state: &mut [u64; 8], chunk: &[u8]) {
        let mut w = [0u64; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u64::from_be_bytes(chunk[i * 8..i * 8 + 8].try_into().expect("8 bytes"));
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K512[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.length += data.len() as u128;
        let state = &mut self.state;
        if !self.buffer.is_empty() {
            let take = (128 - self.buffer.len()).min(data.len());
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buffer.len() == 128 {
                Self::block(state, &self.buffer);
                self.buffer.clear();
            }
        }
        while data.len() >= 128 {
            let (head, rest) = data.split_at(128);
            Self::block(state, head);
            data = rest;
        }
        self.buffer.extend_from_slice(data);
    }

    pub fn finish(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        // Padding leaves 16 bytes of the last block for the length
        tail.resize(1 + (239 - (self.length as usize % 128)) % 128, 0);
        tail.extend_from_slice(&bits.to_be_bytes());
        self.update(&tail);
        self.state.iter().map(|w| format!("{w:016x}")).collect()
    }
}

pub struct Sha1 {
    state: [u32; 5],
    buffer: Vec<u8>,
    length: u64,
}

impl Sha1 {
    pub fn new() -> Self {
        Self { state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0], buffer: Vec::with_capacity(64), length: 0 }
    }

    fn block(state: &mut [u32; 5], chunk: &[u8]) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = *state;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        let state = &mut self.state;
        update_blocks(&mut self.buffer, data, |block| Self::block(state, block));
    }

    pub fn finish(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        tail.resize(1 + (119 - (self.length as usize % 64)) % 64, 0);
        tail.extend_from_slice(&bits.to_be_bytes());
        self.update(&tail);
        self.state.iter().map(|w| format!("{w:08x}")).collect()
    }
}

pub struct Md5 {
    state: [u32; 4],
    buffer: Vec<u8>,
    length: u64,
}

const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11,
    16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

impl Md5 {
    pub fn new() -> Self {
        Self { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], buffer: Vec::with_capacity(64), length: 0 }
    }

    fn block(state: &mut [u32; 4], chunk: &[u8]) {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        let [mut a, mut b, mut c, mut d] = *state;
        for (i, shift) in MD5_SHIFTS.iter().enumerate() {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let k = ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32;
            let sum = a.wrapping_add(f).wrapping_add(k).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(sum.rotate_left(*shift));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        let state = &mut self.state;
        update_blocks(&mut self.buffer, data, |block| Self::block(state, block));
    }

    pub fn finish(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        tail.resize(1 + (119 - (self.length as usize % 64)) % 64, 0);
        tail.extend_from_slice(&bits.to_le_bytes());
        self.update(&tail);
        self.state.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashes {
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
}

impl Hashes {
    /// The name of the hash that equals `candidate` (a hex string, case and spaces ignored).
    pub fn matching(&self, candidate: &str) -> Option<&'static str> {
        let candidate: String = candidate.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        Algo::ALL.into_iter().find(|a| self.get(*a) == candidate).map(Algo::name)
    }

    pub fn get(&self, algo: Algo) -> &str {
        match algo {
            Algo::Md5 => &self.md5,
            Algo::Sha1 => &self.sha1,
            Algo::Sha256 => &self.sha256,
            Algo::Sha512 => &self.sha512,
        }
    }
}

/// Whether `text` looks like a hash someone copied: hex digits of an MD5, SHA-1, SHA-256 or SHA-512 length.
pub fn looks_like_hash(text: &str) -> bool {
    let text = text.trim();
    Algo::from_hex_len(text.len()).is_some() && text.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Algo {
    pub const ALL: [Algo; 4] = [Algo::Md5, Algo::Sha1, Algo::Sha256, Algo::Sha512];

    pub fn name(self) -> &'static str {
        match self {
            Algo::Md5 => "MD5",
            Algo::Sha1 => "SHA-1",
            Algo::Sha256 => "SHA-256",
            Algo::Sha512 => "SHA-512",
        }
    }

    fn from_hex_len(len: usize) -> Option<Self> {
        match len {
            32 => Some(Algo::Md5),
            40 => Some(Algo::Sha1),
            64 => Some(Algo::Sha256),
            128 => Some(Algo::Sha512),
            _ => None,
        }
    }

    /// The tag of the BSD format: `SHA256 (name) = hash`.
    fn from_tag(tag: &str) -> Option<Self> {
        match tag.to_ascii_uppercase().replace('-', "").as_str() {
            "MD5" => Some(Algo::Md5),
            "SHA1" => Some(Algo::Sha1),
            "SHA256" => Some(Algo::Sha256),
            "SHA512" => Some(Algo::Sha512),
            _ => None,
        }
    }
}

/// One hash being computed.
enum Hasher {
    Md5(Md5),
    Sha1(Sha1),
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Hasher {
    fn new(algo: Algo) -> Self {
        match algo {
            Algo::Md5 => Hasher::Md5(Md5::new()),
            Algo::Sha1 => Hasher::Sha1(Sha1::new()),
            Algo::Sha256 => Hasher::Sha256(Sha256::new()),
            Algo::Sha512 => Hasher::Sha512(Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Hasher::Md5(h) => h.finish(),
            Hasher::Sha1(h) => h.finish(),
            Hasher::Sha256(h) => h.finish(),
            Hasher::Sha512(h) => h.finish(),
        }
    }
}

/// A line of a checksum file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub algo: Algo,
    /// Lower case hex.
    pub hash: String,
    /// The file the hash is for, as written; `None` when the file holds just a hash.
    pub name: Option<String>,
}

/// Reads the GNU format (`hash  name`, `hash *name`), the BSD format (`SHA256 (name) = hash`) and
/// a bare hash on a line of its own. Comments and lines that are none of these are skipped.
pub fn parse_checksums(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(entry) = parse_bsd(line).or_else(|| parse_gnu(line)) {
            entries.push(entry);
        }
    }
    entries
}

fn parse_bsd(line: &str) -> Option<Entry> {
    let (tag, rest) = line.split_once(" (")?;
    let (name, hash) = rest.rsplit_once(") = ")?;
    let algo = Algo::from_tag(tag)?;
    let hash = hash.trim().to_ascii_lowercase();
    (hash.len() == hex_len(algo) && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| Entry { algo, hash, name: Some(name.to_owned()) })
}

fn hex_len(algo: Algo) -> usize {
    match algo {
        Algo::Md5 => 32,
        Algo::Sha1 => 40,
        Algo::Sha256 => 64,
        Algo::Sha512 => 128,
    }
}

fn parse_gnu(line: &str) -> Option<Entry> {
    // A leading backslash says the name below is escaped (it had a newline or a backslash)
    let (escaped, line) = match line.strip_prefix('\\') {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let end = line.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(line.len());
    let (hash, rest) = line.split_at(end);
    let algo = Algo::from_hex_len(hash.len())?;
    let hash = hash.to_ascii_lowercase();
    if rest.trim().is_empty() {
        return Some(Entry { algo, hash, name: None });
    }
    // One space, then a space (text) or `*` (binary), then the name
    let name = rest.strip_prefix("  ").or_else(|| rest.strip_prefix(" *"))?;
    let name = if escaped { name.replace("\\n", "\n").replace("\\\\", "\\") } else { name.to_owned() };
    Some(Entry { algo, hash, name: Some(name) })
}

/// The path an entry names inside `dir`, or `None` when it would leave the folder.
pub fn safe_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let name = name.strip_prefix("./").unwrap_or(name);
    let relative = Path::new(name);
    let safe = !name.is_empty() && relative.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    safe.then(|| dir.join(relative))
}

const EXTENSIONS: [&str; 8] = [".sha256", ".sha256sum", ".sha1", ".sha1sum", ".sha512", ".sha512sum", ".md5", ".md5sum"];
const SHARED_LISTS: [&str; 12] = [
    "sha256sums", "sha256sums.txt", "sha256sum.txt", "sha1sums", "sha1sums.txt", "sha512sums", "sha512sums.txt", "md5sums", "md5sums.txt", "md5sum.txt", "checksums", "checksums.txt",
];

/// How long Properties trusts what it read from the clipboard and from checksum files.
const RECHECK: Duration = Duration::from_secs(2);

/// Names that hold checksums of other files.
pub fn is_checksum_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    EXTENSIONS.iter().any(|e| lower.ends_with(e)) || SHARED_LISTS.contains(&lower.as_str())
}

/// The checksum files in the folder of `path` that may mention it: its own (`path.sha256`) and
/// the shared lists (`SHA256SUMS`).
pub fn candidates_for(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase())) else { return Vec::new() };
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let own = format!("{name}.");
    let mut found: Vec<PathBuf> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            let other = e.file_name().to_string_lossy().to_ascii_lowercase();
            (other.starts_with(&own) && is_checksum_file(&other)) || SHARED_LISTS.contains(&other.as_str())
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found
}

/// What the checksum files beside `path` say about it: the file that lists it and whether `hashes`
/// agree. `None` when no file lists it.
pub fn verdict_for(path: &Path, hashes: &Hashes) -> Option<(String, bool)> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let dir = path.parent()?;
    for candidate in candidates_for(path) {
        let Ok(text) = std::fs::read_to_string(&candidate) else { continue };
        let own = candidate.file_name()?.to_string_lossy().to_ascii_lowercase().starts_with(&format!("{}.", name.to_ascii_lowercase()));
        for entry in parse_checksums(&text) {
            let listed = match &entry.name {
                Some(listed) => safe_path(dir, listed).is_some_and(|p| p == path),
                // A file holding only a hash belongs to the file it is named after
                None => own,
            };
            if listed {
                let file = candidate.file_name()?.to_string_lossy().into_owned();
                return Some((file, hashes.get(entry.algo) == entry.hash));
            }
        }
    }
    None
}

/// The outcome of verifying a checksum file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub source: String,
    pub ok: usize,
    pub failed: Vec<String>,
    pub missing: Vec<String>,
    pub cancelled: bool,
}

fn hash_file(w: &Worker, path: &Path, algo: Algo) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Hasher::new(algo);
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        w.shared.checkpoint()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hasher.finish());
        }
        hasher.update(&buffer[..n]);
        w.add_bytes(n as u64);
    }
}

/// Hashes every file `entries` lists inside `dir` and tells `report` how it went.
fn verify(w: &mut Worker, dir: &Path, source: String, entries: Vec<Entry>, report: &std::sync::mpsc::Sender<Report>) -> Outcome {
    use std::sync::atomic::Ordering::Relaxed;
    let targets: Vec<(Entry, Option<PathBuf>)> = entries.into_iter().filter(|e| e.name.is_some()).map(|e| {
        let path = e.name.as_deref().and_then(|n| safe_path(dir, n));
        (e, path)
    }).collect();
    let present: Vec<&PathBuf> = targets.iter().filter_map(|(_, p)| p.as_ref()).filter(|p| p.is_file()).collect();
    w.shared.files_total.store(present.len() as u64, Relaxed);
    w.shared.bytes_total.store(present.iter().filter_map(|p| p.metadata().ok()).map(|m| m.len()).sum(), Relaxed);

    let mut result = Report { source: source.clone(), ..Report::default() };
    for (entry, path) in &targets {
        let name = entry.name.clone().unwrap_or_default();
        let Some(path) = path else {
            result.failed.push(trf("{} (outside the folder)", &[&name]));
            continue;
        };
        if !path.is_file() {
            result.missing.push(name);
            continue;
        }
        w.shared.set_current(&name);
        match hash_file(w, path, entry.algo) {
            Ok(hash) if hash == entry.hash => result.ok += 1,
            Ok(_) => result.failed.push(name),
            Err(e) if engine::is_cancel(&e) => {
                result.cancelled = true;
                break;
            }
            Err(e) => result.failed.push(format!("{name}: {e}")),
        }
        w.file_done();
    }
    let message = trf("{}: {} OK, {} failed, {} missing", &[&source, &result.ok, &result.failed.len(), &result.missing.len()]);
    let cancelled = result.cancelled;
    let _ = report.send(result);
    let mut outcome = Outcome::done(if cancelled { tr("Cancelled").to_owned() } else { message });
    outcome.cancelled = cancelled;
    outcome
}

struct Job {
    done: AtomicU64,
    total: u64,
    /// Milliseconds since `epoch` at which Properties last asked about the file.
    seen: AtomicU64,
    epoch: Instant,
    cancelled: AtomicBool,
    result: Mutex<Option<Result<Hashes, String>>>,
}

impl Job {
    fn touch(&self) {
        self.seen.store(self.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    fn abandoned(&self) -> bool {
        self.epoch.elapsed().as_millis() as u64 > self.seen.load(Ordering::Relaxed) + ABANDON_AFTER_MS
    }
}

fn run_job(path: &Path, job: &Job) -> Result<Hashes, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let (mut md5, mut sha1, mut sha256, mut sha512) = (Md5::new(), Sha1::new(), Sha256::new(), Sha512::new());
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if job.abandoned() {
            job.cancelled.store(true, Ordering::Relaxed);
            return Err(String::new());
        }
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        md5.update(&buf[..n]);
        sha1.update(&buf[..n]);
        sha256.update(&buf[..n]);
        sha512.update(&buf[..n]);
        job.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(Hashes { md5: md5.finish(), sha1: sha1.finish(), sha256: sha256.finish(), sha512: sha512.finish() })
}

type Key = (PathBuf, Option<SystemTime>, u64);

#[derive(Default)]
pub struct Checksum {
    jobs: HashMap<Key, Arc<Job>>,
    /// The text on the clipboard when the file was last looked at; read again after `RECHECK`,
    /// so copying a hash while Properties is open shows up.
    clipboard: HashMap<Key, (Instant, Option<String>)>,
    /// What the checksum files beside a file said about it, looked up once per file.
    verdicts: HashMap<Key, Option<(String, bool)>>,
    /// Verifications report here when they finish.
    reports: Option<(std::sync::mpsc::Sender<Report>, std::sync::mpsc::Receiver<Report>)>,
    report: Option<Report>,
}

impl Plugin for Checksum {
    fn id(&self) -> &'static str {
        "checksum"
    }

    fn name(&self) -> &'static str {
        tr("Checksums")
    }

    fn description(&self) -> &'static str {
        tr("Shows MD5, SHA-1, SHA-256 and SHA-512 in Properties, compares them with the clipboard and with checksum files, and verifies whole checksum lists")
    }

    fn icon(&self) -> &'static str {
        icons::SHIELD
    }

    fn busy(&self) -> bool {
        self.jobs.values().any(|j| j.result.lock().is_ok_and(|r| r.is_none()))
    }

    fn poll(&mut self) -> bool {
        let latest = self.reports.as_ref().and_then(|(_, rx)| rx.try_iter().last());
        let found = latest.is_some();
        if latest.is_some() {
            self.report = latest;
        }
        found
    }

    fn menu_items(&self, selection: &[FileItem]) -> Vec<MenuItem> {
        match selection {
            [one] if !one.is_dir && is_checksum_file(&one.name) => vec![MenuItem::new("verify", tr("Verify files listed"), icons::SHIELD)],
            _ => Vec::new(),
        }
    }

    fn run(&mut self, id: &str, paths: &[PathBuf], _dir: &Path) -> Reaction {
        let (true, [file]) = (id == "verify", paths) else { return Reaction::None };
        let Some(dir) = file.parent().map(Path::to_path_buf) else { return Reaction::None };
        let source = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut entries = match std::fs::read_to_string(file) {
            Ok(text) => parse_checksums(&text),
            Err(e) => {
                self.report = Some(Report { source, failed: vec![e.to_string()], ..Report::default() });
                return Reaction::None;
            }
        };
        // `file.sha256` holding a bare hash is about `file`
        for entry in entries.iter_mut().filter(|e| e.name.is_none()) {
            entry.name = Path::new(&source).file_stem().map(|s| s.to_string_lossy().into_owned());
        }
        let tx = self.reports.get_or_insert_with(std::sync::mpsc::channel).0.clone();
        let title = trf("Verifying {}", &[&source]);
        Reaction::Job(AppJob::new(title, move |w| verify(w, &dir, source, entries, &tx)))
    }

    fn window_title(&self) -> Option<&'static str> {
        self.report.as_ref().map(|_| tr("Verification result"))
    }

    fn close_window(&mut self) {
        self.report = None;
    }

    fn window(&mut self, ui: &mut egui::Ui, theme: &SerpantinumTheme) -> bool {
        let Some(report) = &self.report else { return false };
        let summary = trf("{}: {} OK, {} failed, {} missing", &[&report.source, &report.ok, &report.failed.len(), &report.missing.len()]);
        let clean = report.failed.is_empty() && report.missing.is_empty();
        ui.label(egui::RichText::new(summary).size(material::FONT_BODY).color(if clean { theme.green } else { theme.red }));
        if report.cancelled {
            ui.label(egui::RichText::new(tr("Cancelled, the list is incomplete")).size(material::FONT_CAPTION).color(theme.subtext1));
        }
        egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
            for (title, names, color) in [(tr("Failed"), &report.failed, theme.red), (tr("Missing"), &report.missing, theme.yellow)] {
                if names.is_empty() {
                    continue;
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new(title).size(material::FONT_CAPTION).color(theme.subtext1));
                for name in names {
                    ui.label(egui::RichText::new(name).monospace().size(12.5).color(color));
                }
            }
        });
        true
    }

    fn properties(&mut self, item: &FileItem) -> Vec<PropertyGroup> {
        if item.is_dir {
            return Vec::new();
        }
        let key: Key = (item.path.clone(), item.modified, item.size);
        // Only the file on screen keeps hashing
        self.jobs.retain(|k, job| *k == key || job.result.lock().is_ok_and(|r| r.is_some()));
        let job = self
            .jobs
            .entry(key.clone())
            .or_insert_with(|| {
                let job = Arc::new(Job {
                    done: AtomicU64::new(0),
                    total: item.size,
                    seen: AtomicU64::new(0),
                    epoch: Instant::now(),
                    cancelled: AtomicBool::new(false),
                    result: Mutex::new(None),
                });
                let worker = Arc::clone(&job);
                let path = item.path.clone();
                std::thread::spawn(move || {
                    let result = run_job(&path, &worker);
                    if let Ok(mut slot) = worker.result.lock() {
                        *slot = Some(result);
                    }
                });
                job
            })
            .clone();
        job.touch();

        let result = job.result.lock().ok().and_then(|r| r.clone());
        match result {
            None => {
                let percent = job.done.load(Ordering::Relaxed).saturating_mul(100).checked_div(job.total).unwrap_or(0);
                let busy = trf("Calculating… {}%", &[&percent]);
                vec![PropertyGroup { title: tr("Checksums"), rows: vec![(tr("Progress"), format!("{busy} ({})", format_bytes(job.total)))] }]
            }
            Some(Err(error)) if error.is_empty() => {
                // The worker gave up while the dialog was away: start over
                self.jobs.remove(&key);
                Vec::new()
            }
            Some(Err(error)) => vec![PropertyGroup { title: tr("Checksums"), rows: vec![(tr("Error"), error)] }],
            Some(Ok(hashes)) => {
                let mut rows = vec![("MD5", hashes.md5.clone()), ("SHA-1", hashes.sha1.clone()), ("SHA-256", hashes.sha256.clone()), ("SHA-512", hashes.sha512.clone())];
                let verdict = self.verdicts.entry(key.clone()).or_insert_with(|| verdict_for(&item.path, &hashes));
                if let Some((file, matches)) = verdict {
                    let text = if *matches { trf("Matches {}", &[&file]) } else { trf("Does not match {}", &[&file]) };
                    rows.push((tr("Checksum file"), text));
                }
                if self.clipboard.get(&key).is_none_or(|(at, _)| at.elapsed() > RECHECK) {
                    self.clipboard.insert(key.clone(), (Instant::now(), crate::clipboard::get_text()));
                }
                let clip = &self.clipboard[&key];
                if let Some(text) = clip.1.as_deref().filter(|t| looks_like_hash(t)) {
                    let verdict = match hashes.matching(text) {
                        Some(name) => trf("Match: the clipboard holds the {}", &[&name]),
                        None => tr("No match with the hash on the clipboard").to_owned(),
                    };
                    rows.push((tr("Clipboard"), verdict));
                }
                vec![PropertyGroup { title: tr("Checksums"), rows }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_bytes(data: &[u8]) -> Hashes {
        let (mut md5, mut sha1, mut sha256, mut sha512) = (Md5::new(), Sha1::new(), Sha256::new(), Sha512::new());
        md5.update(data);
        sha1.update(data);
        sha256.update(data);
        sha512.update(data);
        Hashes { md5: md5.finish(), sha1: sha1.finish(), sha256: sha256.finish(), sha512: sha512.finish() }
    }

    #[test]
    fn known_digests() {
        let empty = hash_bytes(b"");
        assert_eq!(empty.md5, "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(empty.sha1, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(empty.sha256, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let abc = hash_bytes(b"abc");
        assert_eq!(abc.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(abc.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(abc.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn sha512_matches_the_published_vectors() {
        assert_eq!(
            hash_bytes(b"").sha512,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        assert_eq!(
            hash_bytes(b"abc").sha512,
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        // The 112-byte message of FIPS 180-2: the length does not fit the first block
        let message = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
        assert_eq!(message.len(), 112);
        assert_eq!(
            hash_bytes(message).sha512,
            "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909"
        );
        // A million 'a's, fed in uneven pieces
        let mut long = Sha512::new();
        for piece in vec![b'a'; 1_000_000].chunks(997) {
            long.update(piece);
        }
        assert_eq!(
            long.finish(),
            "e718483d0ce769644e2e42c7bc15b4638e1f98b13b2044285632a803afa973ebde0ff244877ea60a4cb0432ce577c31beb009c5c2c49aa2e4eadb217ad8cc09b"
        );
    }

    #[test]
    fn parses_the_gnu_and_bsd_formats() {
        let abc256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let text = format!(
            "# comment\n{abc256}  plain.iso\n{upper} *binary.iso\n\nSHA256 (bsd file.iso) = {abc256}\nMD5 (m.bin) = 900150983cd24fb0d6963f7d28e17f72\nnot a checksum line\n{abc256}\n",
            upper = abc256.to_uppercase()
        );
        let entries = parse_checksums(&text);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0], Entry { algo: Algo::Sha256, hash: abc256.to_owned(), name: Some("plain.iso".to_owned()) });
        assert_eq!(entries[1].name.as_deref(), Some("binary.iso"));
        assert_eq!(entries[1].hash, abc256, "hex is read case-insensitively");
        assert_eq!(entries[2], Entry { algo: Algo::Sha256, hash: abc256.to_owned(), name: Some("bsd file.iso".to_owned()) });
        assert_eq!((entries[3].algo, entries[3].name.as_deref()), (Algo::Md5, Some("m.bin")));
        assert_eq!(entries[4].name, None);
    }

    #[test]
    fn names_that_leave_the_folder_are_refused() {
        let dir = Path::new("/d");
        assert_eq!(safe_path(dir, "a.iso"), Some(PathBuf::from("/d/a.iso")));
        assert_eq!(safe_path(dir, "./sub/a.iso"), Some(PathBuf::from("/d/sub/a.iso")));
        assert_eq!(safe_path(dir, "../a.iso"), None);
        assert_eq!(safe_path(dir, "sub/../../a.iso"), None);
        assert_eq!(safe_path(dir, "/etc/passwd"), None);
        assert_eq!(safe_path(dir, ""), None);
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ski-checksum-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_checksum_file_beside_the_download_gives_a_verdict() {
        let dir = folder("verdict");
        let abc = hash_bytes(b"abc");
        std::fs::write(dir.join("good.iso"), b"abc").unwrap();
        std::fs::write(dir.join("bad.iso"), b"abd").unwrap();
        std::fs::write(dir.join("unlisted.iso"), b"abc").unwrap();
        std::fs::write(dir.join("SHA256SUMS"), format!("{}  good.iso\n{}  bad.iso\n", abc.sha256, abc.sha256)).unwrap();
        std::fs::write(dir.join("solo.bin"), b"abc").unwrap();
        std::fs::write(dir.join("solo.bin.md5"), format!("{}\n", abc.md5)).unwrap();

        assert_eq!(verdict_for(&dir.join("good.iso"), &abc), Some(("SHA256SUMS".to_owned(), true)));
        assert_eq!(verdict_for(&dir.join("bad.iso"), &abc), Some(("SHA256SUMS".to_owned(), true)), "the verdict is about the hashes given");
        assert_eq!(verdict_for(&dir.join("bad.iso"), &hash_bytes(b"abd")), Some(("SHA256SUMS".to_owned(), false)));
        assert_eq!(verdict_for(&dir.join("unlisted.iso"), &abc), None);
        assert_eq!(verdict_for(&dir.join("solo.bin"), &abc), Some(("solo.bin.md5".to_owned(), true)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn verifying_a_list_counts_ok_failed_and_missing_files() {
        let dir = folder("verify");
        let abc = hash_bytes(b"abc");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("ok.bin"), b"abc").unwrap();
        std::fs::write(dir.join("sub/ok2.bin"), b"abc").unwrap();
        std::fs::write(dir.join("changed.bin"), b"abX").unwrap();
        let list = format!(
            "{h}  ok.bin\n{h}  sub/ok2.bin\n{h}  changed.bin\n{h}  gone.bin\n{h}  ../escape.bin\n",
            h = abc.sha256
        );
        std::fs::write(dir.join("SHA256SUMS"), &list).unwrap();

        let mut plugin = Checksum::default();
        let reaction = plugin.run("verify", &[dir.join("SHA256SUMS")], &dir);
        let Reaction::Job(job) = reaction else { panic!("no job") };

        let ctx = egui::Context::default();
        let mut engine = engine::Engine::new(&ctx);
        engine.spawn(engine::JobKind::Other, job.title, String::new(), job.work);
        let mut done = false;
        for _ in 0..500 {
            if engine.poll().iter().any(|e| matches!(e, engine::Event::Done { .. })) {
                done = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(done);
        assert!(plugin.poll());
        let report = plugin.report.clone().expect("report");
        assert_eq!(report.ok, 2);
        assert_eq!(report.missing, vec!["gone.bin"]);
        assert_eq!(report.failed.len(), 2, "{:?}", report.failed);
        assert!(report.failed.iter().any(|f| f == "changed.bin"));
        assert!(report.failed.iter().any(|f| f.contains("../escape.bin")));
        assert_eq!(plugin.window_title(), Some(tr("Verification result")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_menu_entry_is_for_a_single_checksum_file() {
        let dir = folder("menu");
        std::fs::write(dir.join("SHA256SUMS"), "").unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        let sums = FileItem::from_path(&dir.join("SHA256SUMS")).unwrap();
        let other = FileItem::from_path(&dir.join("a.txt")).unwrap();
        let plugin = Checksum::default();
        assert_eq!(plugin.menu_items(std::slice::from_ref(&sums)).len(), 1);
        assert!(plugin.menu_items(std::slice::from_ref(&other)).is_empty());
        assert!(plugin.menu_items(&[sums, other]).is_empty());
        assert!(is_checksum_file("Fedora.iso.SHA256") && is_checksum_file("md5sums.txt") && !is_checksum_file("notes.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn padding_boundaries_and_chunked_updates() {
        // 55, 56 and 64 bytes straddle where the length no longer fits the last block
        let sha = |n: usize| hash_bytes(&vec![b'a'; n]).sha256;
        assert_eq!(sha(55), "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318");
        assert_eq!(sha(56), "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
        assert_eq!(sha(64), "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb");
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let mut chunked = Sha256::new();
        for piece in data.chunks(37) {
            chunked.update(piece);
        }
        assert_eq!(chunked.finish(), hash_bytes(&data).sha256);
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hash_bytes(&million).sha1, "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        assert_eq!(hash_bytes(&million).md5, "7707d6ae4e027c70eea2a935c2296f21");
    }

    #[test]
    fn clipboard_comparison() {
        let h = hash_bytes(b"abc");
        assert_eq!(h.matching("  BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD\n"), Some("SHA-256"));
        assert_eq!(h.matching("900150983cd24fb0d6963f7d28e17f72"), Some("MD5"));
        assert_eq!(h.matching("00"), None);
        assert!(looks_like_hash("900150983cd24fb0d6963f7d28e17f72 "));
        assert!(looks_like_hash(&h.sha512));
        assert_eq!(h.matching(&h.sha512), Some("SHA-512"));
        assert!(!looks_like_hash("hello world"));
        assert!(!looks_like_hash("zz0150983cd24fb0d6963f7d28e17f72"));
    }

    #[test]
    fn hashes_a_file_and_gives_up_when_abandoned() {
        let path = std::env::temp_dir().join(format!("ski-hash-test-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let job = Job { done: AtomicU64::new(0), total: 3, seen: AtomicU64::new(0), epoch: Instant::now(), cancelled: AtomicBool::new(false), result: Mutex::new(None) };
        let hashes = run_job(&path, &job).unwrap();
        assert_eq!(hashes, hash_bytes(b"abc"));
        assert_eq!(job.done.load(Ordering::Relaxed), 3);
        std::fs::remove_file(&path).ok();
    }
}
