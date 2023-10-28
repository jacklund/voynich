use crate::logger::Logger;
use std::str::FromStr;

#[derive(Debug)]
pub enum Command {
    Connect { address: String },
    Help { command: Option<String> },
    Quit,
}

impl Command {
    pub fn get_help<'a, L: Logger + ?Sized>(command: Option<String>, logger: &mut L) {
        match command {
            Some(command) => match command.as_str() {
                "connect" => {
                    logger.log_info("Connect to a remote client");
                    logger.log_info("Usage: connect <ONION_ADDRESS>:<PORT>");
                    logger.log_info(
                        "   where <ONION_ADDRESS> is the onion address of the chat client",
                    );
                    logger.log_info("   and <PORT> is the numeric port number");
                    logger.log_info("Example: connect x7h5ctx2himz7th44wh3mlsyyx2qvw42gwgzbi7zzqeskvj45v47qwyd.onion:3000");
                }
                "help" => {
                    logger.log_info("Get help for a command");
                    logger.log_info("Usage: help [<COMMAND>]");
                    logger.log_info("   where <COMMAND> is an optional command to get help for");
                }
                "quit" => {
                    logger.log_info("Quit the chat program");
                    logger.log_info("Usage: quit");
                }
                _ => {
                    logger.log_error(&format!("Unknown command '{}'", command));
                }
            },
            None => {
                logger.log_info("Commands:");
                logger.log_info("   connect - Connect to a remote client");
                logger.log_info("   help - Get help on a command");
                logger.log_info("   quit - Quit this program");
            }
        }
    }
}

impl FromStr for Command {
    type Err = anyhow::Error;

    fn from_str(string: &str) -> Result<Self, Self::Err> {
        let tokens = string.split_whitespace().collect::<Vec<&str>>();
        if !tokens.is_empty() {
            match tokens[0] {
                "connect" => {
                    if tokens.len() != 2 {
                        Err(anyhow::anyhow!("'connect' command only takes one argument"))
                    } else {
                        Ok(Self::Connect {
                            address: tokens[1].to_string(),
                        })
                    }
                }
                "help" => {
                    let command = match tokens.len() {
                        1 => None,
                        2 => Some(tokens[1]),
                        _ => {
                            return Err(anyhow::anyhow!(
                                "'help' only takes a single command as argument.
                                    For more information, type 'help help' in the command prompt"
                            ));
                        }
                    };
                    Ok(Self::Help {
                        command: command.map(|s| s.to_string()),
                    })
                }
                "quit" => Ok(Self::Quit),
                _ => Err(anyhow::anyhow!("Unknown command '{}'", tokens[0])),
            }
        } else {
            Err(anyhow::anyhow!("Empty command"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_help_string() {
        let help_string = "help connect";
        let parse_result = Command::from_str(help_string);
        assert!(parse_result.is_ok());
    }
}
