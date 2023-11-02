use std::rc::Rc;

use clap::{crate_name, crate_version};
use itertools::Itertools;
use ratatui::{backend::CrosstermBackend, prelude::*, widgets::block::*, widgets::*};
use std::io::Write;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::AppContext,
    input::Input,
    logger::{Level, Logger, StandardLogger},
    theme::THEME,
};

pub struct Root<'a, W: Write> {
    context: &'a AppContext,
    logger: &'a mut StandardLogger,
    frame: &'a mut Frame<'a, CrosstermBackend<W>>,
}

impl<'a, W: Write> Root<'_, W> {
    pub fn new(
        context: &'_ AppContext,
        logger: &mut StandardLogger,
        frame: &mut Frame<'_, CrosstermBackend<W>>,
    ) -> Self {
        Root {
            context,
            logger,
            frame,
        }
    }
}

impl<'a, W: Write> Widget for Root<'_, W> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self.context.chat_list.current() {
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
                    .split(area);

                self.render_title_bar(chunks[0], buf);
                self.render_system_messages_panel(chunks[1], self.logger, buf);
                self.render_chat_tabs(chunks[2], buf);
                self.render_chat_panel(chunks[3], buf);
                self.render_status_bar(chunks[4], buf);
                self.render_chat_input(chunks[4], buf);
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
                    .split(area);

                self.render_title_bar(chunks[0], buf);
                self.render_system_messages_panel(chunks[1], self.logger, buf);
            }
        }
        if self.context.show_command_popup {
            self.render_command_popup(area, buf);
        }
    }
}

#[derive(Debug)]
struct CommandPopup<'a, W: Write> {
    input: &'a Input,
    frame: &'a mut Frame<'a, CrosstermBackend<W>>,
}

impl<'a, W: Write> CommandPopup<'a, W> {
    fn new(input: &Input, frame: &mut Frame<'_, CrosstermBackend<W>>) -> Self {
        Self { input, frame }
    }
}

impl<'a, W: Write> Widget for CommandPopup<'a, W> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = centered_rect(Constraint::Percentage(70), Constraint::Length(3), area);
        let inner_width = (area.width - 2) as usize;

        let input = self.input.get_input();
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
        Clear.render(area, buf); //this clears out the background
        input_panel.render(area, buf);

        let input_cursor = self.input.cursor_location(inner_width);
        self.frame
            .set_cursor(area.x + input_cursor.0 + 1, area.y + input_cursor.1 + 1)
    }
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

impl<W: Write> Root<'_, W> {
    fn render_title_bar(&self, area: Rect, buf: &mut Buffer) {
        let title_bar = Paragraph::new(format!("{} {}", crate_name!(), crate_version!(),))
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().bg(Color::Magenta))
            .alignment(Alignment::Left)
            .render(area, buf);
    }

    fn render_system_messages_panel(
        &self,
        area: Rect,
        logger: &mut StandardLogger,
        buf: &mut Buffer,
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
            .scroll((self.context.system_messages_scroll as u16, 0))
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_chat_tabs(&self, area: Rect, buf: &mut Buffer) {
        let tabs = Tabs::new(
            self.context
                .chat_list
                .names()
                .iter()
                .map(|s| Line::from(s.id().as_str().to_string()))
                .collect(),
        )
        .block(Block::default().title("Chats").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow))
        .select(self.context.chat_list.current_index().unwrap())
        .render(area, buf);
    }

    fn render_chat_panel(&self, area: Rect, buf: &mut Buffer) {
        if let Some(connection) = self.context.chat_list.current() {
            let chat = self.context.chats.get(connection.id().as_str()).unwrap();
            let messages = chat
                .iter()
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
                .scroll((self.context.system_messages_scroll as u16, 0))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
    }

    fn render_chat_input(&self, area: Rect, buf: &mut Buffer) {
        let inner_width = (area.width - 2) as usize;

        let input = self.context.chat_input.get_input();
        let input = split_each(input, inner_width)
            .into_iter()
            .map(|line| Line::from(vec![Span::raw(line)]))
            .collect::<Vec<_>>();

        let input_panel = Paragraph::new(input)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(input_panel_color))
            .alignment(Alignment::Left)
            .render(area, buf);

        let input_cursor = self.context.chat_input.cursor_location(inner_width);
        self.frame
            .set_cursor(area.x + input_cursor.0, area.y + input_cursor.1)
    }

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        let status_bar = Paragraph::new("Input")
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().bg(Color::Blue))
            .alignment(Alignment::Left)
            .render(area, buf);
    }

    fn render_command_popup(&self, area: Rect, buf: &mut Buffer) {
        let area = centered_rect(Constraint::Percentage(70), Constraint::Length(3), area);
        let inner_width = (area.width - 2) as usize;

        let input = self.context.command_input.get_input();
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
        Clear.render(area, buf); //this clears out the background
        input_panel.render(area, buf);

        let input_cursor = self.context.command_input.cursor_location(inner_width);
        self.frame
            .set_cursor(area.x + input_cursor.0 + 1, area.y + input_cursor.1 + 1)
    }

    fn parse_content(content: &str) -> Vec<Span> {
        vec![Span::raw(content)]
    }
}

/// simple helper method to split an area into multiple sub-areas
pub fn layout(area: Rect, direction: Direction, heights: Vec<u16>) -> Rc<[Rect]> {
    let constraints = heights
        .iter()
        .map(|&h| {
            if h > 0 {
                Constraint::Length(h)
            } else {
                Constraint::Min(0)
            }
        })
        .collect_vec();
    Layout::default()
        .direction(direction)
        .constraints(constraints)
        .split(area)
}
