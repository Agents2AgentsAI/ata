//! Render a QR code as compact Unicode text using half-block characters.
//!
//! Each output character row represents two QR module rows, using:
//! - `█` (full block)  — both top and bottom modules are dark
//! - `▀` (upper half)  — top dark, bottom light
//! - `▄` (lower half)  — top light, bottom dark
//! - ` ` (space)       — both light

use qrcode::QrCode;
use qrcode::types::EcLevel;

/// Generate a QR code from `data` and render it as a vector of Unicode lines.
///
/// Each line uses half-block characters so two QR module rows are packed into
/// one terminal row.  A one-module quiet zone is included on all sides.
pub(crate) fn render_qr_to_lines(data: &str) -> Vec<String> {
    let code = match QrCode::with_error_correction_level(data, EcLevel::M) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let modules = code.to_colors();
    let width = code.width();

    // Build a boolean grid with a 1-module quiet zone on each side.
    let padded_w = width + 2;
    let padded_h = width + 2;
    let mut grid = vec![vec![false; padded_w]; padded_h];
    for (y, row) in modules.chunks(width).enumerate() {
        for (x, &color) in row.iter().enumerate() {
            grid[y + 1][x + 1] = color == qrcode::Color::Dark;
        }
    }

    // Render pairs of rows using half-block characters.
    let mut lines = Vec::with_capacity(padded_h.div_ceil(2));
    let mut y = 0;
    while y < padded_h {
        let mut line = String::with_capacity(padded_w);
        for (x, &top) in grid[y].iter().enumerate() {
            let bottom = if y + 1 < padded_h {
                grid[y + 1][x]
            } else {
                false
            };
            let ch = match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            line.push(ch);
        }
        lines.push(line);
        y += 2;
    }

    lines
}
