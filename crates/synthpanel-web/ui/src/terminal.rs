// Copyright (C) 2026 Binkap S
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A minimal VT100/xterm-compatible virtual terminal.
//!
//! The browser pipe feeds the terminal a raw stream of bytes; the shell
//! speaks `xterm-256color`. This module keeps just enough of the terminal
//! state machine — cursor, line buffer, scrollback, colors, erase and
//! insert — to render the stream as styled lines without xterm.js.

use std::cell::RefCell;
use std::rc::Rc;

/// A styled run of text inside a terminal line.
#[derive(Clone, Default)]
struct Cell {
    text: slint::SharedString,
    color: slint::SharedString,
    bold: bool,
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

/// The rendered view of the terminal: lines plus the live cursor row.
pub struct VtRender {
    pub lines: Vec<crate::VtLine>,
    #[allow(dead_code)]
    pub cursor_row: usize,
}

const MAX_LINES: usize = 1000;

#[allow(dead_code)]
fn parse_color(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 {
            return Some(format!("#{hex}"));
        }
        return None;
    }
    // Named colors fall through to the theme's own palette via `default`.
    Some("default".to_string())
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
    last_color: String,
    last_bold: bool,
    in_escape: bool,
    escape_buffer: Vec<u8>,
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
            last_color: String::new(),
            last_bold: false,
            in_escape: false,
            escape_buffer: Vec::new(),
            input: String::new(),
        }
    }

    fn current_line(&mut self) -> &mut Vec<Cell> {
        if self.rows.len() == 0 {
            self.rows.push(Vec::new());
        }
        self.rows.last_mut().expect("a row was just created")
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
            if self.in_escape {
                self.escape_buffer.push(ch as u8);
                if let Some(cell) = self.finish_escape() {
                    self.push_cell(cell);
                }
                continue;
            }
            if ch == '\x1b' {
                self.in_escape = true;
                self.escape_buffer.clear();
                continue;
            }
            match ch {
                '\n' => {
                    self.input.push('\n');
                    self.newline();
                }
                '\r' => {
                    self.input.push('\r');
                    self.cursor_col = 0;
                }
                '\x08' => {
                    // Backspace: delete the last cell when possible.
                    if self.cursor_col > 0 {
                        self.cursor_col -= 1;
                        let col = self.cursor_col;
                        let line = self.current_line();
                        if line.len() > col {
                            line.remove(col);
                        }
                    }
                    self.input.push('\x08');
                }
                '\t' => {
                    // Expand the tab to the next 8 column stop.
                    let stops = 8;
                    let target = ((self.cursor_col / stops) + 1) * stops;
                    while self.cursor_col < target {
                        self.push_cell(Cell::default());
                    }
                    self.input.push('\t');
                }
                _ => {
                    if ch.is_control() {
                        continue;
                    }
                    self.push_cell(Cell {
                        text: slint::SharedString::from(ch.to_string()),
                        color: slint::SharedString::from(self.last_color.clone()),
                        bold: self.last_bold,
                    });
                    self.input.push(ch);
                }
            }
        }
    }

    fn newline(&mut self) {
        self.cursor_row += 1;
        if self.cursor_row >= self.rows.len() {
            self.rows.push(Vec::new());
        }
        self.cursor_col = 0;
        self.trim_scrollback();
    }

    fn trim_scrollback(&mut self) {
        if self.rows.len() > MAX_LINES {
            let excess = self.rows.len() - MAX_LINES;
            for line in self.rows.drain(..excess) {
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
        if self.scrollback.len() > MAX_LINES {
            let excess = self.scrollback.len() - MAX_LINES;
            self.scrollback.drain(..excess);
        }
    }

    fn push_cell(&mut self, cell: Cell) {
        let line = self.current_line();
        line.push(cell);
        self.cursor_col += 1;
    }

    /// Resolve a pending `\x1b[...m` or similar sequence. Returns a cell to
    /// render (for cursor moves that need a marker) or `None`.
    fn finish_escape(&mut self) -> Option<Cell> {
        let buffer: Vec<u8> = self.escape_buffer.drain(..).collect();
        self.in_escape = false;
        if buffer.is_empty() {
            return None;
        }

        // CSI sequences: \x1b [ <params> <final>
        if buffer.first() == Some(&b'[') {
            let body: Vec<u8> = buffer[1..].to_vec();
            if body.last() != Some(&b'm') {
                return None;
            }
            let params_str: String = body[..body.len() - 1]
                .iter()
                .map(|byte| *byte as char)
                .collect();
            let params: Vec<u32> = params_str
                .split(|c: char| !c.is_ascii_digit())
                .filter(|value| !value.is_empty())
                .filter_map(|value| value.parse::<u32>().ok())
                .collect();
            self.apply_color_params(&params);
            return None;
        }

        // A couple of other single-character escapes we may see.
        match buffer.last()? {
            b'A' => {
                // Cursor up.
                self.cursor_row = self.cursor_row.saturating_sub(1);
            }
            b'B' => {
                // Cursor down.
                self.cursor_row += 1;
                if self.cursor_row >= self.rows.len() {
                    self.rows.push(Vec::new());
                }
            }
            b'J' => {
                // Erase in display; keep the lines but mark them cleared.
            }
            _ => {}
        }
        None
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
        let mut lines: Vec<crate::VtLine> = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let text = Self::line_text(row);
            if text.trim().is_empty() && row.iter().all(|cell| cell.text.as_str().is_empty()) {
                continue;
            }
            lines.push(crate::VtLine {
                cells: slint::ModelRc::new(slint::VecModel::from(
                    row.iter()
                        .map(|cell| crate::TerminalCell {
                            text: cell.text.clone(),
                            color: parse_brush(cell.color.as_str()),
                            bold: cell.bold,
                        })
                        .collect::<Vec<_>>(),
                )),
                kind: "output".into(),
            });
        }
        VtRender {
            lines,
            cursor_row: self.cursor_row,
        }
    }

    /// The characters typed since the last read; the page forwards these to
    /// the WebSocket so the shell sees the keystrokes.
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


