use chrono::{DateTime, Local};
use circular_queue::CircularQueue;
use tor_client_lib::TorServiceId;

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub date: DateTime<Local>,
    pub sender: String,
    pub recipient: String,
    pub message: String,
}

impl ChatMessage {
    pub fn new(sender: String, recipient: String, message: String) -> ChatMessage {
        ChatMessage {
            date: Local::now(),
            sender,
            recipient,
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
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            current_index: None,
        }
    }

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

    pub fn current(&self) -> Option<&TorServiceId> {
        match self.current_index {
            Some(index) => self.list.get(index),
            None => None,
        }
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn next(&mut self) -> Option<&TorServiceId> {
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

    pub fn prev(&mut self) -> Option<&TorServiceId> {
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
