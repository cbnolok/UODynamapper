//! Generic parser for Ultima Online `.def` text files.

crate::eyre_imports!();

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub struct DefReader {
    lines: std::io::Lines<BufReader<File>>,
    current_parts: Vec<String>,
    cursor: usize,
}

impl DefReader {
    pub fn new(path: &Path) -> eyre::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            lines: BufReader::new(file).lines(),
            current_parts: Vec::new(),
            cursor: 0,
        })
    }

    pub fn next(&mut self) -> bool {
        while let Some(Ok(line)) = self.lines.next() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            let line = line
                .split('#')
                .next()
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim();
            if line.is_empty() {
                continue;
            }

            let mut parts = Vec::new();
            let mut current = String::new();
            let mut in_group = false;

            for ch in line.chars() {
                match ch {
                    '{' => {
                        if !current.is_empty() {
                            parts.push(std::mem::take(&mut current));
                        }
                        in_group = true;
                        current.push(ch);
                    }
                    '}' => {
                        current.push(ch);
                        parts.push(std::mem::take(&mut current));
                        in_group = false;
                    }
                    _ if ch.is_whitespace() && !in_group => {
                        if !current.is_empty() {
                            parts.push(std::mem::take(&mut current));
                        }
                    }
                    _ => current.push(ch),
                }
            }

            if !current.is_empty() {
                parts.push(current);
            }
            if parts.is_empty() {
                continue;
            }

            self.current_parts = parts;
            self.cursor = 0;
            return true;
        }

        false
    }

    pub fn parts_count(&self) -> usize {
        self.current_parts.len()
    }

    pub fn read_int(&mut self) -> i32 {
        if self.cursor >= self.current_parts.len() {
            return -1;
        }

        let value = parse_def_int(&self.current_parts[self.cursor]).unwrap_or(-1);
        self.cursor += 1;
        value
    }

    pub fn read_group(&mut self) -> Option<Vec<i32>> {
        if self.cursor >= self.current_parts.len() {
            return None;
        }

        let part = &self.current_parts[self.cursor];
        if !part.starts_with('{') || !part.ends_with('}') {
            return None;
        }

        self.cursor += 1;
        let content = &part[1..part.len() - 1];
        let values = content
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|part| !part.is_empty())
            .map(|part| parse_def_int(part).unwrap_or(0))
            .collect();
        Some(values)
    }
}

fn parse_def_int(value: &str) -> Option<i32> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let parsed = if let Some(hex) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        i32::from_str_radix(hex, 16).ok()?
    } else {
        digits.parse::<i32>().ok()?
    };

    Some(if negative { -parsed } else { parsed })
}
