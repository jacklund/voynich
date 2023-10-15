use async_trait::async_trait;
use chrono::{DateTime, Local};

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

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

#[async_trait]
pub trait Logger: Send + Sync {
    async fn log_message(&mut self, level: Level, message: String) {
        self.log(LogMessage::new(level, &message)).await;
    }

    async fn log(&mut self, message: LogMessage);

    fn set_log_level(&mut self, level: Level);

    async fn log_error(&mut self, message: &str) {
        self.log_message(Level::Error, format!("ERROR: {}", message))
            .await;
    }

    async fn log_warning(&mut self, message: &str) {
        self.log_message(Level::Warning, format!("WARNING: {}", message))
            .await;
    }

    async fn log_info(&mut self, message: &str) {
        self.log_message(Level::Info, format!("INFO: {}", message))
            .await;
    }

    async fn log_debug(&mut self, message: &str) {
        self.log_message(Level::Debug, format!("DEBUG: {}", message))
            .await;
    }
}
