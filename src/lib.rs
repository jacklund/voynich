//! # Voynich
//!
//! A library for creating anonymous, encrypted, authenticated chat applications.
//!
//! ## Example code
//!
//! ```no_run
//! use anyhow::Result;
//! use voynich::{
//!     connect_to_tor,
//!     create_onion_service,
//!     Engine,
//!     onion_service::OnionType,
//!     logger::StandardLogger
//! };
//! use std::str::FromStr;
//! use std::net::SocketAddr;
//! use tor_client_lib::control_connection::{OnionServiceListener, TorSocketAddr};
//!
//! // My UI Application
//! struct App {}
//!
//! impl App {
//!    // Run the UI app
//!    pub async fn run(
//!        engine: &mut Engine,
//!        listener: &OnionServiceListener,
//!        logger: &mut StandardLogger,
//!    ) -> Result<()> {
//!      // Run your app
//!      Ok(())
//!    }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Get a connection to Tor
//!     let mut control_connection = connect_to_tor(
//!         SocketAddr::from_str("127.0.0.1:9051").unwrap(),
//!         None,
//!         None,
//!         None,
//!     )
//!     .await?;
//!
//!     // Create our onion service
//!     let (mut onion_service, onion_service_address, mut listener) = create_onion_service(
//!         &mut control_connection,
//!         OnionType::Transient,
//!         Some(3000),
//!         Some(TorSocketAddr::from_str("127.0.0.1:3000").unwrap()),
//!     )
//!     .await?;
//!
//!     // Set up the engine
//!     let mut engine = Engine::new(
//!         &mut onion_service,
//!         onion_service_address,
//!         SocketAddr::from_str("127.0.0.1:9050").unwrap(),
//!         false,
//!     )
//!     .await?;
//!
//!     // Logging
//!     let mut logger = StandardLogger::new(500);
//!
//!     // Pass the engine, listener and logger to your application
//!     App::run(&mut engine, &listener, &mut logger).await
//! }
//! ```

/// Chat message structs
pub mod chat;

/// Configuration files
pub mod config;

/// Connection to peer
pub mod connection;

/// Connection to Tor server
pub mod control_connection;

/// Cryptographic functions
mod crypto;

/// Engine
pub mod engine;

/// Logging
pub mod logger;

/// Onion service struct
pub mod onion_service;

/// Utility functions
pub mod util;

pub use config::get_config;
pub use control_connection::{connect_to_tor, create_onion_service};
pub use engine::Engine;
pub use util::test_onion_service_connection;
