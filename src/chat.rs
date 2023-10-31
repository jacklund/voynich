use crate::engine::Connection;
use chrono::{DateTime, Local};
use circular_queue::CircularQueue;

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub date: DateTime<Local>,
    pub id: String,
    pub message: String,
}

impl ChatMessage {
    pub fn new(id: &str, message: String) -> ChatMessage {
        ChatMessage {
            date: Local::now(),
            id: id.to_string(),
            message,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chat {
    connection: Connection,
    messages: CircularQueue<ChatMessage>,
}

impl Chat {
    pub fn new(connection: &Connection) -> Self {
        Self {
            connection: connection.clone(),
            messages: CircularQueue::with_capacity(200), // TODO: Configure this
        }
    }

    pub fn add_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }

    pub fn id(&self) -> String {
        self.connection.id().as_str().to_string()
    }
}

#[derive(Debug, Default, Clone)]
pub struct ChatList {
    list: Vec<Connection>,
    current_index: Option<usize>,
}

impl ChatList {
    fn new() -> Self {
        Self {
            list: Vec::new(),
            current_index: None,
        }
    }

    fn names(&self) -> &Vec<Connection> {
        &self.list
    }

    fn add(&mut self, connection: &Connection) {
        self.list.push(connection.clone());
        self.current_index = Some(self.list.len() - 1);
    }

    fn remove(&mut self, connection: &Connection) {
        if let Some(index) = self.list.iter().position(|t| t == connection) {
            self.list.swap_remove(index);
            if self.list.is_empty() {
                self.current_index = None;
            } else {
                match self.current_index {
                    Some(current) => {
                        if current >= index {
                            self.current_index = Some(current - 1);
                        }
                    }
                    None => {
                        panic!("Current subscription index is None when it shouldn't be");
                    }
                }
            }
        }
    }

    fn current(&self) -> Option<&Connection> {
        match self.current_index {
            Some(index) => self.list.get(index),
            None => None,
        }
    }

    fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    fn next(&mut self) -> Option<&Connection> {
        match self.current_index {
            Some(index) => {
                if index == self.list.len() - 1 {
                    self.current_index = Some(0);
                } else {
                    self.current_index = Some(index + 1);
                }
                self.current()
            }
            None => None,
        }
    }

    fn prev(&mut self) -> Option<&Connection> {
        match self.current_index {
            Some(index) => {
                if index == 0 {
                    self.current_index = Some(self.list.len() - 1);
                } else {
                    self.current_index = Some(index - 1);
                }
                self.current()
            }
            None => None,
        }
    }
}
