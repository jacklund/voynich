use crate::{
    engine::{Connection, Engine},
    logger::{Level, LogMessage, Logger},
};
use async_trait::async_trait;
use chrono::{DateTime, Local};
use circular_queue::CircularQueue;
use clap::{crate_name, crate_version};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use crossterm::ExecutableCommand;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{block::Title, Block, BorderType, Borders, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use ratatui::Terminal;

use std::collections::HashMap;
use std::io::Write;

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
    Message {
        recipient: Box<Connection>,
        message: String,
    },
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

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn show_help_popup(frame: &mut Frame<CrosstermBackend<impl Write>>) {
    show_centered_popup(
        frame,
        Title::from(Line::styled(
            "Welcome",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        vec![Line::from(format!(
            "Welcome to {} version {}",
            crate_name!(),
            crate_version!()
        ))],
        75,
        60,
    );
}

fn show_centered_popup<'a, T, L>(
    frame: &mut Frame<CrosstermBackend<impl Write>>,
    title: T,
    text: L,
    percent_x: u16,
    percent_y: u16,
) where
    T: Into<Title<'a>>,
    L: Into<Text<'a>>,
{
    let paragraph = Paragraph::new(text).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Green)),
    );
    let area = centered_rect(percent_x, percent_y, frame.size());
    frame.render_widget(ratatui::widgets::Clear, area); //this clears out the background
    frame.render_widget(paragraph, area);
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

struct Chat {
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

struct ChatList {
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

    fn contains(&self, connection: &Connection) -> bool {
        self.list.contains(connection)
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
    log_level: Level,
    show_popup: bool,
}

#[async_trait]
impl Logger for UI {
    async fn log(&mut self, message: LogMessage) {
        if message.level >= self.log_level {
            self.log_messages.push(message);
        }
    }

    fn set_log_level(&mut self, level: Level) {
        self.log_level = level;
    }
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
            log_level: Level::Info,
            show_popup: false,
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

    pub fn add_chat(&mut self, connection: &Connection) {
        self.chat_list.add(connection);
        self.chats
            .insert(connection.id().as_str().to_string(), Chat::new(connection));
    }

    pub fn remove_chat(&mut self, connection: &Connection) {
        self.chat_list.remove(connection);
        self.chats.remove(connection.id().as_str());
    }

    pub async fn handle_input_event(
        &mut self,
        engine: &mut Engine,
        event: Event,
    ) -> Result<Option<InputEvent>, anyhow::Error> {
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
                KeyCode::Esc => {
                    self.show_popup = !self.show_popup;
                    Ok(None)
                }
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
                            match engine
                                .handle_command(self, command.split_whitespace().collect())
                                .await
                            {
                                Ok(option) => Ok(option),
                                Err(error) => {
                                    self.log_error(&format!(
                                        "Error on command '{}': {}",
                                        command, error
                                    ))
                                    .await;
                                    Ok(None)
                                }
                            }
                        } else {
                            match self.chat_list.current_index() {
                                Some(_) => {
                                    let connection = self.chat_list.current().unwrap();
                                    match self.chats.get_mut(connection.id().as_str()) {
                                        Some(chat) => {
                                            let message =
                                                ChatMessage::new(engine.id(), input.clone());
                                            chat.add_message(message);
                                            Ok(Some(InputEvent::Message {
                                                recipient: Box::new(connection.clone()),
                                                message: input,
                                            }))
                                        }
                                        None => {
                                            self.log_error("No current chat").await;
                                            Ok(None)
                                        }
                                    }
                                }
                                None => {
                                    self.log_error("No current chat").await;
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

    pub fn add_message(&mut self, message: ChatMessage) {
        if let Some(chat) = self.chats.get_mut(&message.id) {
            chat.add_message(message);
        }
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
                            // Constraint::Length(1),
                            // Constraint::Length(1),
                        ]
                        .as_ref(),
                    )
                    .split(chunk);

                self.draw_title_bar(frame, chunks[0]);
                self.draw_system_messages_panel(frame, chunks[1]);
                // self.draw_status_bar(frame, chunks[2]);
                // self.draw_input_panel(frame, chunks[3]);
            }
        }
        if self.show_popup {
            show_help_popup(frame);
        }
    }

    fn draw_title_bar(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        let title_bar = Paragraph::new(format!("{} {}", crate_name!(), crate_version!(),))
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().bg(Color::Magenta))
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
                    Level::Debug => Color::Yellow,
                    Level::Info => Color::Gray,
                    Level::Warning => Color::Rgb(255, 127, 0),
                    Level::Error => Color::Red,
                };
                let ui_message = vec![
                    Span::styled(date, Style::default().fg(self.date_color)),
                    Span::styled(message.message.clone(), Style::default().fg(color)),
                ];
                Line::from(ui_message)
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
                .map(|s| Line::from(s.id().as_str().to_string()))
                .collect(),
        )
        .block(Block::default().title("Chats").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow))
        .select(self.chat_list.current_index().unwrap());

        frame.render_widget(tabs, chunk);
    }

    fn draw_chat_panel(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        if let Some(connection) = self.chat_list.current() {
            let chat = self.chats.get(connection.id().as_str()).unwrap();
            let messages = chat
                .messages
                .asc_iter()
                .map(|message| {
                    let date = message.date.format("%H:%M:%S ").to_string();
                    let mut ui_message = vec![
                        Span::styled(date, Style::default().fg(self.date_color)),
                        Span::styled(message.id.clone(), Style::default().fg(Color::Blue)),
                        Span::styled(": ", Style::default().fg(Color::Blue)),
                    ];
                    ui_message.extend(Self::parse_content(&message.message));
                    Line::from(ui_message)
                })
                .collect::<Vec<_>>();

            let chat_panel = Paragraph::new(messages)
                .block(Block::default().borders(Borders::ALL).title(Span::styled(
                    chat.id().clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )))
                .style(Style::default().fg(self.chat_panel_color))
                .alignment(Alignment::Left)
                .scroll((self.scroll_messages_view() as u16, 0))
                .wrap(Wrap { trim: false });

            frame.render_widget(chat_panel, chunk);
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
            .map(|line| Line::from(vec![Span::raw(line)]))
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
