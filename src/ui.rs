use crate::engine::Engine;
use chrono::{DateTime, Local};
use circular_queue::CircularQueue;
use clap::{crate_name, crate_version};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use crossterm::ExecutableCommand;
use std::net::SocketAddr;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tui::style::Color;
use tui::style::{Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use tui::Frame;
use tui::Terminal;

use std::collections::HashMap;
use std::io::Write;

enum Level {
    Info,
    Warning,
    Error,
}

struct LogMessage {
    date: DateTime<Local>,
    level: Level,
    message: String,
}

pub enum CursorMovement {
    Left,
    Right,
    Start,
    End,
}

pub enum ScrollMovement {
    Up,
    Down,
    Start,
}

pub enum InputEvent {
    Message { sender: String, message: Vec<u8> },
    Shutdown,
}

// split messages to fit the width of the ui panel
pub fn split_each(input: String, width: usize) -> Vec<String> {
    let mut splitted = Vec::with_capacity(input.width() / width);
    let mut row = String::new();

    let mut index = 0;

    for current_char in input.chars() {
        if (index != 0 && index == width) || index + current_char.width().unwrap_or(0) > width {
            splitted.push(row.drain(..).collect());
            index = 0;
        }

        row.push(current_char);
        index += current_char.width().unwrap_or(0);
    }
    // leftover
    if !row.is_empty() {
        splitted.push(row.drain(..).collect());
    }
    splitted
}

pub struct Renderer {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl Renderer {
    pub fn new() -> Self {
        terminal::enable_raw_mode().expect("Error: unable to put terminal in raw mode");
        let mut out = std::io::stdout();
        out.execute(terminal::EnterAlternateScreen).unwrap();

        Self {
            terminal: Terminal::new(CrosstermBackend::new(out)).unwrap(),
        }
    }

    pub fn render(&mut self, ui: &UI) -> Result<(), std::io::Error> {
        self.terminal.draw(|frame| ui.draw(frame, frame.size()))?;
        Ok(())
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.terminal
            .backend_mut()
            .execute(terminal::LeaveAlternateScreen)
            .expect("Could not execute LeaveAlternateScreen");
        terminal::disable_raw_mode().expect("Failed disabling raw mode");
    }
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub date: DateTime<Local>,
    pub address: SocketAddr,
    pub message: String,
}

impl ChatMessage {
    pub fn new(address: SocketAddr, message: String) -> ChatMessage {
        ChatMessage {
            date: Local::now(),
            address,
            message,
        }
    }
}

struct Chat {
    address: SocketAddr,
    messages: CircularQueue<ChatMessage>,
}

impl Chat {
    pub fn new(address: SocketAddr) -> Self {
        Self {
            address,
            messages: CircularQueue::with_capacity(200), // TODO: Configure this
        }
    }

    pub fn add_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }
}

struct ChatList {
    list: Vec<String>,
    current_index: Option<usize>,
}

impl ChatList {
    fn new() -> Self {
        Self {
            list: Vec::new(),
            current_index: None,
        }
    }

    fn contains(&self, address: &String) -> bool {
        self.list.contains(address)
    }

    fn names(&self) -> &Vec<String> {
        &self.list
    }

    fn add(&mut self, address: &SocketAddr) {
        self.list.push(address.to_string());
        self.current_index = Some(self.list.len() - 1);
    }

    fn remove(&mut self, address: &SocketAddr) {
        match self.list.iter().position(|t| t == &address.to_string()) {
            Some(index) => {
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
            None => {}
        }
    }

    fn current(&self) -> Option<&String> {
        match self.current_index {
            Some(index) => self.list.get(index),
            None => None,
        }
    }

    fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    fn next(&mut self) -> Option<&String> {
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

    fn prev(&mut self) -> Option<&String> {
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

pub struct UI {
    log_messages: CircularQueue<LogMessage>,
    chats: HashMap<String, Chat>,
    chat_list: ChatList,
    connect_list: Vec<String>,
    scroll_messages_view: usize,
    input: Vec<char>,
    input_cursor: usize,
    message_colors: Vec<Color>,
    my_user_color: Color,
    date_color: Color,
    chat_panel_color: Color,
    input_panel_color: Color,
    discovery_methods: String,
    nat_traversal_methods: String,
}

impl UI {
    pub fn new() -> Self {
        Self {
            log_messages: CircularQueue::with_capacity(200),
            chats: HashMap::new(),
            chat_list: ChatList::new(),
            connect_list: Vec::new(),
            scroll_messages_view: 0,
            input: Vec::new(),
            input_cursor: 0,
            message_colors: vec![Color::Blue, Color::Yellow, Color::Cyan, Color::Magenta],
            my_user_color: Color::Green,
            date_color: Color::DarkGray,
            chat_panel_color: Color::White,
            input_panel_color: Color::White,
            discovery_methods: String::new(),
            nat_traversal_methods: String::new(),
        }
    }

    pub fn scroll_messages_view(&self) -> usize {
        self.scroll_messages_view
    }

    pub fn ui_input_cursor(&self, width: usize) -> (u16, u16) {
        let mut position = (0, 0);

        for current_char in self.input.iter().take(self.input_cursor) {
            let char_width = unicode_width::UnicodeWidthChar::width(*current_char).unwrap_or(0);

            position.0 += char_width;

            match position.0.cmp(&width) {
                std::cmp::Ordering::Equal => {
                    position.0 = 0;
                    position.1 += 1;
                }
                std::cmp::Ordering::Greater => {
                    // Handle a char with width > 1 at the end of the row
                    // width - (char_width - 1) accounts for the empty column(s) left behind
                    position.0 -= width - (char_width - 1);
                    position.1 += 1;
                }
                _ => (),
            }
        }

        (position.0 as u16, position.1 as u16)
    }

    pub async fn handle_input_event(
        &mut self,
        engine: &mut Engine,
        event: Event,
    ) -> Result<Option<InputEvent>, Box<dyn std::error::Error>> {
        // debug!("Got input event {:?}", event);
        match event {
            Event::Mouse(_) => Ok(None),
            Event::Resize(_, _) => Ok(None),
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind: _,
                state: _,
            }) => match code {
                KeyCode::Esc => Ok(Some(InputEvent::Shutdown)),
                KeyCode::Char(character) => {
                    if character == 'c' && modifiers.contains(KeyModifiers::CONTROL) {
                        Ok(Some(InputEvent::Shutdown))
                    } else if character == 'u' && modifiers.contains(KeyModifiers::CONTROL) {
                        self.clear_input_to_cursor();
                        Ok(None)
                    } else {
                        self.input_write(character);
                        Ok(None)
                    }
                }
                KeyCode::Enter => {
                    if let Some(input) = self.reset_input() {
                        if let Some(command) = input.strip_prefix('/') {
                            engine
                                .handle_command(command.split_whitespace().collect())
                                .await
                        } else {
                            match self.chat_list.current_index() {
                                Some(_) => {
                                    let address = self.chat_list.current().unwrap();
                                    match self.chats.get_mut(address) {
                                        Some(chat) => {
                                            let address = chat.address.clone();
                                            let message = ChatMessage::new(address, input.clone());
                                            chat.add_message(message);
                                            Ok(Some(InputEvent::Message {
                                                sender: address.to_string(),
                                                message: input.into_bytes(),
                                            }))
                                        }
                                        None => {
                                            self.log_error("No current chat");
                                            Ok(None)
                                        }
                                    }
                                }
                                None => {
                                    self.log_error("No current chat");
                                    Ok(None)
                                }
                            }
                        }
                    } else {
                        Ok(None)
                    }
                }
                KeyCode::Delete => {
                    self.input_remove();
                    Ok(None)
                }
                KeyCode::Backspace => {
                    self.input_remove_previous();
                    Ok(None)
                }
                KeyCode::Left => {
                    if modifiers == KeyModifiers::CONTROL {
                        self.chat_list.prev();
                    } else {
                        self.input_move_cursor(CursorMovement::Left);
                    }
                    Ok(None)
                }
                KeyCode::Right => {
                    if modifiers == KeyModifiers::CONTROL {
                        self.chat_list.next();
                    } else {
                        self.input_move_cursor(CursorMovement::Right);
                    }
                    Ok(None)
                }
                KeyCode::Home => {
                    self.input_move_cursor(CursorMovement::Start);
                    Ok(None)
                }
                KeyCode::End => {
                    self.input_move_cursor(CursorMovement::End);
                    Ok(None)
                }
                KeyCode::Up => {
                    self.messages_scroll(ScrollMovement::Up);
                    Ok(None)
                }
                KeyCode::Down => {
                    self.messages_scroll(ScrollMovement::Down);
                    Ok(None)
                }
                KeyCode::PageUp => {
                    self.messages_scroll(ScrollMovement::Start);
                    Ok(None)
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    pub fn input_write(&mut self, character: char) {
        self.input.insert(self.input_cursor, character);
        self.input_cursor += 1;
    }

    pub fn input_remove(&mut self) {
        if self.input_cursor < self.input.len() {
            self.input.remove(self.input_cursor);
        }
    }

    pub fn input_remove_previous(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            self.input.remove(self.input_cursor);
        }
    }

    pub fn input_move_cursor(&mut self, movement: CursorMovement) {
        match movement {
            CursorMovement::Left => {
                if self.input_cursor > 0 {
                    self.input_cursor -= 1;
                }
            }
            CursorMovement::Right => {
                if self.input_cursor < self.input.len() {
                    self.input_cursor += 1;
                }
            }
            CursorMovement::Start => {
                self.input_cursor = 0;
            }
            CursorMovement::End => {
                self.input_cursor = self.input.len();
            }
        }
    }

    pub fn messages_scroll(&mut self, movement: ScrollMovement) {
        match movement {
            ScrollMovement::Up => {
                if self.scroll_messages_view > 0 {
                    self.scroll_messages_view -= 1;
                }
            }
            ScrollMovement::Down => {
                self.scroll_messages_view += 1;
            }
            ScrollMovement::Start => {
                self.scroll_messages_view += 0;
            }
        }
    }

    pub fn clear_input_to_cursor(&mut self) {
        if !self.input.is_empty() {
            self.input.drain(..self.input_cursor);
            self.input_cursor = 0;
        }
    }

    pub fn reset_input(&mut self) -> Option<String> {
        if !self.input.is_empty() {
            self.input_cursor = 0;
            return Some(self.input.drain(..).collect());
        }
        None
    }

    fn log_message(&mut self, level: Level, message: String) {
        self.log_messages.push(LogMessage {
            date: Local::now(),
            level,
            message,
        });
    }

    pub fn log_error(&mut self, message: &str) {
        self.log_message(Level::Error, format!("ERROR: {}", message));
    }

    pub fn log_warning(&mut self, message: &str) {
        self.log_message(Level::Warning, format!("WARNING: {}", message));
    }

    pub fn log_info(&mut self, message: &str) {
        self.log_message(Level::Info, format!("INFO: {}", message));
    }

    pub fn draw(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        // debug!("UI::draw called");
        match self.chat_list.current() {
            Some(_) => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(
                        [
                            Constraint::Length(1),
                            Constraint::Percentage(20),
                            Constraint::Length(3),
                            Constraint::Min(1),
                            Constraint::Length(1),
                            Constraint::Length(1),
                        ]
                        .as_ref(),
                    )
                    .split(chunk);

                self.draw_title_bar(frame, chunks[0]);
                self.draw_system_messages_panel(frame, chunks[1]);
                self.draw_chat_tabs(frame, chunks[2]);
                self.draw_chat_panel(frame, chunks[3]);
                self.draw_status_bar(frame, chunks[4]);
                self.draw_input_panel(frame, chunks[5]);
            }
            None => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(
                        [
                            Constraint::Length(1),
                            Constraint::Min(1),
                            Constraint::Length(1),
                            Constraint::Length(1),
                        ]
                        .as_ref(),
                    )
                    .split(chunk);

                self.draw_title_bar(frame, chunks[0]);
                self.draw_system_messages_panel(frame, chunks[1]);
                self.draw_status_bar(frame, chunks[2]);
                self.draw_input_panel(frame, chunks[3]);
            }
        }
    }

    fn draw_title_bar(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        let title_bar = Paragraph::new(format!(
            "{} {}  |  Discovery methods: {}  |  Nat traversal methods: {}",
            crate_name!(),
            crate_version!(),
            self.discovery_methods,
            self.nat_traversal_methods,
        ))
        .block(Block::default().borders(Borders::NONE))
        .style(Style::default().bg(Color::Blue))
        .alignment(Alignment::Left);

        frame.render_widget(title_bar, chunk);
    }

    fn draw_system_messages_panel(
        &self,
        frame: &mut Frame<CrosstermBackend<impl Write>>,
        chunk: Rect,
    ) {
        let messages = self
            .log_messages
            .asc_iter()
            .map(|message| {
                let date = message.date.format("%H:%M:%S ").to_string();
                let color = match message.level {
                    Level::Info => Color::Gray,
                    Level::Warning => Color::Rgb(255, 127, 0),
                    Level::Error => Color::Red,
                };
                let ui_message = vec![
                    Span::styled(date, Style::default().fg(self.date_color)),
                    Span::styled(message.message.clone(), Style::default().fg(color)),
                ];
                Spans::from(ui_message)
            })
            .collect::<Vec<_>>();

        let messages_panel = Paragraph::new(messages)
            .block(Block::default().borders(Borders::ALL).title(Span::styled(
                "System Messages",
                Style::default().add_modifier(Modifier::BOLD),
            )))
            .style(Style::default().fg(self.chat_panel_color))
            .alignment(Alignment::Left)
            .scroll((self.scroll_messages_view() as u16, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(messages_panel, chunk);
    }

    fn draw_chat_tabs(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        let tabs = Tabs::new(
            self.chat_list
                .names()
                .iter()
                .map(|s| Spans::from(s.clone()))
                .collect(),
        )
        .block(Block::default().title("Chats").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow))
        .select(self.chat_list.current_index().unwrap());

        frame.render_widget(tabs, chunk);
    }

    fn draw_chat_panel(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        match self.chat_list.current() {
            Some(address) => {
                let chat = self.chats.get(address).unwrap();
                let messages = chat
                    .messages
                    .asc_iter()
                    .map(|message| {
                        let date = message.date.format("%H:%M:%S ").to_string();
                        let mut ui_message = vec![
                            Span::styled(date, Style::default().fg(self.date_color)),
                            Span::styled(
                                message.address.to_string(),
                                Style::default().fg(Color::Blue),
                            ),
                            Span::styled(": ", Style::default().fg(Color::Blue)),
                        ];
                        ui_message.extend(Self::parse_content(&message.message));
                        Spans::from(ui_message)
                    })
                    .collect::<Vec<_>>();

                let chat_panel = Paragraph::new(messages)
                    .block(Block::default().borders(Borders::ALL).title(Span::styled(
                        chat.address.to_string().clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    )))
                    .style(Style::default().fg(self.chat_panel_color))
                    .alignment(Alignment::Left)
                    .scroll((self.scroll_messages_view() as u16, 0))
                    .wrap(Wrap { trim: false });

                frame.render_widget(chat_panel, chunk);
            }
            None => (),
        }
    }

    fn draw_status_bar(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        let status_bar = Paragraph::new("Input")
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().bg(Color::Blue))
            .alignment(Alignment::Left);

        frame.render_widget(status_bar, chunk);
    }

    fn parse_content(content: &str) -> Vec<Span> {
        vec![Span::raw(content)]
    }

    fn draw_input_panel(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        let inner_width = (chunk.width - 2) as usize;

        let input = self.input.iter().collect::<String>();
        let input = split_each(input, inner_width)
            .into_iter()
            .map(|line| Spans::from(vec![Span::raw(line)]))
            .collect::<Vec<_>>();

        let input_panel = Paragraph::new(input)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(self.input_panel_color))
            .alignment(Alignment::Left);

        frame.render_widget(input_panel, chunk);

        let input_cursor = self.ui_input_cursor(inner_width);
        frame.set_cursor(chunk.x + input_cursor.0, chunk.y + input_cursor.1)
    }
}
