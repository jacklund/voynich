use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::{
    stream::{FusedStream, Stream},
    task::Poll,
    StreamExt,
};
use futures_lite::StreamExt as LiteStreamExt;
use std::pin::Pin;
use std::task::Context as TaskContext;
use tokio::select;

use crate::{engine::Engine, logger::Logger, root::Root, term::Term};

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

#[derive(Debug, Default, Clone, Copy)]
pub struct AppContext {
    pub tab_index: usize,
    pub row_index: usize,
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

    pub async fn run<L: Logger + ?Sized>(engine: &mut Engine, logger: &mut L) -> Result<()> {
        install_panic_hook();
        let mut app = Self::new()?;
        while !app.should_quit {
            app.draw(logger)?;
            app.handle_events(engine, logger).await?;
        }
        Term::stop()?;
        Ok(())
    }

    fn draw<L: Logger + ?Sized>(&mut self, logger: &mut L) -> Result<()> {
        self.term
            .draw(|frame| frame.render_widget(Root::new(&self.context, logger), frame.size()))
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
