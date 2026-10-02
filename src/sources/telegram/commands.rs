use teloxide::utils::command::BotCommands;

/// Chat commands. Adding one = a variant here and an arm in [`Command::answer`].
#[derive(BotCommands, Debug, Clone, Copy, PartialEq, Eq)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "say hello")]
    Start,
    #[command(description = "list the commands")]
    Help,
}

impl Command {
    pub fn answer(self) -> String {
        match self {
            Self::Start => "Hello! I'm Devy !".to_owned(),
            Self::Help => Self::descriptions().to_string(),
        }
    }
}

/// Used when someone talks to the bot with a command it doesn't know.
pub const UNKNOWN_COMMAND_REPLY: &str = "💀";
