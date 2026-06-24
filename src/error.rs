use thiserror::Error;

use crate::RiscvChip;

/// Alias for a `Result` with the error type `wlink::Error`.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Custom(String),
    #[error("USB error: {0}")]
    Usb(nusb::Error),
    #[error("WCH-Link not found, please check your connection")]
    ProbeNotFound,
    #[error("WCH-Link is connected, but is not in RV mode")]
    ProbeModeNotSupported,
    #[error("WCH-Link doesn't support current chip: {0:?}")]
    UnsupportedChip(RiscvChip),
    #[error("Unknown WCH-Link variant: {0}")]
    UnknownLinkVariant(u8),
    #[error("Unknown RISC-V Chip: 0x{0:02x}")]
    UnknownChip(u8),
    #[error(
        "Probe is not attached to an MCU, or debug is not enabled. (hint: use wchisp to enable debug)"
    )]
    NotAttached,
    #[error("Chip mismatch: expected {0:?}, got {1:?}")]
    ChipMismatch(RiscvChip, RiscvChip),
    #[error("WCH-Link underlying protocol error: {0:#04x} {1:#04x?}")]
    Protocol(u8, Vec<u8>),
    #[error("Invalid payload length")]
    InvalidPayloadLength,
    #[error("Invalid payload")]
    InvalidPayload,
    #[error("DM Abstract comand error: {0:?}")]
    AbstractCommandError(AbstractcsCmdErr),
    #[error("DM is busy")]
    Busy,
    #[error("DMI Status Failed")]
    DmiFailed,
    #[error("Operation timeout")]
    Timeout,
    #[error("Serial port error: {0}")]
    Serial(#[from] serialport::Error),
    #[error("Io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Driver error")]
    Driver,
}

#[derive(Debug, Clone, Copy)]
pub enum AbstractcsCmdErr {
    /// Write to the command, abstractcs and abstractauto registers, or read/write to the data
    /// and progbuf registers when the abstract command is executed.
    Busy = 1,
    /// The current abstract command is not supported
    NotSupported = 2,
    /// error occurs when the abstract command is executed.
    Exception = 3,
    /// the hart wasn’t in the required state (running/halted), or unavailable
    HaltOrResume = 4,
    /// bus error (e.g. alignment, access size, or timeout)
    Bus = 5,
    /// Parity bit error during communication (WCH's extension)
    Parity = 6,
    /// The command failed for another reason.
    Other = 7,
}

impl Error {
    pub fn protocol_diagnostic(&self) -> Option<ProtocolErrorDiagnostic> {
        match self {
            Error::Protocol(0x55, _) => {
                Some(ProtocolErrorDiagnostic::TargetDebugAttachOrControlFailed)
            }
            Error::Protocol(_, _) => Some(ProtocolErrorDiagnostic::Unknown),
            _ => None,
        }
    }

    pub fn recovery_hint(&self) -> Option<&'static str> {
        self.protocol_diagnostic()
            .and_then(|diagnostic| diagnostic.recovery_hint())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolErrorDiagnostic {
    TargetDebugAttachOrControlFailed,
    Unknown,
}

impl ProtocolErrorDiagnostic {
    pub fn recovery_hint(&self) -> Option<&'static str> {
        match self {
            ProtocolErrorDiagnostic::TargetDebugAttachOrControlFailed => Some(
                "WCH-Link reported protocol error 0x55 while controlling the target debug interface. Check target power, true target power-on reset, nRST/SWIO wiring, chip family selection, read protection/debug enable state, and whether firmware has remapped or disabled debug pins. For externally powered targets, resetting only USB or the probe may not reset the MCU power rail.",
            ),
            ProtocolErrorDiagnostic::Unknown => None,
        }
    }
}

impl AbstractcsCmdErr {
    pub(crate) fn try_from_cmderr(value: u8) -> Result<()> {
        match value {
            0 => Ok(()),
            1 => Err(Error::AbstractCommandError(AbstractcsCmdErr::Busy)),
            2 => Err(Error::AbstractCommandError(AbstractcsCmdErr::NotSupported)),
            3 => Err(Error::AbstractCommandError(AbstractcsCmdErr::Exception)),
            4 => Err(Error::AbstractCommandError(AbstractcsCmdErr::HaltOrResume)),
            5 => Err(Error::AbstractCommandError(AbstractcsCmdErr::Bus)),
            6 => Err(Error::AbstractCommandError(AbstractcsCmdErr::Parity)),
            7 => Err(Error::AbstractCommandError(AbstractcsCmdErr::Other)),

            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_0x55_reports_target_debug_attach_or_control_failure() {
        let err = Error::Protocol(0x55, vec![0x81, 0x55, 0x01, 0x01]);

        assert_eq!(
            err.protocol_diagnostic(),
            Some(ProtocolErrorDiagnostic::TargetDebugAttachOrControlFailed)
        );
        let hint = err.recovery_hint().unwrap();
        assert!(hint.contains("target debug interface"));
        assert!(hint.contains("true target power-on reset"));
        assert!(hint.contains("externally powered targets"));
    }

    #[test]
    fn other_protocol_errors_are_unknown_without_recovery_hint() {
        let err = Error::Protocol(0x42, vec![0x81, 0x42, 0x00]);

        assert_eq!(
            err.protocol_diagnostic(),
            Some(ProtocolErrorDiagnostic::Unknown)
        );
        assert_eq!(err.recovery_hint(), None);
    }

    #[test]
    fn non_protocol_errors_have_no_protocol_diagnostic() {
        let err = Error::InvalidPayload;

        assert_eq!(err.protocol_diagnostic(), None);
        assert_eq!(err.recovery_hint(), None);
    }
}
