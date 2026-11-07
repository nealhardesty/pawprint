//! Text pixel grids: one character ("glyph") per pixel.

/// A parsed grid of glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    pub glyphs: Vec<char>,
}

/// Parse a grid. Rows are lines; surrounding whitespace on each line is
/// ignored (so grids may be indented), blank lines before and after the grid
/// are dropped, and every row must have the same width. Glyphs are any
/// non-whitespace characters.
pub fn parse_grid(text: &str) -> Result<Grid, String> {
    let rows: Vec<&str> = text.lines().map(str::trim).collect();
    let start = rows.iter().position(|r| !r.is_empty());
    let end = rows.iter().rposition(|r| !r.is_empty());
    let (Some(start), Some(end)) = (start, end) else {
        return Err("grid is empty".into());
    };
    let rows = &rows[start..=end];
    let width = rows[0].chars().count();
    let mut glyphs = Vec::with_capacity(width * rows.len());
    for (i, row) in rows.iter().enumerate() {
        let n = row.chars().count();
        if n != width {
            return Err(format!(
                "grid row {} has {n} glyphs, expected {width}",
                i + 1
            ));
        }
        if row.chars().any(char::is_whitespace) {
            return Err(format!("grid row {} contains whitespace", i + 1));
        }
        glyphs.extend(row.chars());
    }
    Ok(Grid {
        width,
        height: rows.len(),
        glyphs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_indented_grid() {
        let g = parse_grid("\n  .k.\n  kek\n\n").unwrap();
        assert_eq!((g.width, g.height), (3, 2));
        assert_eq!(g.glyphs, vec!['.', 'k', '.', 'k', 'e', 'k']);
        assert_eq!(
            parse_grid(".k.\r\nkek\r\n").unwrap(),
            g,
            "CRLF line endings"
        );
    }

    #[test]
    fn rejects_ragged_and_empty() {
        assert_eq!(
            parse_grid("..\n...").unwrap_err(),
            "grid row 2 has 3 glyphs, expected 2"
        );
        assert!(parse_grid("  \n ").is_err());
        assert!(parse_grid(". .\n...").is_err());
    }
}
