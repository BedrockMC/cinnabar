//! Account and Help Center routes use the platform browser and native dialogs.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportLink {
    Help,
    Attribution,
    LicensedContent,
    Gamertag,
    Account,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportDialog {
    Help,
    FontLicense,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportAction {
    Open(SupportLink),
    Dialog(SupportDialog),
}

impl SupportLink {
    /// Fixed destinations from general_section.json and AppPlatform::getFeedbackHelpLink.
    pub fn url(self) -> &'static str {
        match self {
            Self::Help => "https://aka.ms/MCHelp",
            Self::Attribution => "https://www.minecraft.net/attribution/?hideChrome",
            Self::LicensedContent => "https://www.minecraft.net/licensed-content/?hideChrome",
            Self::Gamertag => "https://social.xbox.com/changegamertag",
            Self::Account => "https://account.xbox.com/Settings",
        }
    }
}

/// The open fonts' actual shipped licenses, rather than vanilla's different font license.
pub fn font_licenses() -> String {
    [
        include_str!("../../../../assets/licenses/Monocraft-OFL-1.1.txt"),
        include_str!("../../../../assets/licenses/NotoSansCJK-OFL-1.1.txt"),
    ]
    .join("\n\n")
}
