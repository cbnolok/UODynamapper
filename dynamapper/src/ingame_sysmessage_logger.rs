use std::collections::VecDeque;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};
use bevy::prelude::Color;

#[derive(Clone, Debug)]
pub struct InGameLog {
    pub symbol: &'static str,
    pub message: String,
    pub color: Color,
    pub timestamp: Instant,
}

static IN_GAME_LOGS: OnceLock<RwLock<VecDeque<InGameLog>>> = OnceLock::new();

pub fn get_logs() -> Vec<InGameLog> {
    IN_GAME_LOGS.get_or_init(|| RwLock::new(VecDeque::new()))
        .read()
        .unwrap()
        .iter()
        .cloned()
        .collect()
}

pub fn clear_expired(max_age: Duration) {
    let mut logs = IN_GAME_LOGS.get_or_init(|| RwLock::new(VecDeque::new()))
        .write()
        .unwrap();
    let now = Instant::now();
    while let Some(log) = logs.front() {
        if now.duration_since(log.timestamp) > max_age {
            logs.pop_front();
        } else {
            break;
        }
    }
}

pub fn normal(msg: impl Into<String>) {
    push("ℹ", msg, Color::WHITE);
}

pub fn warning(msg: impl Into<String>) {
    push("⚠", msg, Color::srgb(1.0, 1.0, 0.0)); // Yellow
}

pub fn error(msg: impl Into<String>) {
    push("✘", msg, Color::srgb(1.0, 0.0, 0.0)); // Red
}

pub fn custom(msg: impl Into<String>, color: Color) {
    push("•", msg, color);
}

fn push(symbol: &'static str, msg: impl Into<String>, color: Color) {
    let mut logs = IN_GAME_LOGS.get_or_init(|| RwLock::new(VecDeque::new())).write().unwrap();
    logs.push_back(InGameLog {
        symbol,
        message: msg.into(),
        color,
        timestamp: Instant::now(),
    });
    if logs.len() > 100 {
        logs.pop_front();
    }
}
