// Copyright (C) 2026 Binkap S
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A minimal VT100/xterm-compatible virtual terminal.
//!
//! The browser pipe feeds the terminal a raw stream of bytes; the shell
//! speaks `xterm-256color`. This module keeps just enough of the terminal
//! state machine — cursor, line buffer, scrollback, colors, erase and
//! cursor movement — to render the stream as styled lines without xterm.js.
//!
//! The cursor is rendered into the returned lines: the current line is tagged
//! with kind `"cursor"` and the cell under the cursor is replaced by a block
//! glyph. `Bridge` simply forwards `VtRender::lines` to `terminal-lines`, so
//! the cursor needs no extra property on `AppWindow`.

use std::cell::RefCell;
use std::rc::Rc;

/// A styled run of text inside a terminal line.
#[derive(Clone, Default)]
struct Cell {
    text: slint::SharedString,
    color: slint::SharedString,
    bold: bool,
}

fn blank_cell() -> Cell {
    Cell {
        text: slint::SharedString::from(" "),
        color: slint::SharedString::new(),
        bold: false,
    }
}

fn parse_brush(value: &str) -> slint::Brush {
    let value = value.trim();
    if value.is_empty() || value == "default" {
        return slint::Brush::SolidColor(slint::Color::from_rgb_u8(215, 220, 229));
    }
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return slint::Brush::SolidColor(slint::Color::from_rgb_u8(r, g, b));
            }
        }
    }
    slint::Brush::SolidColor(slint::Color::from_rgb_u8(215, 220, 229))
}

/// The rendered view of the terminal: lines plus the live cursor position.
pub struct VtRender {
    pub lines: Vec<crate::VtLine>,
    #[allow(dead_code)]
    pub cursor_row: usize,
    #[allow(dead_code)]
    pub cursor_col: usize,
}

const MAX_LINES: usize = 1000;
/// Scrollback is kept shorter than the live screen: every cell becomes a
/// Slint `Text` element, so a long history would make WASM repaints costly.
const MAX_SCROLLBACK: usize = 200;
/// Guard against runaway cursor positions from malformed CSI sequences.
const MAX_COLS: usize = 512;

/// Parser states. The old implementation tried to resolve an escape sequence
/// after every single byte, which only worked for `ESC[...m` and silently
/// dropped cursor movement, erase and OSC sequences. This state machine
/// collects a whole sequence before acting on it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParseState {
    Ground,
    Escape,
    Csi,
    Osc,
}

/// Convert the 256 xterm color index into an approximate hex value.
fn xterm_color(index: u32) -> Option<String> {
    let hex = match index {
        0..=7 => match index {
            0 => "7f8c8d",
            1 => "e74c3c",
            2 => "2ecc71",
            3 => "f1c40f",
            4 => "3498db",
            5 => "9b59b6",
            6 => "1abc9c",
            _ => "95a5a6",
        },
        8..=15 => match index {
            8 => "95a5a6",
            9 => "e74c3c",
            10 => "2ecc71",
            11 => "f1c40f",
            12 => "3498db",
            13 => "9b59b6",
            14 => "1abc9c",
            _ => "ecf0f1",
        },
        16..=231 => {
            // The 6×6×6 color cube.
            let r = ((index - 16) / 36) % 6;
            let g = ((index - 16) / 6) % 6;
            let b = (index - 16) % 6;
            let scale = |channel: u32| -> u32 {
                if channel == 0 { 0 } else { 55 + 40 * channel }
            };
            let rgb = [scale(r), scale(g), scale(b)];
            return Some(format!(
                "#{:02x}{:02x}{:02x}",
                rgb[0], rgb[1], rgb[2]
            ));
        }
        232..=255 => {
            let level = 8 + 10 * (index - 232);
            return Some(format!("#{level:02x}{level:02x}{level:02x}"));
        }
        _ => return None,
    };
    Some(format!("#{hex}"))
}

