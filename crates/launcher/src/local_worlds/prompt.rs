use super::form::FLAT_WORLD_LABEL;
use super::model::Input;

pub const DOCKER_URL: &str = "https://www.docker.com/products/docker-desktop/";

/// Why default worlds cannot run (macOS only; the core never reports a reason on Windows or Linux).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptKind {
    DockerMissing,
    DockerNotRunning,
}

/// What the Docker modal is blocking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptFor {
    /// Opening the create screen: informational, can be dismissed for good.
    BeginCreate,
    /// Creating a default world, which needs the dedicated server.
    CreateDefault,
    /// Playing a saved world that runs on the dedicated server.
    Play,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptButton {
    /// Continue with a Flat world on the built-in server.
    CreateFlat,
    GetDocker,
    DontShowAgain,
    Retry,
    Cancel,
}

/// The Docker modal: its reason and what it blocks decide the text and the ways forward.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub blocking: PromptFor,
}

impl Prompt {
    pub fn title(self) -> &'static str {
        match self.kind {
            PromptKind::DockerMissing => "Docker is needed",
            PromptKind::DockerNotRunning => "Docker is not running",
        }
    }

    pub fn text(self) -> String {
        match (self.kind, self.blocking) {
            (PromptKind::DockerMissing, PromptFor::Play) => {
                "This world runs on the official Bedrock Dedicated Server, which needs Docker on Mac \
                 (Docker Desktop, OrbStack or Colima). Install Docker, start it, then play again."
                    .to_owned()
            }
            (PromptKind::DockerNotRunning, PromptFor::Play) => {
                "This world runs on the official Bedrock Dedicated Server in Docker. Start Docker, \
                 then choose Retry."
                    .to_owned()
            }
            (PromptKind::DockerMissing, _) => format!(
                "Default worlds run on the official Bedrock Dedicated Server, which needs Docker on \
                 Mac (Docker Desktop, OrbStack or Colima). Without Docker you can still create a \
                 {FLAT_WORLD_LABEL} world on the built-in server."
            ),
            (PromptKind::DockerNotRunning, _) => format!(
                "Default worlds run on the official Bedrock Dedicated Server in Docker. Start Docker, \
                 then choose Retry, or create a {FLAT_WORLD_LABEL} world on the built-in server."
            ),
        }
    }

    pub fn buttons(self) -> &'static [PromptButton] {
        use PromptButton::*;
        match (self.kind, self.blocking) {
            (PromptKind::DockerMissing, PromptFor::BeginCreate) => {
                &[CreateFlat, GetDocker, DontShowAgain]
            }
            (PromptKind::DockerMissing, PromptFor::CreateDefault) => {
                &[CreateFlat, GetDocker, Cancel]
            }
            (PromptKind::DockerMissing, PromptFor::Play) => &[GetDocker, Cancel],
            (PromptKind::DockerNotRunning, PromptFor::Play) => &[Retry, Cancel],
            (PromptKind::DockerNotRunning, _) => &[Retry, CreateFlat, Cancel],
        }
    }
}

impl PromptButton {
    pub fn label(self) -> String {
        match self {
            Self::CreateFlat => format!("Create {FLAT_WORLD_LABEL} world"),
            Self::GetDocker => "Get Docker".to_owned(),
            Self::DontShowAgain => "Don't show again".to_owned(),
            Self::Retry => "Retry".to_owned(),
            Self::Cancel => "Cancel".to_owned(),
        }
    }

    pub fn input(self) -> Input {
        Input::Prompt(self)
    }
}
