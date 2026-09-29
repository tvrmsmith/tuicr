//! Picks which in-TUI graphics protocol (if any) to draw images with.

use std::env;
use std::time::Duration;

use ratatui_image::picker::{Capability, Picker, ProtocolType, cap_parser::QueryStdioOptions};

/// Config `image_protocol`: "auto" (default) | "kitty" | "sixel" | "iterm2" | "off".
/// No "halfblocks": it is too coarse for screenshots, so it always means "use
/// the external opener".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageProtocolSetting {
    Auto,
    Force(ProtocolType),
    Off,
}

impl ImageProtocolSetting {
    /// None (unset) -> Auto. Case-insensitive.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        let Some(raw) = value else {
            return Ok(ImageProtocolSetting::Auto);
        };
        match raw.to_ascii_lowercase().as_str() {
            "auto" => Ok(ImageProtocolSetting::Auto),
            "kitty" => Ok(ImageProtocolSetting::Force(ProtocolType::Kitty)),
            "sixel" => Ok(ImageProtocolSetting::Force(ProtocolType::Sixel)),
            "iterm2" => Ok(ImageProtocolSetting::Force(ProtocolType::Iterm2)),
            "off" => Ok(ImageProtocolSetting::Off),
            _ => Err(format!(
                "must be \"auto\", \"kitty\", \"sixel\", \"iterm2\" or \"off\"; got \"{raw}\""
            )),
        }
    }
}

/// Rule: `Off` -> None. `Force(p)` -> `Some(p)`. `Auto`: Kitty + (orca ||
/// zellij) -> Sixel if `caps` has Sixel, else None; Halfblocks (includes the
/// no-reply fallback) -> None; any other pick -> Some(pick).
///
/// None means "don't draw in-TUI" (use the external opener).
pub(crate) fn choose_protocol(
    auto: ProtocolType,
    caps: &[Capability],
    orca: bool,
    zellij: bool,
    setting: &ImageProtocolSetting,
) -> Option<ProtocolType> {
    match setting {
        ImageProtocolSetting::Off => None,
        ImageProtocolSetting::Force(p) => Some(*p),
        ImageProtocolSetting::Auto => match auto {
            ProtocolType::Halfblocks => None,
            ProtocolType::Kitty if orca || zellij => {
                if caps.contains(&Capability::Sixel) {
                    Some(ProtocolType::Sixel)
                } else {
                    None
                }
            }
            other => Some(other),
        },
    }
}

