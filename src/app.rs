use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::{
    stream::{FusedStream, Stream},
    task::Poll,
    StreamExt,
};
use futures_lite::StreamExt as LiteStreamExt;
use std::pin::Pin;
use std::str::FromStr;
use std::task::Context as TaskContext;
use tokio::select;
use tor_client_lib::TorServiceId;

use crate::{
    app_context::AppContext,
    chat::{Chat, ChatMessage},
    commands::Command,
    engine::{Engine, NetworkEvent},
    input::{CursorMovement, ScrollMovement},
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

impl App {
    fn new(id: TorServiceId, onion_service_address: String) -> Result<Self> {
        Ok(Self {
            term: Term::start()?,
            input_stream: TermInputStream::new(),
            should_quit: false,
            context: AppContext::new(id, onion_service_address),
        })
    }

    pub async fn run(engine: &mut Engine, logger: &mut StandardLogger) -> Result<()> {
        install_panic_hook();
        let mut app = Self::new(engine.id(), engine.onion_service_address())?;

        logger.log_info(&format!(
            "Onion service {} created",
            engine.onion_service_address(),
        ));

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
                let root = Root::new(&self.context, logger);
                if let Some((x, y)) = root.get_cursor_location(frame.size()) {
                    frame.set_cursor(x, y);
                }
                frame.render_widget(root, frame.size());
            })
            .context("terminal.draw")?;
        Ok(())
    }

    async fn handle_events(
        &mut self,
        engine: &mut Engine,
        logger: &mut StandardLogger,
    ) -> Result<()> {
        select! {
            result = self.input_stream.select_next_some() => {
                match result {
                    Ok(event) => {
                        self.handle_input_event(event, engine, logger).await;
                        Ok(())
                    },
                    Err(error) => {
                        logger.log_error(&format!("Error reading input: {}", error));
                        Ok(())
                    },
                }
            }
            result = engine.get_network_event(logger) => {
                match result {
                    Ok(Some(NetworkEvent::NewConnection(connection))) => {
                        self.context.chat_list.add(&connection.id());
                        self.context.chats
                            .insert(connection.id(), Chat::new(&connection.id()));
                        Ok(())
                    }
                    Ok(Some(NetworkEvent::Message { sender, message })) => {
                        match TorServiceId::from_str(&sender) {
                            Ok(id) => {
                                if let Some(chat) = self.context.chats.get_mut(&id) {
                                    chat.add_message(ChatMessage::new(sender, self.context.id.clone().into(), message));
                                }
                                Ok(())
                            },
                            Err(error) => Err(anyhow::anyhow!("Error parsing Tor Service ID {}: {}", sender, error)),
                        }
                    },
                    Ok(Some(NetworkEvent::ConnectionClosed(connection))) => {
                        self.context.chat_list.remove(&connection.id());
                        self.context.chats.remove(&connection.id());
                        Ok(())
                    }
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                }
            }
        }
    }

    async fn handle_input_event(
        &mut self,
        event: Event,
        engine: &mut Engine,
        logger: &mut StandardLogger,
    ) {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: _,
            state: _,
        }) = event
        {
            match code {
                KeyCode::Char(character) => {
                    if character == 'c' && modifiers.contains(KeyModifiers::CONTROL) {
                        self.should_quit = true;
                    } else if character == 'k' && modifiers.contains(KeyModifiers::CONTROL) {
                        self.context.show_command_popup = !self.context.show_command_popup;
                        if self.context.show_command_popup {
                            self.context.show_welcome_popup = false;
                        }
                    } else if character == 'u' && modifiers.contains(KeyModifiers::CONTROL) {
                        self.context.current_input().clear_input_to_cursor();
                    } else if character == 'h' && modifiers.contains(KeyModifiers::CONTROL) {
                        self.context.show_welcome_popup = !self.context.show_welcome_popup;
                        if self.context.show_welcome_popup {
                            self.context.show_command_popup = false;
                        }
                    } else {
                        self.context.current_input().write(character);
                    }
                }
                KeyCode::Esc => {
                    if self.context.show_welcome_popup {
                        self.context.show_welcome_popup = false;
                    }
                    if self.context.show_command_popup {
                        self.context.show_command_popup = false;
                    }
                }
                KeyCode::Enter => {
                    if let Some(input) = self.context.current_input().reset_input() {
                        if self.context.show_command_popup {
                            self.context.toggle_command_popup();
                            match Command::from_str(&input) {
                                Ok(Command::Quit) => {
                                    self.should_quit = true;
                                }
                                Ok(command) => {
                                    self.handle_command(logger, command, engine).await;
                                }
                                Err(error) => {
                                    logger.log_error(&format!("Error parsing command: {}", error));
                                }
                            }
                        } else {
                            match self.context.chat_list.current_index() {
                                Some(_) => {
                                    let id = self.context.chat_list.current().unwrap();
                                    match self.context.chats.get_mut(id) {
                                        Some(chat) => {
                                            let message = ChatMessage::new(
                                                self.context.id.clone().into(),
                                                id.as_str().to_string(),
                                                input.clone(),
                                            );
                                            chat.add_message(message.clone());
                                            if let Err(error) =
                                                engine.send_message(message, logger).await
                                            {
                                                logger.log_error(&format!(
                                                    "Error sending chat message: {}",
                                                    error
                                                ));
                                            }
                                        }
                                        None => {
                                            logger.log_error("No current chat");
                                        }
                                    }
                                }
                                None => {
                                    logger.log_error("No current chat");
                                }
                            }
                        }
                    }
                }
                KeyCode::Delete => {
                    self.context.current_input().remove();
                }
                KeyCode::Backspace => {
                    self.context.current_input().remove_previous();
                }
                KeyCode::Left => {
                    if modifiers == KeyModifiers::CONTROL {
                        self.context.chat_list.prev();
                    } else {
                        self.context
                            .current_input()
                            .move_cursor(CursorMovement::Left);
                    }
                }
                KeyCode::Right => {
                    if modifiers == KeyModifiers::CONTROL {
                        self.context.chat_list.next();
                    } else {
                        self.context
                            .current_input()
                            .move_cursor(CursorMovement::Right);
                    }
                }
                KeyCode::Home => {
                    self.context
                        .current_input()
                        .move_cursor(CursorMovement::Start);
                }
                KeyCode::End => {
                    self.context
                        .current_input()
                        .move_cursor(CursorMovement::End);
                }
                KeyCode::Up => {
                    self.messages_scroll(ScrollMovement::Up);
                }
                KeyCode::Down => {
                    self.messages_scroll(ScrollMovement::Down);
                }
                KeyCode::PageUp => {
                    self.messages_scroll(ScrollMovement::Start);
                }
                _ => {}
            }
        }
    }

    pub async fn handle_command(
        &mut self,
        logger: &mut StandardLogger,
        command: Command,
        engine: &mut Engine,
    ) {
        match command {
            Command::Connect { address } => {
                if let Err(error) = engine.connect(&address, logger).await {
                    logger.log_error(&format!("Connect error: {}", error));
                }
            }
            _ => {}
        }
    }

    fn messages_scroll(&mut self, movement: ScrollMovement) {
        match movement {
            ScrollMovement::Up => {
                if self.context.system_messages_scroll > 0 {
                    self.context.system_messages_scroll -= 1;
                }
            }
            ScrollMovement::Down => {
                self.context.system_messages_scroll += 1;
            }
            ScrollMovement::Start => {
                self.context.system_messages_scroll += 0;
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