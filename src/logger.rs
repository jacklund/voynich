use chrono::{DateTime, Local};
use circular_queue::CircularQueue;

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug)]
pub struct LogMessage {
    pub date: DateTime<Local>,
    pub level: Level,
    pub message: String,
}

impl LogMessage {
    pub fn new(level: Level, message: &str) -> Self {
        Self {
            date: Local::now(),
            level,
            message: message.to_string(),
        }
    }
}

pub trait Logger: Send + Sync {
    fn log(&mut self, message: LogMessage);

    fn set_log_level(&mut self, level: Level);

    fn log_message(&mut self, level: Level, message: String) {
        self.log(LogMessage::new(level, &message));
    }

    fn log_error(&mut self, message: &str) {
        self.log_message(Level::Error, format!("ERROR: {}", message));
    }

    fn log_warning(&mut self, message: &str) {
        self.log_message(Level::Warning, format!("WARNING: {}", message));
    }

    fn log_info(&mut self, message: &str) {
        self.log_message(Level::Info, format!("INFO: {}", message));
    }

    fn log_debug(&mut self, message: &str) {
        self.log_message(Level::Debug, format!("DEBUG: {}", message));
    }
}

pub trait LogIterator {
    fn iter(&self) -> Box<dyn Iterator<Item = &LogMessage> + '_>;
}

pub trait LoggerPlusIterator: Logger + LogIterator {}

pub struct StandardLogger {
    log_messages: CircularQueue<LogMessage>,
    log_level: Level,
}

impl StandardLogger {
    pub fn new(capacity: usize) -> Self {
        Self {
            log_messages: CircularQueue::with_capacity(capacity),
            log_level: Level::Info,
        }
    }
}

impl LogIterator for StandardLogger {
    fn iter(&self) -> Box<dyn Iterator<Item = &LogMessage> + '_> {
        Box::new(self.log_messages.asc_iter())
    }
}

impl Logger for StandardLogger {
    fn log(&mut self, message: LogMessage) {
        if message.level >= self.log_level {
            self.log_messages.push(message);
        }
    }

    fn set_log_level(&mut self, level: Level) {
        self.log_level = level;
    }
}

impl<T> LoggerPlusIterator for T where T: std::any::Any + Logger + LogIterator {}
