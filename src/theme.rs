use ratatui::prelude::*;

use crate::logger::{Level, LogMessage};
use std::collections::HashMap;

pub struct SystemMessage {
    pub date: Style,
    pub message: Style,
}

pub struct ChatMessage {
    pub date: Style,
    pub message_id_colors: Vec<Color>,
    pub message: Style,
}

pub struct ChatTabs {
    pub style: Style,
    pub highlight_style: Style,
}

pub struct InputPanel {
    pub style: Style,
    pub title: Style,
    pub border: Style,
}

pub struct Theme {
    pub root: Style,
    pub title_bar: Style,
    pub system_messages_panel: Style,
    pub chat_panel: Style,
    pub chat_input: Style,
    pub chat_message: ChatMessage,
    pub chat_tabs: ChatTabs,
    pub status_bar: Style,
    pub input_panel: InputPanel,
}

impl Theme {
    pub fn get_system_message_style(message: &LogMessage) -> SystemMessage {
        SystemMessage {
            date: Style::default().fg(Color::DarkGray),
            message: Style::default().fg(*SYSTEM_MESSAGE_COLORS.get(&message.level).unwrap()),
        }
    }
}

lazy_static::lazy_static! {
    pub static ref THEME: Theme = Theme {
        root: Style::new().bg(DARK_BLUE),
        title_bar: Style::new().fg(Color::White).bg(Color::Magenta),
        system_messages_panel: Style::new().fg(Color::White),
        chat_panel: Style::new().fg(Color::White),
        chat_input: Style::new().fg(Color::White),
        input_panel: InputPanel {
            style: Style::new().fg(Color::White),
            title: Style::new().fg(Color::Blue),
            border: Style::new().fg(Color::Green),
        },
        chat_message: ChatMessage {
            date: Style::new().fg(Color::DarkGray),
            message_id_colors: vec![Color::Cyan, Color::Red, Color::Yellow, Color::Green, Color::Magenta],
            message: Style::new().fg(Color::White),
        },
        chat_tabs: ChatTabs {
            style: Style::new().fg(Color::White),
            highlight_style: Style::new().fg(Color::Yellow),
        },
        status_bar: Style::new().bg(Color::Blue),
    };
}

lazy_static::lazy_static! {
    static ref SYSTEM_MESSAGE_COLORS: HashMap<Level, Color> = HashMap::from([
        (Level::Debug, Color::Yellow),
        (Level::Info, Color::Green),
        (Level::Warning, Color::Rgb(255, 127, 0)),
        (Level::Error, Color::Red)
    ]);
}

const DARK_BLUE: Color = Color::Rgb(16, 24, 48);
// const LIGHT_BLUE: Color = Color::Rgb(64, 96, 192);
// const LIGHT_YELLOW: Color = Color::Rgb(192, 192, 96);
// const LIGHT_GREEN: Color = Color::Rgb(64, 192, 96);
// const LIGHT_RED: Color = Color::Rgb(192, 96, 96);
// const RED: Color = Color::Indexed(160);
// const BLACK: Color = Color::Indexed(232); // not really black, often #080808
// const DARK_GRAY: Color = Color::Indexed(238);
// const MID_GRAY: Color = Color::Indexed(244);
// const LIGHT_GRAY: Color = Color::Indexed(250);
// const WHITE: Color = Color::Indexed(255); // not really white, often #eeeeee
