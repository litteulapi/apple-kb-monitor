//! `led <name> <on|off>`: goes through the safe LED path of akm-core
//! (`check_led` refuses NumLock on, the target is chosen by vendor/product).

use akm_core::led;
use clap::ValueEnum;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedName {
    #[value(alias = "capslock", alias = "capsl")]
    Caps,
    #[value(alias = "numlock", alias = "numl")]
    Num,
    #[value(alias = "scrolllock", alias = "scrolll")]
    Scroll,
    Compose,
    Kana,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedState {
    #[value(alias = "1", alias = "true")]
    On,
    #[value(alias = "0", alias = "false")]
    Off,
}

impl LedName {
    pub fn code(self) -> u16 {
        match self {
            LedName::Caps => led::LED_CAPSL,
            LedName::Num => led::LED_NUML,
            LedName::Scroll => led::LED_SCROLLL,
            LedName::Compose => led::LED_COMPOSE,
            LedName::Kana => led::LED_KANA,
        }
    }
}

/// Validate before touching anything: the refusal of NumLock on happens here
/// and again in `akm_core::led::set_led`.
pub fn validate(name: LedName, state: LedState) -> Result<(u16, bool), String> {
    let (code, on) = (name.code(), state == LedState::On);
    led::check_led(code, on).map_err(|e| e.to_string())?;
    Ok((code, on))
}

/// Set the LED; the message to print.
pub fn run(name: LedName, state: LedState) -> Result<String, String> {
    let (code, on) = validate(name, state)?;
    led::set_led(code, on).map_err(|e| e.to_string())?;
    Ok(format!("{} LED: {}", led::led_suffix(code).unwrap_or("?"), if on { "on" } else { "off" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numlock_on_is_refused_before_any_io() {
        let e = validate(LedName::Num, LedState::On).unwrap_err();
        assert!(e.contains("NumLock"), "{e}");
        assert_eq!(validate(LedName::Num, LedState::Off), Ok((led::LED_NUML, false)));
    }

    #[test]
    fn every_other_led_validates() {
        for (n, c) in [(LedName::Caps, 1), (LedName::Scroll, 2), (LedName::Compose, 3), (LedName::Kana, 4)] {
            assert_eq!(validate(n, LedState::On), Ok((c, true)));
            assert_eq!(validate(n, LedState::Off), Ok((c, false)));
        }
    }

    #[test]
    fn names_and_states_accept_the_old_spellings() {
        assert_eq!(LedName::from_str("capslock", true), Ok(LedName::Caps));
        assert_eq!(LedName::from_str("CAPS", true), Ok(LedName::Caps));
        assert_eq!(LedName::from_str("numlock", true), Ok(LedName::Num));
        assert_eq!(LedState::from_str("1", true), Ok(LedState::On));
        assert_eq!(LedState::from_str("false", true), Ok(LedState::Off));
        assert!(LedName::from_str("shift", true).is_err());
        assert!(LedState::from_str("maybe", true).is_err());
    }
}
