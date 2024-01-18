use crate::{input::Input, root::split_each, theme::THEME};
use ratatui::{prelude::*, widgets::block::*, widgets::*};

pub struct ChatInput {
    input: String,
}

impl ChatInput {
    pub fn new(chat_input: &Input) -> Self {
        Self {
            input: chat_input.get_input(),
        }
    }
}

impl Widget for ChatInput {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner_width = (area.width - 2) as usize;

        let input = split_each(self.input, inner_width)
            .into_iter()
            .map(|line| Line::from(vec![Span::raw(line)]))
            .collect::<Vec<_>>();

        Paragraph::new(input)
            .block(Block::default().borders(Borders::NONE))
            .style(THEME.chat_input)
            .alignment(Alignment::Left)
            .render(area, buf);
    }
}
