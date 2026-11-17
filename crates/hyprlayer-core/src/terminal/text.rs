//! Terminal output as plain text: escape sequences stripped, the last lines kept.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// The last `lines` lines of `bytes` with CSI, OSC, string and lone ESC sequences removed, on a
/// terminal `cols` wide.
pub fn tail(bytes: &[u8], cols: usize, lines: usize) -> String {
    let text = String::from_utf8_lossy(&strip(bytes, cols)).into_owned();
    let all: Vec<&str> = text.lines().map(str::trim_end).collect();
    let skip = all.len().saturating_sub(lines);
    all[skip..].join("\n")
}

fn strip(bytes: &[u8], cols: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    // Characters since the last newline, so a column move can be padded out.
    let mut col = 0;
    // A stream of `\r` and far moves would pad every line: all padding together never outgrows the input.
    let mut budget = bytes.len();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == ESC {
            let len = sequence_len(&bytes[i..]);
            // claude places a line's words with column moves rather than spaces; any program can ask
            // for a billion columns, so a move stops at the terminal's edge.
            let pad = match forward_move(&bytes[i..i + len]) {
                Some(Move::Column(to)) => to.min(cols).saturating_sub(col),
                Some(Move::Right(n)) => n.min(cols.saturating_sub(col)),
                None => 0,
            }
            .min(budget);
            budget -= pad;
            out.resize(out.len() + pad, b' ');
            col += pad;
            i += len;
            continue;
        }
        match b {
            // A TUI redraws a line in place; each redraw reads as its own line.
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => out.push(b'\n'),
            b'\n' | b'\t' => out.push(b),
            0x00..=0x1f | 0x7f => {}
            _ => out.push(b),
        }
        match out.last() {
            Some(b'\n') => col = 0,
            Some(b'\t') if b == b'\t' => col = (col / 8 + 1) * 8,
            _ if b >= 0x20 && b != 0x7f && b & 0xc0 != 0x80 => col += 1,
            _ => {}
        }
        i += 1;
    }
    out
}

enum Move {
    /// `CSI n G`: to the zero-based column.
    Column(usize),
    /// `CSI n C`: right by n.
    Right(usize),
}

fn forward_move(seq: &[u8]) -> Option<Move> {
    let (&last, body) = seq.strip_prefix(&[ESC, b'['])?.split_last()?;
    let n = std::str::from_utf8(body)
        .ok()?
        .parse::<usize>()
        .unwrap_or(1)
        .max(1);
    match last {
        b'G' => Some(Move::Column(n - 1)),
        b'C' => Some(Move::Right(n)),
        _ => None,
    }
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