/// Probes the terminal for graphics support and applies `setting`.
///
/// `Off` skips the query entirely, so it never touches the terminal. A query
/// timeout or error yields `None` rather than failing the TUI: this only
/// determines whether images draw in-TUI, never whether the app starts.
///
/// Must run after entering the alternate screen but before reading terminal
/// events; safe to call from an event handler because crossterm only reads
/// stdin inside `event::poll`/`event::read`.
pub fn probe(setting: &ImageProtocolSetting) -> Option<Picker> {
    if *setting == ImageProtocolSetting::Off {
        return None;
    }
    let options = QueryStdioOptions {
        timeout: Duration::from_millis(500),
        ..QueryStdioOptions::default()
    };
    let mut picker = Picker::from_query_stdio_with_options(options).ok()?;
    let orca = env::var("TERM_PROGRAM").is_ok_and(|v| v == "Orca");
    let zellij = env::var("ZELLIJ").is_ok();
    let protocol = choose_protocol(
        picker.protocol_type(),
        picker.capabilities(),
        orca,
        zellij,
        setting,
    )?;
    picker.set_protocol_type(protocol);
    Some(picker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_none_is_auto() {
        assert_eq!(
            ImageProtocolSetting::parse(None),
            Ok(ImageProtocolSetting::Auto)
        );
    }

    #[test]
    fn parse_auto_is_auto() {
        assert_eq!(
            ImageProtocolSetting::parse(Some("auto")),
            Ok(ImageProtocolSetting::Auto)
        );
    }

    #[test]
    fn parse_kitty_is_force_kitty() {
        assert_eq!(
            ImageProtocolSetting::parse(Some("kitty")),
            Ok(ImageProtocolSetting::Force(ProtocolType::Kitty))
        );
    }

    #[test]
    fn parse_is_case_insensitive() {
        assert_eq!(
            ImageProtocolSetting::parse(Some("Sixel")),
            Ok(ImageProtocolSetting::Force(ProtocolType::Sixel))
        );
    }

    #[test]
    fn parse_iterm2_is_force_iterm2() {
        assert_eq!(
            ImageProtocolSetting::parse(Some("iterm2")),
            Ok(ImageProtocolSetting::Force(ProtocolType::Iterm2))
        );
    }

    #[test]
    fn parse_off_is_off() {
        assert_eq!(
            ImageProtocolSetting::parse(Some("off")),
            Ok(ImageProtocolSetting::Off)
        );
    }

    #[test]
    fn parse_halfblocks_is_an_error() {
        assert!(ImageProtocolSetting::parse(Some("halfblocks")).is_err());
    }

    #[test]
    fn parse_bogus_is_an_error() {
        assert!(ImageProtocolSetting::parse(Some("bogus")).is_err());
    }

    #[test]
    fn force_overrides_auto_pick_and_caps() {
        // user override
        assert_eq!(
            choose_protocol(
                ProtocolType::Kitty,
                &[Capability::Sixel],
                true,
                false,
                &ImageProtocolSetting::Force(ProtocolType::Kitty)
            ),
            Some(ProtocolType::Kitty)
        );
    }

    #[test]
    fn force_overrides_halfblocks_auto_pick() {
        // user override
        assert_eq!(
            choose_protocol(
                ProtocolType::Halfblocks,
                &[],
                false,
                false,
                &ImageProtocolSetting::Force(ProtocolType::Sixel)
            ),
            Some(ProtocolType::Sixel)
        );
    }

    #[test]
    fn off_ignores_auto_pick_and_caps() {
        assert_eq!(
            choose_protocol(
                ProtocolType::Kitty,
                &[Capability::Sixel],
                false,
                false,
                &ImageProtocolSetting::Off
            ),
            None
        );
    }

    #[test]
    fn auto_kitty_under_orca_steps_down_to_sixel_when_supported() {
        // Orca
        assert_eq!(
            choose_protocol(
                ProtocolType::Kitty,
                &[Capability::Sixel],
                true,
                false,
                &ImageProtocolSetting::Auto
            ),
            Some(ProtocolType::Sixel)
        );
    }

    #[test]
    fn auto_kitty_under_zellij_steps_down_to_sixel_when_supported() {
        // Orca + zellij
        assert_eq!(
            choose_protocol(
                ProtocolType::Kitty,
                &[Capability::Sixel],
                false,
                true,
                &ImageProtocolSetting::Auto
            ),
            Some(ProtocolType::Sixel)
        );
    }

    #[test]
    fn auto_kitty_under_orca_without_sixel_cap_is_none() {
        // Orca, no sixel
        assert_eq!(
            choose_protocol(
                ProtocolType::Kitty,
                &[],
                true,
                false,
                &ImageProtocolSetting::Auto
            ),
            None
        );
    }

    #[test]
    fn auto_kitty_under_zellij_without_sixel_cap_is_none() {
        // Ghostty + zellij
        assert_eq!(
            choose_protocol(
                ProtocolType::Kitty,
                &[],
                false,
                true,
                &ImageProtocolSetting::Auto
            ),
            None
        );
    }

    #[test]
    fn auto_kitty_outside_orca_or_zellij_stays_kitty() {
        // Ghostty
        assert_eq!(
            choose_protocol(
                ProtocolType::Kitty,
                &[],
                false,
                false,
                &ImageProtocolSetting::Auto
            ),
            Some(ProtocolType::Kitty)
        );
    }

    #[test]
    fn auto_halfblocks_is_none() {
        // pane off screen / no reply
        assert_eq!(
            choose_protocol(
                ProtocolType::Halfblocks,
                &[],
                false,
                false,
                &ImageProtocolSetting::Auto
            ),
            None
        );
    }

    #[test]
    fn auto_sixel_pick_stays_sixel() {
        assert_eq!(
            choose_protocol(
                ProtocolType::Sixel,
                &[Capability::Sixel],
                false,
                false,
                &ImageProtocolSetting::Auto
            ),
            Some(ProtocolType::Sixel)
        );
    }

    #[test]
    fn auto_iterm2_pick_stays_iterm2() {
        assert_eq!(
            choose_protocol(
                ProtocolType::Iterm2,
                &[],
                false,
                false,
                &ImageProtocolSetting::Auto
            ),
            Some(ProtocolType::Iterm2)
        );
    }
}
