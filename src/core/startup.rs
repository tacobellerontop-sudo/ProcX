//! Startup entry models.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartupSource {
    /// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
    RunUser,
    /// `HKCU\...\RunOnce`
    RunOnceUser,
    /// `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`
    RunMachine,
    /// `HKLM\...\RunOnce`
    RunOnceMachine,
    /// `HKLM\SOFTWARE\WOW6432Node\...\Run` (32-bit programs)
    RunMachine32,
    /// The current user's Startup folder.
    FolderUser,
    /// The all-users Startup folder.
    FolderCommon,
}

impl StartupSource {
    pub const ALL: [StartupSource; 7] = [
        Self::RunUser,
        Self::RunOnceUser,
        Self::RunMachine,
        Self::RunOnceMachine,
        Self::RunMachine32,
        Self::FolderUser,
        Self::FolderCommon,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::RunUser => "Registry: Run (current user)",
            Self::RunOnceUser => "Registry: RunOnce (current user)",
            Self::RunMachine => "Registry: Run (all users)",
            Self::RunOnceMachine => "Registry: RunOnce (all users)",
            Self::RunMachine32 => "Registry: Run (all users, 32-bit)",
            Self::FolderUser => "Startup folder (current user)",
            Self::FolderCommon => "Startup folder (all users)",
        }
    }

    /// Whether changing this entry affects all users (and needs administrator rights).
    pub fn is_machine_wide(self) -> bool {
        matches!(
            self,
            Self::RunMachine | Self::RunOnceMachine | Self::RunMachine32 | Self::FolderCommon
        )
    }

    /// RunOnce entries have no enable/disable mechanism.
    pub fn supports_toggle(self) -> bool {
        !matches!(self, Self::RunOnceUser | Self::RunOnceMachine)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupStatus {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartupEntry {
    /// Registry value name, or file name for Startup-folder items.
    pub name: String,
    pub source: StartupSource,
    /// Registry key or folder path, for display.
    pub location: String,
    /// Registry command line; `None` for folder items.
    pub command: Option<String>,
    /// Resolved executable or file path.
    pub path: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub status: StartupStatus,
}

/// Extracts the program path from a Run-key command line:
/// a quoted first token, otherwise everything up to and including the first
/// `.exe`, otherwise the first whitespace-separated token.
pub fn executable_from_command(command: &str) -> Option<String> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return None;
    }
    if let Some(rest) = cmd.strip_prefix('"') {
        return rest
            .split('"')
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    let lower = cmd.to_ascii_lowercase();
    if let Some(i) = lower.find(".exe") {
        return Some(cmd[..i + 4].to_owned());
    }
    cmd.split_whitespace().next().map(str::to_owned)
}

/// Interprets a `StartupApproved` value: an odd first byte means disabled.
/// A missing or malformed value means the entry is enabled (Windows' default).
pub fn status_from_approved(data: Option<&[u8]>) -> StartupStatus {
    match data.and_then(|d| d.first()) {
        Some(b) if b & 1 == 1 => StartupStatus::Disabled,
        _ => StartupStatus::Enabled,
    }
}

/// Builds the 12-byte `StartupApproved` value Windows writes: a flag DWORD
/// (2 = enabled, 3 = disabled) followed by the FILETIME of the change
/// (zero when enabling).
pub fn approved_value(status: StartupStatus, filetime_now: u64) -> [u8; 12] {
    let mut v = [0u8; 12];
    match status {
        StartupStatus::Enabled => v[0] = 0x02,
        StartupStatus::Disabled => {
            v[0] = 0x03;
            v[4..].copy_from_slice(&filetime_now.to_le_bytes());
        }
    }
    v
}

/// Case-insensitive match on name, publisher, command or path. `query` must be lowercase.
pub fn matches(entry: &StartupEntry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let contains = |s: &str| s.to_lowercase().contains(query);
    contains(&entry.name)
        || entry.publisher.as_deref().is_some_and(contains)
        || entry.command.as_deref().is_some_and(contains)
        || entry.path.as_deref().is_some_and(contains)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(
            executable_from_command(r#""C:\Program Files\App\app.exe" --min"#).as_deref(),
            Some(r"C:\Program Files\App\app.exe")
        );
        assert_eq!(
            executable_from_command(r"C:\Program Files\App\app.EXE /background").as_deref(),
            Some(r"C:\Program Files\App\app.EXE")
        );
        assert_eq!(
            executable_from_command("rundll32 foo.dll,Entry").as_deref(),
            Some("rundll32")
        );
        assert_eq!(executable_from_command("   "), None);
        assert_eq!(executable_from_command(r#""""#), None);
    }

    #[test]
    fn approved_round_trip() {
        assert_eq!(status_from_approved(None), StartupStatus::Enabled);
        assert_eq!(
            status_from_approved(Some(&[0x02, 0, 0, 0])),
            StartupStatus::Enabled
        );
        assert_eq!(status_from_approved(Some(&[0x06])), StartupStatus::Enabled);
        assert_eq!(
            status_from_approved(Some(&[0x03, 0, 0, 0])),
            StartupStatus::Disabled
        );
        assert_eq!(status_from_approved(Some(&[0x07])), StartupStatus::Disabled);
        let v = approved_value(StartupStatus::Disabled, 0x0102_0304_0506_0708);
        assert_eq!(status_from_approved(Some(&v)), StartupStatus::Disabled);
        assert_eq!(&v[4..], &0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(
            status_from_approved(Some(&approved_value(StartupStatus::Enabled, 5))),
            StartupStatus::Enabled
        );
    }
}
