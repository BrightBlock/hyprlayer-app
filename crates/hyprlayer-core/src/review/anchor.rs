const REACH: u32 = 50;

/// Where an anchor sits in `lines` now, 1-based; `None` means outdated.
pub fn reanchor(lines: &[&str], line: u32, anchor_text: &str) -> Option<u32> {
    let anchor = anchor_text.trim_end();
    if anchor.is_empty() {
        return (line >= 1 && line as usize <= lines.len()).then_some(line);
    }
    let matches = |n: u32| {
        n.checked_sub(1)
            .and_then(|i| lines.get(i as usize))
            .is_some_and(|text| text.trim_end() == anchor)
    };
    if matches(line) {
        return Some(line);
    }
    // Below before above at each distance, so a tie goes to the lower line.
    (1..=REACH)
        .flat_map(|d| [line.checked_sub(d), line.checked_add(d)])
        .flatten()
        .find(|&n| matches(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` distinct lines, `line 1` through `line n`.
    fn file(n: u32) -> Vec<String> {
        (1..=n).map(|i| format!("line {i}")).collect()
    }

    fn view(lines: &[String]) -> Vec<&str> {
        lines.iter().map(String::as_str).collect()
    }

    #[test]
    fn an_unchanged_line_stays_put() {
        let lines = file(10);
        assert_eq!(reanchor(&view(&lines), 1, "line 1"), Some(1));
        assert_eq!(reanchor(&view(&lines), 4, "line 4"), Some(4));
        assert_eq!(reanchor(&view(&lines), 10, "line 10"), Some(10));
    }

    #[test]
    fn lines_added_or_removed_above_move_it() {
        let mut grown = file(10);
        grown.splice(1..1, ["a".to_string(), "b".into(), "c".into()]);
        assert_eq!(reanchor(&view(&grown), 4, "line 4"), Some(7));
        let mut shrunk = file(10);
        shrunk.drain(0..2);
        assert_eq!(reanchor(&view(&shrunk), 4, "line 4"), Some(2));
    }

    #[test]
    fn a_duplicate_nearby_resolves_to_the_nearest_then_the_lower() {
        let mut lines = file(20);
        lines[8] = "line 10".into();
        lines[10] = "line 10".into();
        assert_eq!(reanchor(&view(&lines), 10, "line 10"), Some(10));
        lines[8] = "line 9".into();
        lines[10] = "line 11".into();
        lines[9] = "changed".into();
        lines[6] = "line 10".into();
        lines[11] = "line 10".into();
        assert_eq!(reanchor(&view(&lines), 10, "line 10"), Some(12));
        lines[6] = "line 7".into();
        lines[7] = "line 10".into();
        assert_eq!(reanchor(&view(&lines), 10, "line 10"), Some(8));
    }

    #[test]
    fn a_blank_anchor_keeps_its_line_and_is_never_searched() {
        let mut lines = file(10);
        lines[4] = String::new();
        assert_eq!(reanchor(&view(&lines), 3, ""), Some(3));
        assert_eq!(reanchor(&view(&lines), 3, " \t\r"), Some(3));
        assert_eq!(reanchor(&view(&lines), 10, ""), Some(10));
        assert_eq!(reanchor(&view(&lines), 11, ""), None);
    }

    #[test]
    fn a_line_past_the_end_with_no_match_is_outdated() {
        let lines = file(5);
        assert_eq!(reanchor(&view(&lines), 8, "line 8"), None);
        assert_eq!(reanchor(&view(&lines), 8, "line 3"), Some(3));
    }

    #[test]
    fn a_deleted_file_is_outdated() {
        assert_eq!(reanchor(&[], 1, "line 1"), None);
        assert_eq!(reanchor(&[], 1, ""), None);
    }

    #[test]
    fn crlf_and_trailing_whitespace_drift_still_match() {
        let lines = ["fn a() {", "    let x = 1;\r", "    let y = 2;   ", "}"];
        assert_eq!(reanchor(&lines, 2, "    let x = 1;"), Some(2));
        assert_eq!(reanchor(&lines, 3, "    let y = 2;"), Some(3));
        assert_eq!(reanchor(&lines, 1, "fn a() {\r"), Some(1));
        assert_eq!(reanchor(&lines, 1, "    let y = 2;\t"), Some(3));
        assert_eq!(reanchor(&lines, 2, "let x = 1;"), None);
    }

    #[test]
    fn a_match_more_than_fifty_lines_away_is_outdated() {
        let lines = file(200);
        assert_eq!(reanchor(&view(&lines), 10, "line 60"), Some(60));
        assert_eq!(reanchor(&view(&lines), 10, "line 61"), None);
        assert_eq!(reanchor(&view(&lines), 100, "line 50"), Some(50));
        assert_eq!(reanchor(&view(&lines), 100, "line 49"), None);
    }

    #[test]
    fn the_base_side_follows_its_line_after_the_merge_base_moves() {
        let old_base = file(6);
        assert_eq!(reanchor(&view(&old_base), 3, "line 3"), Some(3));
        let mut new_base = file(6);
        new_base.splice(0..0, ["use a;".to_string(), "use b;".into()]);
        assert_eq!(reanchor(&view(&new_base), 3, "line 3"), Some(5));
        let mut dropped = file(6);
        dropped.remove(2);
        assert_eq!(reanchor(&view(&dropped), 3, "line 3"), None);
    }
}
