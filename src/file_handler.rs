//! File handling for detex - opening files with TEXINPUTS search.

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::config::Options;

/// Try to open a TeX file, searching through input paths.
///
/// For each input path the following order is used:
/// - file.tex - must be as named, if not there go to next path
/// - file.ext - random extension, try it
/// - file     - base name, add .tex and try it
/// - file     - try it as is
///
/// If the file begins with '/', no paths are searched.
pub fn tex_open(filename: &str, opts: &Options) -> Option<(File, PathBuf)> {
    let path = Path::new(filename);

    // Check if absolute path
    if path.is_absolute() {
        return try_open_file(path);
    }

    // Search through input paths
    for input_path in &opts.input_paths {
        let full_path = Path::new(input_path).join(filename);

        // If filename ends in .tex, it must be exactly that
        if filename.ends_with(".tex") {
            if let Some(result) = try_open_file(&full_path) {
                return Some(result);
            }
            continue;
        }

        // If has some other extension, try it
        if let Some(ext) = path.extension() {
            if !ext.is_empty() {
                if let Some(result) = try_open_file(&full_path) {
                    return Some(result);
                }
            }
        }

        // Try adding .tex extension
        let tex_path = full_path.with_extension("tex");
        if let Some(result) = try_open_file(&tex_path) {
            return Some(result);
        }

        // Try as-is
        if let Some(result) = try_open_file(&full_path) {
            return Some(result);
        }
    }

    // If no input paths or nothing found, try current directory
    if opts.input_paths.is_empty() {
        if let Some(result) = try_open_file(path) {
            return Some(result);
        }

        let tex_path = path.with_extension("tex");
        if let Some(result) = try_open_file(&tex_path) {
            return Some(result);
        }
    }

    None
}

fn try_open_file(path: &Path) -> Option<(File, PathBuf)> {
    File::open(path).ok().map(|f| (f, path.to_path_buf()))
}

/// Check if a file is in the includeonly list.
/// If there is no list, all files are considered "in the list".
pub fn in_include_list(filename: &str, opts: &Options) -> bool {
    if opts.include_list.is_empty() {
        return true;
    }

    let base = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);

    opts.include_list.iter().any(|inc| inc == base)
}

/// A buffered character source that supports pushback and line tracking
pub struct CharSource {
    buffer: Vec<char>,
    pos: usize,
    pushback: Vec<char>,
    pub line: usize,
    pub at_line_start: bool,
}

impl CharSource {
    pub fn new(content: String) -> Self {
        Self {
            buffer: content.chars().collect(),
            pos: 0,
            pushback: Vec::new(),
            line: 1,
            at_line_start: true,
        }
    }

    pub fn peek(&self) -> Option<char> {
        if let Some(&c) = self.pushback.last() {
            Some(c)
        } else if self.pos < self.buffer.len() {
            Some(self.buffer[self.pos])
        } else {
            None
        }
    }

    pub fn next(&mut self) -> Option<char> {
        let c = if let Some(c) = self.pushback.pop() {
            c
        } else if self.pos < self.buffer.len() {
            let c = self.buffer[self.pos];
            self.pos += 1;
            c
        } else {
            return None;
        };

        if c == '\n' {
            self.line += 1;
            self.at_line_start = true;
        } else {
            self.at_line_start = false;
        }

        Some(c)
    }

    pub fn unget(&mut self, c: char) {
        if c == '\n' {
            self.line = self.line.saturating_sub(1);
            self.at_line_start = false;
        }
        self.pushback.push(c);
    }

    pub fn is_eof(&self) -> bool {
        self.pushback.is_empty() && self.pos >= self.buffer.len()
    }

    /// Peek ahead at the next n characters without consuming them
    pub fn peek_ahead(&self, n: usize) -> String {
        let mut result = String::new();
        let mut pushback_used = 0;

        // First consume from pushback (in reverse order)
        for i in (0..self.pushback.len()).rev() {
            if result.len() >= n {
                break;
            }
            result.push(self.pushback[self.pushback.len() - 1 - i]);
            pushback_used += 1;
        }

        // Then from buffer
        let start_pos = self.pos;
        for i in 0..(n - pushback_used) {
            if start_pos + i >= self.buffer.len() {
                break;
            }
            result.push(self.buffer[start_pos + i]);
        }

        result
    }
}
