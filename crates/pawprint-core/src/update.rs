//! Checking for and installing a new release, as the badge sees it (SPEC
//! §15.8). The core never touches the network: it asks the shell to run a
//! [`Job`], shows a [`Notice`] on the crate while it waits, and hears the
//! result as [`News`].

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// A release version, `major.minor.patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl FromStr for Version {
    type Err = String;

    /// `1.2.3` or `v1.2.3`; anything else (a pre-release, a word) is refused.
    fn from_str(s: &str) -> Result<Version, String> {
        let mut parts = s.strip_prefix('v').unwrap_or(s).split('.').map(|p| {
            p.parse::<u32>()
                .ok()
                .filter(|_| p.bytes().all(|b| b.is_ascii_digit()))
        });
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) => Ok(Version {
                major,
                minor,
                patch,
            }),
            _ => Err(format!("{s:?} is not a version like 1.2.3")),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What the badge asks the shell to do. Both run outside the badge's
/// process and answer with [`News`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Ask what the latest release is.
    Check,
    /// Download, verify, and install the latest release, then restart.
    Install,
}

/// How a [`Job`] ended (or, from `pawprint update` in a terminal, what it
/// found). Sent to the badge over IPC as the `update` command's `arg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum News {
    /// This is the latest release.
    UpToDate,
    /// A newer release exists.
    Available(Version),
    /// The new release was installed: said to the badge it started.
    Updated,
    /// The check or the install failed; the log says why.
    Failed,
}

impl News {
    /// The IPC form: `current`, `available 1.3.0`, `updated`, `failed`.
    pub fn wire(self) -> String {
        match self {
            News::UpToDate => "current".into(),
            News::Available(v) => format!("available {v}"),
            News::Updated => "updated".into(),
            News::Failed => "failed".into(),
        }
    }

    pub fn parse(s: &str) -> Option<News> {
        match s {
            "current" => Some(News::UpToDate),
            "updated" => Some(News::Updated),
            "failed" => Some(News::Failed),
            _ => s
                .strip_prefix("available ")
                .and_then(|v| v.parse().ok())
                .map(News::Available),
        }
    }
}

/// What the crate says instead of the hostname while an update is in the
/// air. Every notice ends by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Checking,
    UpToDate,
    Available(Version),
    Updating,
    Updated,
    Failed,
}

impl Notice {
    /// The label text. Short, since the crate is: a size-32 badge shows the
    /// first six characters.
    pub fn label(self) -> String {
        match self {
            Notice::Checking => "CHECKING".into(),
            Notice::UpToDate => "UP TO DATE".into(),
            Notice::Available(v) => format!("NEW {v}"),
            Notice::Updating => "UPDATING".into(),
            Notice::Updated => "UPDATED".into(),
            Notice::Failed => "FAILED".into(),
        }
    }

    /// A job is running: another may not start.
    pub fn busy(self) -> bool {
        matches!(self, Notice::Checking | Notice::Updating)
    }

    /// How long it shows. A result is read in a few seconds; a job that
    /// never answers (its process was killed) is given up on after this.
    pub fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Notice::Checking => 60,
            Notice::Updating => 600,
            _ => 8,
        })
    }

    /// Good news gets the mascot's signature action.
    pub fn happy(self) -> bool {
        matches!(
            self,
            Notice::UpToDate | Notice::Available(_) | Notice::Updated
        )
    }
}

impl From<News> for Notice {
    fn from(news: News) -> Notice {
        match news {
            News::UpToDate => Notice::UpToDate,
            News::Available(v) => Notice::Available(v),
            News::Updated => Notice::Updated,
            News::Failed => Notice::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_order_and_print() {
        let v = |s: &str| s.parse::<Version>().unwrap();
        assert_eq!(v("1.2.3").to_string(), "1.2.3");
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert!(v("1.10.0") > v("1.9.9"));
        assert!(v("2.0.0") > v("1.99.99"));
        assert!(v("1.2.1") > v("1.2.0"));
        for bad in [
            "",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1.2.3-rc1",
            "+1.2.3",
            "latest",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
    }

    #[test]
    fn news_round_trips_on_the_wire() {
        let v = "1.3.0".parse().unwrap();
        for news in [
            News::UpToDate,
            News::Available(v),
            News::Updated,
            News::Failed,
        ] {
            assert_eq!(News::parse(&news.wire()), Some(news));
        }
        assert_eq!(News::Available(v).wire(), "available 1.3.0");
        assert_eq!(News::parse("available soon"), None);
        assert_eq!(News::parse("dance"), None);
    }

    #[test]
    fn notices_fit_a_label() {
        let v = "10.20.30".parse().unwrap();
        for n in [
            Notice::Checking,
            Notice::UpToDate,
            Notice::Available(v),
            Notice::Updating,
            Notice::Updated,
            Notice::Failed,
        ] {
            // 13 characters fit a size-64 crate in the micro font.
            assert!(n.label().len() <= 13, "{}", n.label());
        }
    }
}
