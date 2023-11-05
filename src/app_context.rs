use crate::{
    chat::{Chat, ChatList},
    input::Input,
    logger::StandardLogger,
};
use std::collections::HashMap;
use tor_client_lib::key::TorServiceId;

#[derive(Debug)]
pub struct AppContext {
    pub id: TorServiceId,
    pub chat_list: ChatList,
    pub chats: HashMap<TorServiceId, Chat>,
    pub show_command_popup: bool,
    pub system_messages_scroll: usize,
    pub chat_input: Input,
    pub command_input: Input,
    pub cursor_location: Option<(u16, u16)>,
}

impl AppContext {
    pub fn new(id: TorServiceId) -> Self {
        Self {
            id,
            chat_list: ChatList::default(),
            chats: HashMap::default(),
            show_command_popup: false,
            system_messages_scroll: 0,
            chat_input: Input::new(None),
            command_input: Input::new(Some(":> ")),
            cursor_location: None,
        }
    }

    pub fn toggle_command_popup(&mut self) {
        self.show_command_popup = !self.show_command_popup;
    }

    pub fn current_input(&mut self) -> &mut Input {
        if self.show_command_popup {
            &mut self.command_input
        } else {
            &mut self.chat_input
        }
    }
}
