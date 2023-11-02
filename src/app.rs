use std::collections::HashMap;

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures::{
    stream::{FusedStream, Stream},
    task::Poll,
    StreamExt,
};
use futures_lite::StreamExt as LiteStreamExt;
use std::pin::Pin;
use std::task::Context as TaskContext;
use tokio::select;

use crate::{
    chat::{Chat, ChatList},
    engine::Engine,
    input::Input,
    logger::{Logger, StandardLogger},
    root::Root,
    term::Term,
};

#[derive(Debug)]
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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        self.reader.poll_next(cx)
    }
}

impl FusedStream for TermInputStream {
    fn is_terminated(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct App {
    term: Term,
    input_stream: TermInputStream,
    should_quit: bool,
    context: AppContext,
}

#[derive(Debug, Default, Clone)]
pub struct AppContext {
    pub chat_list: ChatList,
    pub chats: HashMap<String, Chat>,
    pub show_command_popup: bool,
    pub system_messages_scroll: usize,
    pub chat_input: Input,
    pub command_input: Input,
}

impl AppContext {
    fn new() -> Self {
        Self {
            chat_list: ChatList::default(),
            chats: HashMap::default(),
            show_command_popup: false,
            system_messages_scroll: 0,
            chat_input: Input::new(None),
            command_input: Input::new(Some(":>")),
        }
    }
}

impl App {
    fn new() -> Result<Self> {
        Ok(Self {
            term: Term::start()?,
            input_stream: TermInputStream::new(),
            should_quit: false,
            context: AppContext::default(),
        })
    }

    pub async fn run(engine: &mut Engine, logger: &mut StandardLogger) -> Result<()> {
        install_panic_hook();
        let mut app = Self::new()?;
        while !app.should_quit {
            app.draw(logger)?;
            app.handle_events(engine, logger).await?;
        }
        Term::stop()?;
        Ok(())
    }

    fn draw(&mut self, logger: &mut StandardLogger) -> Result<()> {
        self.term
            .draw(|frame| {
                frame.render_widget(Root::new(&self.context, logger, frame), frame.size())
            })
            .context("terminal.draw")?;
        Ok(())
    }

    async fn handle_events<L: Logger + ?Sized>(
        &mut self,
        engine: &mut Engine,
        logger: &mut L,
    ) -> Result<()> {
        select! {
            result = self.input_stream.select_next_some() => {
                match result {
                    Ok(_event) => Ok(()),
                    Err(error) => Err(error.into()),
                }
            }
            result = engine.get_network_event(logger) => {
                match result {
                    Ok(Some(_event)) => Ok(()),
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                }
            }
        }
    }
}

pub fn install_panic_hook() {
    better_panic::install();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = Term::stop();
        hook(info);
        std::process::exit(1);
    }));
}
