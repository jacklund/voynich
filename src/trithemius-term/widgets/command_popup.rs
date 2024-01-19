use crate::{
    input::command_input::CommandInput,
    root::{centered_rect, split_each},
    theme::THEME,
};
use ratatui::{prelude::*, widgets::block::*, widgets::*};

pub struct CommandPopup {
    input: String,
}

impl CommandPopup {
    pub fn new(command_input: &CommandInput) -> Self {
        Self {
            input: command_input.get_input(),
        }
    }
}

impl Widget for CommandPopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = centered_rect(Constraint::Percentage(70), Constraint::Length(3), area);
        let inner_width = (area.width - 2) as usize;

        let input = split_each(self.input, inner_width)
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
}
