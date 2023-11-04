use crate::{
    chat::{Chat, ChatList, ChatMessage},
    commands::Command,
    engine::{Connection, InputEvent},
    input::{CursorMovement, Input, ScrollMovement},
    logger::{Level, Logger, StandardLogger},
};
use async_trait::async_trait;
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
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use ratatui::Terminal;

use std::collections::HashMap;
use std::io::Write;

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
        logger: &mut StandardLogger,
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
        logger: &mut StandardLogger,
    ) -> Result<(), std::io::Error>;

    async fn get_input_event(
        &mut self,
        logger: &mut dyn Logger,
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
    system_messages_scroll: usize,
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
        logger: &mut StandardLogger,
    ) -> Result<(), std::io::Error> {
        renderer.render(self, logger)
    }

    async fn get_input_event(
        &mut self,
        logger: &mut dyn Logger,
    ) -> Result<Option<InputEvent>, anyhow::Error> {
        let event = self.input_stream.select_next_some().await?;
        match self.handle_input_event(event, logger).await? {
            Some(input_event) => Ok(Some(input_event)),
            None => Ok(None),
        }
    }

    fn add_chat(&mut self, connection: &Connection) {
        self.chat_list.add(&connection.id());
        self.chats
            .insert(connection.id().into(), Chat::new(&connection.id()));
    }

    fn add_message(&mut self, message: ChatMessage) {
        if let Some(chat) = self.chats.get_mut(&message.sender) {
            chat.add_message(message);
        }
    }

    fn remove_chat(&mut self, connection: &Connection) {
        self.chat_list.remove(&connection.id());
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
            system_messages_scroll: 0,
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

    async fn handle_input_event(
        &mut self,
        event: Event,
        logger: &mut dyn Logger,
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
                                    let id = self.chat_list.current().unwrap();
                                    match self.chats.get_mut(id.as_str()) {
                                        Some(chat) => {
                                            let message = ChatMessage::new(
                                                id.as_str().to_string(),
                                                self.id.clone(),
                                                input.clone(),
                                            );
                                            chat.add_message(message);
                                            Ok(Some(InputEvent::Message {
                                                recipient: id.as_str().to_string(),
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
                if self.system_messages_scroll > 0 {
                    self.system_messages_scroll -= 1;
                }
            }
            ScrollMovement::Down => {
                self.system_messages_scroll += 1;
            }
            ScrollMovement::Start => {
                self.system_messages_scroll += 0;
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
        logger: &mut StandardLogger,
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
        logger: &mut StandardLogger,
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
            .scroll((self.system_messages_scroll as u16, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(messages_panel, chunk);
    }

    fn draw_chat_tabs(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        let tabs = Tabs::new(
            self.chat_list
                .names()
                .iter()
                .map(|s| Line::from(s.as_str().to_string()))
                .collect(),
        )
        .block(Block::default().title("Chats").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow))
        .select(self.chat_list.current_index().unwrap());

        frame.render_widget(tabs, chunk);
    }

    fn draw_chat_panel(&self, frame: &mut Frame<CrosstermBackend<impl Write>>, chunk: Rect) {
        if let Some(id) = self.chat_list.current() {
            let chat = self.chats.get(id.as_str()).unwrap();
            let messages = chat
                .iter()
                .map(|message| {
                    let date = message.date.format("%H:%M:%S ").to_string();
                    let mut ui_message = vec![
                        Span::styled(date, Style::default().fg(self.date_color)),
                        Span::styled(message.sender.clone(), Style::default().fg(Color::Blue)),
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
                .scroll((self.system_messages_scroll as u16, 0))
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

    fn draw_command_popup<L: Logger + ?Sized>(
        &self,
        frame: &mut Frame<'_, CrosstermBackend<impl Write>>,
        logger: &mut L,
    ) {
        let area = centered_rect(
            Constraint::Percentage(70),
            Constraint::Length(3),
            frame.size(),
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
