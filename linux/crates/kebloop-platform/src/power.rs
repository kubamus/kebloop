//! Power-off through systemd-logind (`org.freedesktop.login1.Manager`).
//!
//! The real implementation is [`Logind`]; everything that needs to be tested
//! goes through the [`PowerBackend`] trait so tests never power the machine off.

use zbus::blocking::{Connection, Proxy};

const DESTINATION: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const INTERFACE: &str = "org.freedesktop.login1.Manager";

/// Result of `CanPowerOff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerCapability {
    /// Allowed.
    Yes,
    /// Allowed after an authentication challenge (polkit prompt).
    Challenge,
    /// Not allowed for this user/session.
    No,
    /// The operation is not supported on this system.
    NotAvailable,
}

impl PowerCapability {
    /// Parses the logind answer (`yes`, `challenge`, `no`, `na`).
    pub fn parse(value: &str) -> Self {
        match value {
            "yes" => Self::Yes,
            "challenge" => Self::Challenge,
            "no" => Self::No,
            _ => Self::NotAvailable,
        }
    }

    /// Whether `power_off` may be attempted.
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Yes | Self::Challenge)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PowerError {
    #[error("D-Bus error: {0}")]
    Dbus(String),
    #[error("power off is not permitted on this system")]
    NotPermitted,
}

pub trait PowerBackend {
    fn capability(&self) -> Result<PowerCapability, PowerError>;
    /// Requests power-off with `interactive = true` (polkit may prompt).
    fn power_off(&self) -> Result<(), PowerError>;
}

/// systemd-logind over the system bus.
#[derive(Debug, Default, Clone, Copy)]
pub struct Logind;

impl Logind {
    fn proxy() -> Result<Proxy<'static>, PowerError> {
        let conn = Connection::system().map_err(|e| PowerError::Dbus(e.to_string()))?;
        Proxy::new(&conn, DESTINATION, PATH, INTERFACE).map_err(|e| PowerError::Dbus(e.to_string()))
    }
}

impl PowerBackend for Logind {
    fn capability(&self) -> Result<PowerCapability, PowerError> {
        let reply: String = Self::proxy()?
            .call("CanPowerOff", &())
            .map_err(|e| PowerError::Dbus(e.to_string()))?;
        Ok(PowerCapability::parse(&reply))
    }

    fn power_off(&self) -> Result<(), PowerError> {
        Self::proxy()?
            .call::<_, _, ()>("PowerOff", &(true,))
            .map_err(|e| PowerError::Dbus(e.to_string()))
    }
}

/// Whether the shutdown command should be offered (any D-Bus failure means no).
pub fn can_power_off_with(backend: &dyn PowerBackend) -> bool {
    backend.capability().is_ok_and(PowerCapability::is_allowed)
}

/// Checks the capability, then powers off. Never called from tests.
pub fn power_off_with(backend: &dyn PowerBackend) -> Result<(), PowerError> {
    if !backend.capability()?.is_allowed() {
        return Err(PowerError::NotPermitted);
    }
    backend.power_off()
}

/// `CanPowerOff` through logind.
pub fn can_power_off() -> bool {
    can_power_off_with(&Logind)
}

/// `PowerOff(interactive = true)` through logind.
pub fn power_off() -> Result<(), PowerError> {
    power_off_with(&Logind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Mock {
        cap: Result<PowerCapability, ()>,
        calls: Cell<u32>,
    }

    impl PowerBackend for Mock {
        fn capability(&self) -> Result<PowerCapability, PowerError> {
            self.cap.map_err(|()| PowerError::Dbus("boom".into()))
        }
        fn power_off(&self) -> Result<(), PowerError> {
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
    }

    fn mock(cap: Result<PowerCapability, ()>) -> Mock {
        Mock {
            cap,
            calls: Cell::new(0),
        }
    }

    #[test]
    fn parses_logind_answers() {
        assert_eq!(PowerCapability::parse("yes"), PowerCapability::Yes);
        assert_eq!(
            PowerCapability::parse("challenge"),
            PowerCapability::Challenge
        );
        assert_eq!(PowerCapability::parse("no"), PowerCapability::No);
        assert_eq!(PowerCapability::parse("na"), PowerCapability::NotAvailable);
        assert_eq!(
            PowerCapability::parse("weird"),
            PowerCapability::NotAvailable
        );
    }

    #[test]
    fn power_off_requires_capability() {
        for (cap, expect_call) in [
            (Ok(PowerCapability::Yes), true),
            (Ok(PowerCapability::Challenge), true),
            (Ok(PowerCapability::No), false),
            (Ok(PowerCapability::NotAvailable), false),
        ] {
            let m = mock(cap);
            let result = power_off_with(&m);
            assert_eq!(result.is_ok(), expect_call);
            assert_eq!(m.calls.get(), u32::from(expect_call));
            assert_eq!(can_power_off_with(&m), expect_call);
        }
        let failing = mock(Err(()));
        assert!(!can_power_off_with(&failing));
        assert!(matches!(power_off_with(&failing), Err(PowerError::Dbus(_))));
        assert_eq!(failing.calls.get(), 0);
    }

    /// Read-only: asks logind whether power-off is possible.
    #[test]
    #[ignore = "requires the system bus"]
    fn live_capability_query() {
        let cap = Logind.capability();
        eprintln!("logind CanPowerOff: {cap:?}");
        assert!(cap.is_ok());
    }
}