pub struct Terminal {
    rows: Vec<Vec<Cell>>,
    scrollback: Vec<crate::VtLine>,
    cursor_col: usize,
    cursor_row: usize,
    saved_cursor: Option<(usize, usize)>,
    last_color: String,
    last_bold: bool,
    cursor_visible: bool,
    state: ParseState,
    escape_buffer: Vec<u8>,
    osc_escape: bool,
    /// Reserved for callers that explicitly queue keystrokes. Output fed via
    /// `feed_str` is never recorded here: `Bridge::on_terminal_input` uses
    /// `take_input()` first, and recording shell output would send the prompt
    /// back to the shell instead of the user's command.
    input: String,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            scrollback: Vec::new(),
            cursor_col: 0,
            cursor_row: 0,
            saved_cursor: None,
            last_color: String::new(),
            last_bold: false,
            cursor_visible: true,
            state: ParseState::Ground,
            escape_buffer: Vec::new(),
            osc_escape: false,
            input: String::new(),
        }
    }

    fn ensure_row(&mut self, row: usize) {
        while self.rows.len() <= row {
            self.rows.push(Vec::new());
        }
    }

    fn line_text(line: &[Cell]) -> String {
        line.iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>()
    }

    /// Feed raw bytes (UTF-8) into the state machine.
    #[allow(dead_code)]
    pub fn feed(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        self.feed_str(&text);
    }

    pub fn feed_str(&mut self, text: &str) {
        for ch in text.chars() {
            match self.state {
                ParseState::Osc => {
                    // OSC (window title, etc.) ends at BEL or ST (`ESC \`).
                    if self.osc_escape {
                        self.osc_escape = false;
                        if ch == '\\' {
                            self.state = ParseState::Ground;
                        }
                    } else if ch == '\x07' {
                        self.state = ParseState::Ground;
                    } else if ch == '\x1b' {
                        self.osc_escape = true;
                    }
                    continue;
                }
                ParseState::Escape => {
                    self.handle_escape(ch);
                    continue;
                }
                ParseState::Csi => {
                    if ('@'..='~').contains(&ch) {
                        let params = std::mem::take(&mut self.escape_buffer);
                        self.state = ParseState::Ground;
                        self.finish_csi(ch, &params);
                    } else {
                        self.escape_buffer.push(ch as u8);
                    }
                    continue;
                }
                ParseState::Ground => {}
            }

            match ch {
                '\x1b' => {
                    self.state = ParseState::Escape;
                    self.escape_buffer.clear();
                }
                '\n' => self.newline(),
                '\r' => self.cursor_col = 0,
                '\x08' => {
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                }
                '\t' => {
                    let stops = 8;
                    let target = ((self.cursor_col / stops) + 1) * stops;
                    while self.cursor_col < target {
                        self.push_cell(blank_cell());
                    }
                }
                '\x07' => {}
                _ if ch.is_control() => {}
                _ => {
                    self.push_cell(Cell {
                        text: slint::SharedString::from(ch.to_string()),
                        color: slint::SharedString::from(self.last_color.clone()),
                        bold: self.last_bold,
                    });
                }
            }
        }
    }

    /// Resolve the byte following `ESC`.
    fn handle_escape(&mut self, ch: char) {
        self.state = ParseState::Ground;
        match ch {
            '[' => {
                self.state = ParseState::Csi;
                self.escape_buffer.clear();
            }
            ']' | 'P' | 'X' | '^' | '_' => {
                self.state = ParseState::Osc;
                self.osc_escape = false;
            }
            '7' => self.saved_cursor = Some((self.cursor_row, self.cursor_col)),
            '8' => {
                if let Some((row, col)) = self.saved_cursor.take() {
                    self.cursor_row = row;
                    self.cursor_col = col;
                    self.ensure_row(row);
                }
            }
            'D' => {
                // Index: move down one line.
                self.newline();
            }
            'M' => {
                // Reverse index: move up one line.
                self.cursor_row = self.cursor_row.saturating_sub(1);
            }
            'c' => {
                // RIS: full reset.
                self.rows.clear();
                self.scrollback.clear();
                self.cursor_row = 0;
                self.cursor_col = 0;
                self.reset_attributes();
                self.cursor_visible = true;
            }
            _ => {}
        }
    }

    fn newline(&mut self) {
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.trim_scrollback();
    }

    fn trim_scrollback(&mut self) {
        if self.rows.len() > MAX_LINES {
            let excess = self.rows.len() - MAX_LINES;
            for line in self.rows.drain(..excess) {
                self.cursor_row = self.cursor_row.saturating_sub(1);
                self.scrollback.push(crate::VtLine {
                    cells: slint::ModelRc::new(slint::VecModel::from(
                        line.into_iter()
                            .map(|cell| crate::TerminalCell {
                                text: cell.text,
                                color: parse_brush(cell.color.as_str()),
                                bold: cell.bold,
                            })
                            .collect::<Vec<_>>(),
                    )),
                    kind: "output".into(),
                });
            }
        }
        if self.scrollback.len() > MAX_SCROLLBACK {
            let excess = self.scrollback.len() - MAX_SCROLLBACK;
            self.scrollback.drain(..excess);
        }
    }

    fn push_cell(&mut self, cell: Cell) {
        self.ensure_row(self.cursor_row);
        let row = &mut self.rows[self.cursor_row];
        if self.cursor_col < row.len() {
            row[self.cursor_col] = cell;
        } else {
            while row.len() < self.cursor_col {
                row.push(blank_cell());
            }
            row.push(cell);
        }
        self.cursor_col = (self.cursor_col + 1).min(MAX_COLS);
    }

    /// Numeric CSI parameters, ignoring private prefixes (`?`, `>`) and
    /// sub-parameters (`:`). Missing or zero parameters use the CSI default.
    fn csi_params(raw: &[u8]) -> Vec<u32> {
        let text: String = raw
            .iter()
            .filter(|byte| byte.is_ascii_digit() || **byte == b';')
            .map(|byte| *byte as char)
            .collect();
        text.split(';')
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u32>().ok())
            .collect()
    }

    fn param(params: &[u32], index: usize, default: usize) -> usize {
        params
            .get(index)
            .copied()
            .filter(|value| *value != 0)
            .map(|value| value as usize)
            .unwrap_or(default)
    }

    fn clear_row(&mut self, row: usize, from: usize, to: usize) {
        self.ensure_row(row);
        let cells = &mut self.rows[row];
        let end = to.min(cells.len());
        if from < end {
            for cell in &mut cells[from..end] {
                *cell = blank_cell();
            }
        }
    }

    /// Resolve a CSI sequence once its final byte has arrived.
    fn finish_csi(&mut self, final_byte: char, raw: &[u8]) {
        let params = Self::csi_params(raw);
        match final_byte {
            'm' => self.apply_color_params(&params),
            'A' => {
                let count = Self::param(&params, 0, 1);
                self.cursor_row = self.cursor_row.saturating_sub(count);
            }
            'B' => {
                let count = Self::param(&params, 0, 1);
                self.cursor_row += count;
                self.ensure_row(self.cursor_row);
                self.trim_scrollback();
            }
            'C' => {
                self.cursor_col = (self.cursor_col + Self::param(&params, 0, 1)).min(MAX_COLS);
            }
            'D' => {
                self.cursor_col = self.cursor_col.saturating_sub(Self::param(&params, 0, 1));
            }
            'G' => {
                self.cursor_col = Self::param(&params, 0, 1)
                    .saturating_sub(1)
                    .min(MAX_COLS);
            }
            'H' | 'f' => {
                self.cursor_row = Self::param(&params, 0, 1).saturating_sub(1);
                self.cursor_col = Self::param(&params, 1, 1)
                    .saturating_sub(1)
                    .min(MAX_COLS);
                self.ensure_row(self.cursor_row);
            }
            'J' => match Self::param(&params, 0, 0) {
                0 => {
                    let row = self.cursor_row;
                    self.clear_row(row, self.cursor_col, usize::MAX);
                    for next in row + 1..self.rows.len() {
                        self.clear_row(next, 0, usize::MAX);
                    }
                }
                1 => {
                    let row = self.cursor_row;
                    self.clear_row(row, 0, self.cursor_col + 1);
                    for previous in 0..row {
                        self.clear_row(previous, 0, usize::MAX);
                    }
                }
                _ => {
                    self.rows.clear();
                    self.rows.push(Vec::new());
                    self.cursor_row = 0;
                    self.cursor_col = 0;
                }
            },
            'K' => match Self::param(&params, 0, 0) {
                0 => self.clear_row(self.cursor_row, self.cursor_col, usize::MAX),
                1 => self.clear_row(self.cursor_row, 0, self.cursor_col + 1),
                _ => self.clear_row(self.cursor_row, 0, usize::MAX),
            },
            'h' | 'l' => {
                // DEC private modes such as `?25l` / `?25h` hide/show the
                // cursor. Other modes are not emulated.
                if raw.first() == Some(&b'?')
                    && params.first() == Some(&25)
                    && final_byte == 'h'
                {
                    self.cursor_visible = false;
                } else if raw.first() == Some(&b'?')
                    && params.first() == Some(&25)
                    && final_byte == 'l'
                {
                    self.cursor_visible = true;
                }
            }
            _ => {}
        }
    }

    fn apply_color_params(&mut self, params: &[u32]) {
        let mut index = 0;
        if params.is_empty() {
            self.reset_attributes();
            return;
        }
        while index < params.len() {
            let value = params[index];
            match value {
                0 => self.reset_attributes(),
                1 => self.last_bold = true,
                22 => self.last_bold = false,
                39 => self.last_color.clear(),
                30..=37 => {
                    self.last_color = xterm_color(value - 30).unwrap_or_default();
                }
                90..=97 => {
                    self.last_color = xterm_color(value - 90 + 8).unwrap_or_default();
                }
                40..=47 => {
                    // Background is not rendered by the simple view; ignore.
                }
                38 => {
                    // Extended: 38;5;<n> or 38;2;r;g;b
                    if index + 1 < params.len() && params[index + 1] == 5 {
                        if index + 2 < params.len() {
                            self.last_color =
                                xterm_color(params[index + 2]).unwrap_or_default();
                        }
                        index += 2;
                    } else if index + 1 < params.len() && params[index + 1] == 2 {
                        if index + 4 < params.len() {
                            self.last_color = format!(
                                "#{:02x}{:02x}{:02x}",
                                params[index + 2],
                                params[index + 3],
                                params[index + 4]
                            );
                        }
                        index += 4;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn reset_attributes(&mut self) {
        self.last_color.clear();
        self.last_bold = false;
    }

    /// Return the rendered view. The caller owns the result; the internal
    /// state is advanced so the next feed continues from the cursor.
    pub fn render(&self) -> VtRender {
        let mut lines: Vec<crate::VtLine> = Vec::with_capacity(self.scrollback.len() + self.rows.len());
        lines.extend(self.scrollback.iter().cloned());

        // Include every line the shell has touched plus the cursor row. Empty
        // lines in between are kept as a single space so their height survives
        // the Slint `VerticalLayout`.
        let last_written = self
            .rows
            .iter()
            .rposition(|row| !Self::line_text(row).trim().is_empty())
            .unwrap_or(0);
        let end = (last_written + 1)
            .max(self.cursor_row.saturating_add(1))
            .min(self.rows.len());

        for row_index in 0..end {
            let row = &self.rows[row_index];
            let is_cursor = row_index == self.cursor_row && self.cursor_visible;
            let mut cells: Vec<crate::TerminalCell> = row
                .iter()
                .map(|cell| crate::TerminalCell {
                    text: cell.text.clone(),
                    color: parse_brush(cell.color.as_str()),
                    bold: cell.bold,
                })
                .collect();
            if cells.is_empty() {
                cells.push(crate::TerminalCell {
                    text: slint::SharedString::from(" "),
                    color: parse_brush(""),
                    bold: false,
                });
            }
            if is_cursor {
                let cursor = crate::TerminalCell {
                    text: slint::SharedString::from("█"),
                    color: parse_brush("#34d399"),
                    bold: true,
                };
                if self.cursor_col < cells.len() {
                    cells[self.cursor_col] = cursor;
                } else {
                    while cells.len() < self.cursor_col {
                        cells.push(crate::TerminalCell {
                            text: slint::SharedString::from(" "),
                            color: parse_brush(""),
                            bold: false,
                        });
                    }
                    cells.push(cursor);
                }
            }
            lines.push(crate::VtLine {
                cells: slint::ModelRc::new(slint::VecModel::from(cells)),
                kind: if is_cursor { "cursor".into() } else { "output".into() },
            });
        }

        VtRender {
            lines,
            cursor_row: self.scrollback.len() + self.cursor_row,
            cursor_col: self.cursor_col,
        }
    }

    /// Keystrokes explicitly queued by a caller. Shell output is never queued
    /// here (see the field documentation), so with the current bridge this
    /// returns an empty string and `terminal-input(data)` is used instead.
    pub fn take_input(&mut self) -> String {
        std::mem::take(&mut self.input)
    }
}

impl Terminal {
    /// Convenience for the `RefCell`-wrapped instance used by the page.
    #[allow(dead_code)]
    pub fn shared() -> Rc<RefCell<Terminal>> {
        Rc::new(RefCell::new(Self::new()))
    }
}
