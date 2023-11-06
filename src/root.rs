use std::rc::Rc;

use clap::{crate_name, crate_version};
use ratatui::{prelude::*, widgets::block::*, widgets::*};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app_context::AppContext,
    logger::StandardLogger,
    theme::{Theme, THEME},
};

pub struct Root<'a> {
    context: &'a AppContext,
    logger: &'a mut StandardLogger,
}

impl<'a> Root<'a> {
    pub fn new(context: &'a AppContext, logger: &'a mut StandardLogger) -> Self {
        Root { context, logger }
    }
}

impl Widget for Root<'_> {
    fn render(mut self, area: Rect, buf: &mut Buffer) {
        match self.context.chat_list.current() {
            Some(_) => {
                let chunks = self.get_layout(area);

                self.render_title_bar(chunks[0], buf);
                self.render_system_messages_panel(chunks[1], buf);
                self.render_chat_tabs(chunks[2], buf);
                self.render_chat_panel(chunks[3], buf);
                self.render_status_bar(chunks[4], buf);
                self.render_chat_input(chunks[5], buf);
            }
            None => {
                let chunks = self.get_layout(area);

                self.render_title_bar(chunks[0], buf);
                self.render_system_messages_panel(chunks[1], buf);
            }
        }
        if self.context.show_command_popup {
            self.render_command_popup(area, buf);
        }
        if self.context.show_welcome_popup {
            self.render_welcome_popup(area, buf);
        }
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

impl Root<'_> {
    pub fn get_cursor_location(&self, area: Rect) -> Option<(u16, u16)> {
        if self.context.show_command_popup {
            let area = centered_rect(Constraint::Percentage(70), Constraint::Length(3), area);
            let inner_width = (area.width - 2) as usize;
            let input_cursor = self.context.command_input.cursor_location(inner_width);
            Some((area.x + input_cursor.0 + 1, area.y + input_cursor.1 + 1))
        } else {
            let chunks = self.get_layout(area);
            if chunks.len() < 6 {
                return None;
            }
            let inner_width = (area.width - 2) as usize;
            let input_cursor = self.context.chat_input.cursor_location(inner_width);
            Some((area.x + input_cursor.0, chunks[5].y + input_cursor.1 + 1))
        }
    }

    fn get_layout(&self, area: Rect) -> Rc<[Rect]> {
        match self.context.chat_list.current() {
            Some(_) => Layout::default()
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
                .split(area),
            None => Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)].as_ref())
                .split(area),
        }
    }

    fn render_title_bar(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "{} {}  Onion address: {}",
                crate_name!(),
                crate_version!(),
                self.context.onion_service_address
            ),
            Style::new().add_modifier(Modifier::BOLD),
        )]))
        .block(Block::default().borders(Borders::NONE))
        .style(THEME.title_bar)
        .alignment(Alignment::Left)
        .render(area, buf);
    }

    fn render_system_messages_panel(&mut self, area: Rect, buf: &mut Buffer) {
        let messages = self
            .logger
            .iter()
            .map(|message| {
                let date = message.date.format("%H:%M:%S ").to_string();
                let system_message_style = Theme::get_system_message_style(message);
                let ui_message = vec![
                    Span::styled(date, system_message_style.date),
                    Span::styled(message.message.clone(), system_message_style.message),
                ];
                Line::from(ui_message)
            })
            .collect::<Vec<_>>();

        let inner_height = area.height - 2;
        let scroll = if messages.len() as u16 > inner_height {
            messages.len() as u16 - inner_height
        } else {
            0
        };
        Paragraph::new(messages)
            .block(Block::default().borders(Borders::ALL).title(Span::styled(
                "System Messages",
                Style::default().add_modifier(Modifier::BOLD),
            )))
            .style(THEME.system_messages_panel)
            .alignment(Alignment::Left)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_chat_tabs(&self, area: Rect, buf: &mut Buffer) {
        Tabs::new(
            self.context
                .chat_list
                .names()
                .iter()
                .map(|s| Line::from(s.as_str().to_string()))
                .collect(),
        )
        .block(Block::default().title("Chats").borders(Borders::ALL))
        .style(THEME.chat_tabs.style)
        .highlight_style(THEME.chat_tabs.highlight_style)
        .select(self.context.chat_list.current_index().unwrap())
        .render(area, buf);
    }

    fn render_chat_panel(&self, area: Rect, buf: &mut Buffer) {
        if let Some(id) = self.context.chat_list.current() {
            let chat = self.context.chats.get(id).unwrap();
            let messages = chat
                .iter()
                .map(|message| {
                    let date = message.date.format("%H:%M:%S ").to_string();
                    let ui_message = vec![
                        Span::styled(date, THEME.chat_message.date),
                        Span::styled(message.sender.as_str(), THEME.chat_message.message_id),
                        Span::styled(": ", THEME.chat_message.separator),
                        Span::raw(message.message.clone()),
                    ];
                    Line::from(ui_message)
                })
                .collect::<Vec<_>>();

            let inner_height = area.height - 2;
            let scroll = if messages.len() as u16 > inner_height {
                messages.len() as u16 - inner_height
            } else {
                0
            };
            Paragraph::new(messages)
                .block(Block::default().borders(Borders::ALL).title(Span::styled(
                    chat.id().clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )))
                .style(THEME.chat_panel)
                .alignment(Alignment::Left)
                .scroll((scroll, 0))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
    }

    fn render_chat_input(&mut self, area: Rect, buf: &mut Buffer) {
        let inner_width = (area.width - 2) as usize;

        let input = self.context.chat_input.get_input();
        let input = split_each(input, inner_width)
            .into_iter()
            .map(|line| Line::from(vec![Span::raw(line)]))
            .collect::<Vec<_>>();

        Paragraph::new(input)
            .block(Block::default().borders(Borders::NONE))
            .style(THEME.chat_input)
            .alignment(Alignment::Left)
            .render(area, buf);
    }

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new("Input")
            .block(Block::default().borders(Borders::NONE))
            .style(THEME.status_bar)
            .alignment(Alignment::Left)
            .render(area, buf);
    }

    fn render_command_popup(&mut self, area: Rect, buf: &mut Buffer) {
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
                    .title(Line::styled("Command Input", THEME.input_panel.title))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(THEME.input_panel.border),
            )
            .style(THEME.input_panel.style)
            .alignment(Alignment::Left);
        Clear.render(area, buf); //this clears out the background
        input_panel.render(area, buf);
    }

    fn render_welcome_popup(&mut self, area: Rect, buf: &mut Buffer) {
        let title = format!("Welcome to {} version {}", crate_name!(), crate_version!());
        let address = format!(
            "Your onion service address is: {}",
            self.context.onion_service_address
        );
        let greeting_text = vec![
            Line::styled(title, Style::default().add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center),
            Line::raw(""),
            Line::raw(address),
            Line::raw(""),
            Line::styled(
                "Help",
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Line::raw(""),
            Line::raw("To connect to someone, press ctrl-k to bring up a command window, and type 'connect <onion-address>'"),
            Line::raw("Once connected, type your messages in the input box at the bottom"),
            Line::raw("Type ctrl-c anywhere, or 'quit' in the command window, to exit"),
            Line::raw("Type ctrl-h to show/hide this window again"),
            Line::raw("Type ctrl-k to show/hide the command window"),
            Line::raw(""),
            Line::styled(
                "Commands",
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Line::raw(""),
            Line::raw("connect <address>  - to connect to another chat user"),
            Line::raw("quit               - to exit the application"),
            Line::raw(""),
        ];

        let area = centered_rect(
            Constraint::Percentage(60),
            Constraint::Length(greeting_text.len() as u16 + 2),
            area,
        );
        let greeting = Paragraph::new(greeting_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(THEME.input_panel.border),
        );
        Clear.render(area, buf); //this clears out the background
        greeting.render(area, buf);
    }
}
