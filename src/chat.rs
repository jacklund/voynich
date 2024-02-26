use chrono::{serde::ts_seconds, DateTime, SubsecRound, Utc};
use circular_queue::CircularQueue;
use serde::{Deserialize, Serialize};
use tor_client_lib::TorServiceId;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ChatMessage {
    #[serde(with = "ts_seconds")]
    pub date: DateTime<Utc>,
    pub sender: TorServiceId,
    pub recipient: TorServiceId,
    pub message: String,
}

impl ChatMessage {
    pub fn new(sender: &TorServiceId, recipient: &TorServiceId, message: String) -> ChatMessage {
        ChatMessage {
            // Current DateTime rounded to second
            date: Utc::now().round_subsecs(0),
            sender: sender.clone(),
            recipient: recipient.clone(),
            message,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chat {
    id: TorServiceId,
    messages: CircularQueue<ChatMessage>,
}

impl Chat {
    pub fn new(id: &TorServiceId) -> Self {
        Self {
            id: id.clone(),
            messages: CircularQueue::with_capacity(200), // TODO: Configure this
        }
    }

    pub fn add_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }

    pub fn id(&self) -> String {
        self.id.as_str().to_string()
    }

    pub fn iter(&self) -> Box<dyn Iterator<Item = &ChatMessage> + '_> {
        Box::new(self.messages.asc_iter())
    }
}

#[derive(Debug, Default, Clone)]
pub struct ChatList {
    list: Vec<TorServiceId>,
    current_index: Option<usize>,
}

impl ChatList {
    pub fn names(&self) -> &Vec<TorServiceId> {
        &self.list
    }

    pub fn add(&mut self, id: &TorServiceId) {
        self.list.push(id.clone());
        self.current_index = Some(self.list.len() - 1);
    }

    pub fn remove(&mut self, id: &TorServiceId) {
        if let Some(index) = self.list.iter().position(|t| t == id) {
            self.list.swap_remove(index);
            if self.list.is_empty() {
                self.current_index = None;
            } else {
                match self.current_index {
                    Some(current) => {
                        if current >= index {
                            if current == 0 {
                                self.current_index = None;
                            } else {
                                self.current_index = Some(current - 1);
                            }
                        }
                    }
                    None => {
                        panic!("Current subscription index is None when it shouldn't be");
                    }
                }
            }
        }
    }

    pub fn current(&self) -> Option<&TorServiceId> {
        match self.current_index {
            Some(index) => self.list.get(index),
            None => None,
        }
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn next_chat(&mut self) -> Option<&TorServiceId> {
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

    pub fn prev_chat(&mut self) -> Option<&TorServiceId> {
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
