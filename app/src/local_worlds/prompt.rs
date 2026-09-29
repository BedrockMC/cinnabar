use super::model::Input;

pub(crate) const DOCKER_URL: &str = "https://www.docker.com/products/docker-desktop/";

/// The modal shown when vanilla worlds cannot run for want of Docker (macOS only; the core never
/// reports a reason on Windows or Linux).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptKind {
    DockerMissing,
    DockerNotRunning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptButton {
    PlayAnyway,
    GetDocker,
    DontShowAgain,
    Retry,
}

impl PromptKind {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::DockerMissing => {
                "Vanilla terrain and mobs on Mac need Docker (Docker Desktop, OrbStack or Colima). \
                 Without it, this world runs on a basic server with simpler terrain and no vanilla \
                 mob behavior."
            }
            Self::DockerNotRunning => "Start Docker to use vanilla worlds",
        }
    }

    pub(crate) fn buttons(self) -> &'static [PromptButton] {
        match self {
            Self::DockerMissing => &[
                PromptButton::PlayAnyway,
                PromptButton::GetDocker,
                PromptButton::DontShowAgain,
            ],
            Self::DockerNotRunning => &[PromptButton::Retry, PromptButton::PlayAnyway],
        }
    }
}

impl PromptButton {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PlayAnyway => "Play anyway",
            Self::GetDocker => "Get Docker",
            Self::DontShowAgain => "Don't show again",
            Self::Retry => "Retry",
        }
    }

    pub(crate) fn input(self) -> Input {
        Input::Prompt(self)
    }
}
