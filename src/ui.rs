use crate::{
    commands::Command,
    engine::{Connection, InputEvent},
    logger::{Level, Logger, LoggerPlusIterator},
};
use async_trait::async_trait;
use chrono::{DateTime, Local};
use circular_queue::CircularQueue;
use clap::{crate_name, crate_version};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use crossterm::ExecutableCommand;
use futures::{
    stream::{FusedStream, Stream},
    task::Poll,
    StreamExt,
};
use futures_lite::StreamExt as LiteStreamExt;
use std::pin::Pin;
use std::str::FromStr;
use std::task::Context;
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
fn centered_rect(constraint_x: Constraint, constraint_y: Constraint, r: Rect) -> Rect {
    let vertical_constraints = match constraint_y {
        Constraint::Percentage(percent_y) => [
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ],
        Constraint::Length(length_y) => [
            Constraint::Min((r.height - length_y) / 2),
            Constraint::Min(length_y),
            Constraint::Min(((r.height - length_y) / 2) - 2),
        ],
        _ => panic!("Expected Length or Percentage, got {}", constraint_y),
    };
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vertical_constraints)
        .split(r);

    let horizontal_constraints = match constraint_x {
        Constraint::Percentage(percent_x) => [
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ],
        Constraint::Length(length_x) => [
            Constraint::Min((r.width - length_x) / 2),
            Constraint::Percentage(length_x),
            Constraint::Min((r.width - length_x) / 2),
        ],
        _ => panic!("Expected Length or Percentage, got {}", constraint_y),
    };
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(horizontal_constraints)
        .split(popup_layout[1])[1]
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

    pub fn render(
        &mut self,
        ui: &mut TerminalUI,
        logger: &mut dyn LoggerPlusIterator,
    ) -> Result<(), std::io::Error> {
        self.terminal
            .draw(|frame| ui.draw(frame, frame.size(), logger))?;
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

#[derive(Debug)]
struct Input {
    buffer: Vec<char>,
    cursor: usize,
    prompt_size: usize,
}

impl Input {
    fn new(prompt: Option<&str>) -> Self {
        let mut buffer = Vec::new();
        let prompt_size = match prompt {
            Some(prompt) => {
                buffer.extend_from_slice(prompt.chars().collect::<Vec<char>>().as_slice());
                prompt.len()
            }
            None => 0,
        };
        Self {
            buffer,
            cursor: prompt_size,
            prompt_size,
        }
    }

    fn get_input(&self) -> String {
        self.buffer[..].iter().collect::<String>()
    }

    fn write(&mut self, character: char) {
        self.buffer.insert(self.cursor, character);
        self.cursor += 1;
    }

    fn remove(&mut self) {
        if self.cursor < self.buffer.len() {
            self.buffer.remove(self.cursor);
        }
    }

    fn remove_previous(&mut self) {
        if self.cursor > self.prompt_size {
            self.cursor -= 1;
            self.buffer.remove(self.cursor);
        }
    }

    fn move_cursor(&mut self, movement: CursorMovement) {
        match movement {
            CursorMovement::Left => {
                if self.cursor > self.prompt_size {
                    self.cursor -= 1;
                }
            }
            CursorMovement::Right => {
                if self.cursor < self.buffer.len() {
                    self.cursor += 1;
                }
            }
            CursorMovement::Start => {
                self.cursor = self.prompt_size;
            }
            CursorMovement::End => {
                self.cursor = self.buffer.len();
            }
        }
    }

    fn clear_input_to_cursor(&mut self) {
        if !self.buffer.is_empty() {
            self.buffer.drain(self.prompt_size..self.cursor);
            self.cursor = self.prompt_size;
        }
    }

    fn reset_input(&mut self) -> Option<String> {
        if !self.buffer.is_empty() {
            self.cursor = self.prompt_size;
            return Some(self.buffer.drain(self.prompt_size..).collect());
        }
        None
    }

    fn cursor_location(&self, width: usize) -> (u16, u16) {
        let mut position = (0, 0);

        for current_char in self.buffer.iter().take(self.cursor) {
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
}

pub struct TermInputStream {
    reader: EventStream,
}

impl TermInputStream {
    fn new() -> Self {
        Self {
            reader: EventStream::new(),
        }
    }
}

impl Stream for TermInputStream {
    type Item = Result<Event, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.reader.poll_next(cx)
    }
}

impl FusedStream for TermInputStream {
    fn is_terminated(&self) -> bool {
        false
    }
}

#[async_trait]
pub trait UI {
    fn render(
        &mut self,
        renderer: &mut Renderer,
        logger: &mut dyn LoggerPlusIterator,
    ) -> Result<(), std::io::Error>;

    async fn get_input_event(
        &mut self,
        logger: &mut dyn LoggerPlusIterator,
    ) -> Result<Option<InputEvent>, anyhow::Error>;

    fn add_chat(&mut self, connection: &Connection);

    fn add_message(&mut self, message: ChatMessage);

    fn remove_chat(&mut self, connection: &Connection);
}

pub struct TerminalUI {
    id: String,
    input_stream: TermInputStream,
    chats: HashMap<String, Chat>,
    chat_list: ChatList,
    scroll_messages_view: usize,
    chat_input: Input,
    command_input: Input,
    message_colors: Vec<Color>,
    my_user_color: Color,
    date_color: Color,
    chat_panel_color: Color,
    input_panel_color: Color,
    log_level: Level,
    show_command_popup: bool,
}

#[async_trait]
impl UI for TerminalUI {
    fn render(
        &mut self,
        renderer: &mut Renderer,
        logger: &mut dyn LoggerPlusIterator,
    ) -> Result<(), std::io::Error> {
        renderer.render(self, logger)
    }

    async fn get_input_event(
        &mut self,
        logger: &mut dyn LoggerPlusIterator,
    ) -> Result<Option<InputEvent>, anyhow::Error> {
        let event = self.input_stream.select_next_some().await?;
        match self.handle_input_event(event, logger).await? {
            Some(input_event) => Ok(Some(input_event)),
            None => Ok(None),
        }
    }

    fn add_chat(&mut self, connection: &Connection) {
        self.chat_list.add(connection);
        self.chats
            .insert(connection.id().as_str().to_string(), Chat::new(connection));
    }

    fn add_message(&mut self, message: ChatMessage) {
        if let Some(chat) = self.chats.get_mut(&message.id) {
            chat.add_message(message);
        }
    }

    fn remove_chat(&mut self, connection: &Connection) {
        self.chat_list.remove(connection);
        self.chats.remove(connection.id().as_str());
    }
}

impl TerminalUI {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            input_stream: TermInputStream::new(),
            chats: HashMap::new(),
            chat_list: ChatList::new(),
            scroll_messages_view: 0,
            chat_input: Input::new(None),
            command_input: Input::new(Some(":> ")),
            message_colors: vec![Color::Blue, Color::Yellow, Color::Cyan, Color::Magenta],
            my_user_color: Color::Green,
            date_color: Color::DarkGray,
            chat_panel_color: Color::White,
            input_panel_color: Color::White,
            log_level: Level::Info,
            show_command_popup: false,
        }
    }

    fn scroll_messages_view(&self) -> usize {
        self.scroll_messages_view
    }

    async fn handle_input_event(
        &mut self,
        event: Event,
        logger: &mut dyn LoggerPlusIterator,
    ) -> Result<Option<InputEvent>, anyhow::Error> {
        logger.log_debug(&format!("Got input event {:?}", event));
        match event {
            Event::Mouse(_) => Ok(None),
            Event::Resize(_, _) => Ok(None),
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind: _,
                state: _,
            }) => match code {
                KeyCode::Esc => Ok(None),
                KeyCode::Char(character) => {
                    if character == 'c' && modifiers.contains(KeyModifiers::CONTROL) {
                        Ok(Some(InputEvent::Shutdown))
                    } else if character == 'k' && modifiers.contains(KeyModifiers::CONTROL) {
                        self.show_command_popup = !self.show_command_popup;
                        logger.log_debug(&format!(
                            "Got command key, show_command_popup = {}",
                            self.show_command_popup
                        ));
                        Ok(None)
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
                        if self.show_command_popup {
                            self.show_command_popup = false;
                            match Command::from_str(&input) {
                                Ok(command) => Ok(Some(InputEvent::Command(command))),
                                Err(error) => {
                                    logger.log_error(&format!("Error parsing command: {}", error));
                                    Ok(None)
                                }
                            }
                        } else {
                            match self.chat_list.current_index() {
                                Some(_) => {
                                    let connection = self.chat_list.current().unwrap();
                                    match self.chats.get_mut(connection.id().as_str()) {
                                        Some(chat) => {
                                            let message = ChatMessage::new(&self.id, input.clone());
                                            chat.add_message(message);
                                            Ok(Some(InputEvent::Message {
                                                recipient: Box::new(connection.clone()),
                                                message: input,
                                            }))
                                        }
                                        None => {
                                            logger.log_error("No current chat");
                                            Ok(None)
                                        }
                                    }
                                }
                                None => {
                                    logger.log_error("No current chat");
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

    fn input_write(&mut self, character: char) {
        if self.show_command_popup {
            self.command_input.write(character);
        } else {
            self.chat_input.write(character);
        }
    }

    fn input_remove(&mut self) {
        if self.show_command_popup {
            self.command_input.remove();
        } else {
            self.chat_input.remove();
        }
    }

    fn input_remove_previous(&mut self) {
        if self.show_command_popup {
            self.command_input.remove_previous();
        } else {
            self.chat_input.remove_previous();
        }
    }

    fn input_move_cursor(&mut self, movement: CursorMovement) {
        if self.show_command_popup {
            self.command_input.move_cursor(movement);
        } else {
            self.chat_input.move_cursor(movement);
        }
    }

    fn get_input(&self) -> String {
        if self.show_command_popup {
            self.command_input.get_input()
        } else {
            self.chat_input.get_input()
        }
    }

    fn get_cursor_location(&self, inner_width: usize) -> (u16, u16) {
        if self.show_command_popup {
            self.command_input.cursor_location(inner_width)
        } else {
            self.chat_input.cursor_location(inner_width)
        }
    }

    fn messages_scroll(&mut self, movement: ScrollMovement) {
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

    fn clear_input_to_cursor(&mut self) {
        if self.show_command_popup {
            self.command_input.clear_input_to_cursor();
        } else {
            self.chat_input.clear_input_to_cursor();
        }
    }

    fn reset_input(&mut self) -> Option<String> {
        if self.show_command_popup {
            self.command_input.reset_input()
        } else {
            self.chat_input.reset_input()
        }
    }

    pub fn draw(
        &self,
        frame: &mut Frame<'_, CrosstermBackend<impl Write>>,
        chunk: Rect,
        logger: &mut dyn LoggerPlusIterator,
    ) {
        logger.log_debug("UI::draw called");
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
                self.draw_system_messages_panel(frame, chunks[1], logger);
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
                self.draw_system_messages_panel(frame, chunks[1], logger);
            }
        }
        if self.show_command_popup {
            self.draw_command_popup(frame, logger);
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
        logger: &mut dyn LoggerPlusIterator,
    ) {
        let messages = logger
            .iter()
            .map(|message| {
                let date = message.date.format("%H:%M:%S ").to_string();
                let color = match message.level {
                    Level::Debug => Color::Yellow,
                    Level::Info => Color::Green,
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

        let input = self.get_input();
        let input = split_each(input, inner_width)
            .into_iter()
            .map(|line| Line::from(vec![Span::raw(line)]))
            .collect::<Vec<_>>();

        let input_panel = Paragraph::new(input)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(self.input_panel_color))
            .alignment(Alignment::Left);

        frame.render_widget(input_panel, chunk);

        let input_cursor = self.get_cursor_location(inner_width);
        frame.set_cursor(chunk.x + input_cursor.0, chunk.y + input_cursor.1)
    }

    fn draw_centered_popup<'a, S, T, L: Logger + ?Sized>(
        &self,
        frame: &mut Frame<CrosstermBackend<impl Write>>,
        title: S,
        text: T,
        constraint_x: Constraint,
        constraint_y: Constraint,
        logger: &mut L,
    ) -> Rect
    where
        S: Into<Title<'a>>,
        T: Into<Text<'a>>,
    {
        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .wrap(Wrap { trim: true });
        let area = centered_rect(constraint_x, constraint_y, frame.size());
        logger.log_debug(&format!("help popup rect = {}", area));
        frame.render_widget(ratatui::widgets::Clear, area); //this clears out the background
        frame.render_widget(paragraph, area);

        area
    }

    fn draw_command_popup<L: Logger + ?Sized>(
        &self,
        frame: &mut Frame<'_, CrosstermBackend<impl Write>>,
        logger: &mut L,
    ) {
        let area = self.draw_centered_popup(
            frame,
            Line::styled("Command Input", Style::default().fg(Color::Blue)),
            "",
            Constraint::Percentage(70),
            Constraint::Length(3),
            logger,
        );

        let inner_width = (area.width - 2) as usize;

        let input = self.get_input();
        let input = split_each(input, inner_width)
            .into_iter()
            .map(|line| Line::from(vec![Span::raw(line)]))
            .collect::<Vec<_>>();

        let input_panel = Paragraph::new(input)
            .block(
                Block::default()
                    .title(Line::styled(
                        "Command Input",
                        Style::default().fg(Color::Blue),
                    ))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Left);

        frame.render_widget(ratatui::widgets::Clear, area); //this clears out the background
        frame.render_widget(input_panel, area);

        let input_cursor = self.get_cursor_location(inner_width);
        frame.set_cursor(area.x + input_cursor.0 + 1, area.y + input_cursor.1 + 1)
    }
}
