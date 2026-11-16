//! Terminal output as plain text: escape sequences stripped, the last lines kept.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// The last `lines` lines of `bytes` with CSI, OSC, string and lone ESC sequences removed.
pub fn tail(bytes: &[u8], lines: usize) -> String {
    let text = String::from_utf8_lossy(&strip(bytes)).into_owned();
    let all: Vec<&str> = text.lines().map(str::trim_end).collect();
    let skip = all.len().saturating_sub(lines);
    all[skip..].join("\n")
}

fn strip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == ESC {
            i += sequence_len(&bytes[i..]);
            continue;
        }
        match b {
            // A TUI redraws a line in place; each redraw reads as its own line.
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => out.push(b'\n'),
            b'\n' | b'\t' => out.push(b),
            0x00..=0x1f | 0x7f => {}
            _ => out.push(b),
        }
        i += 1;
    }
    out
}

/// Bytes from the ESC at `b[0]` to the end of its sequence; an unterminated one runs to the end.
fn sequence_len(b: &[u8]) -> usize {
    match b.get(1) {
        Some(b'[') => b[2..]
            .iter()
            .position(|c| (0x40..=0x7e).contains(c))
            .map_or(b.len(), |end| end + 3),
        Some(b']' | b'P' | b'X' | b'^' | b'_') => b[2..]
            .iter()
            .enumerate()
            .find_map(|(i, &c)| match c {
                BEL => Some(i + 3),
                ESC if b.get(i + 3) == Some(&b'\\') => Some(i + 4),
                _ => None,
            })
            .unwrap_or(b.len()),
        // Charset designation carries one more byte.
        Some(b'(' | b')' | b'*' | b'+') => b.len().min(3),
        Some(_) => 2,
        None => 1,
    }
}
