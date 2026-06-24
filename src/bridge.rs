use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::operations::ProbeSession;
use crate::probe::{SerialWatchOutput, WatchSerialOptions, WchLink};
use crate::{Error, Result, RiscvChip, commands::Speed};

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct BridgeCommand {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub watch_port: Option<String>,
    #[serde(default)]
    pub probe_serial: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum BridgeMessage<'a> {
    #[serde(rename = "response")]
    Response {
        id: &'a str,
        status: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        stream: Option<&'a str>,
    },
    #[serde(rename = "error")]
    Error {
        id: Option<&'a str>,
        status: &'a str,
        error: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostic: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<&'a str>,
    },
}

impl BridgeCommand {
    pub fn sdi_watch_options(&self) -> WatchSerialOptions {
        WatchSerialOptions {
            output: SerialWatchOutput::BridgeJsonl,
            port_name: self.watch_port.clone(),
            probe_serial: self.probe_serial.clone(),
            ..Default::default()
        }
    }
}

pub fn parse_command(line: &str) -> std::result::Result<BridgeCommand, serde_json::Error> {
    serde_json::from_str(line)
}

pub fn response_json(id: &str, stream: Option<&str>) -> Result<String> {
    Ok(serde_json::to_string(&BridgeMessage::Response {
        id,
        status: "ok",
        stream,
    })?)
}

pub fn error_json(id: Option<&str>, err: &Error) -> Result<String> {
    let diagnostic = err
        .protocol_diagnostic()
        .map(|diagnostic| match diagnostic {
            crate::error::ProtocolErrorDiagnostic::TargetDebugAttachOrControlFailed => {
                "target_debug_attach_or_control_failed"
            }
            crate::error::ProtocolErrorDiagnostic::Unknown => "unknown_protocol_error",
        });
    Ok(serde_json::to_string(&BridgeMessage::Error {
        id,
        status: "error",
        error: &err.to_string(),
        diagnostic,
        hint: err.recovery_hint(),
    })?)
}

pub fn run_jsonl_daemon<R: BufRead, W: Write>(reader: R, mut writer: W) -> Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let cmd = match parse_command(&line) {
            Ok(cmd) => cmd,
            Err(err) => {
                let message = BridgeMessage::Error {
                    id: None,
                    status: "error",
                    error: &format!("invalid JSON command: {err}"),
                    diagnostic: None,
                    hint: None,
                };
                writeln!(writer, "{}", serde_json::to_string(&message)?)?;
                continue;
            }
        };

        match cmd.method.as_str() {
            "sdi.start" => {
                writeln!(writer, "{}", response_json(&cmd.id, Some("sdi"))?)?;
                writer.flush()?;
                enable_sdi_print_for_bridge_watch()?;
                crate::probe::watch_serial_with_options(cmd.sdi_watch_options())?;
                return Ok(());
            }
            _ => {
                let err = Error::Custom(format!("unknown bridge method: {}", cmd.method));
                writeln!(writer, "{}", error_json(Some(&cmd.id), &err)?)?;
            }
        }
    }
    Ok(())
}

fn enable_sdi_print_for_bridge_watch() -> Result<()> {
    let probe = WchLink::open_nth(0)?;
    let mut sess = ProbeSession::attach(probe, Some(RiscvChip::CH32V00X), Speed::High)?;
    sess.soft_reset()?;
    sess.set_sdi_print_enabled(true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sdi_start_command() {
        let cmd = parse_command(r#"{"id":"1","method":"sdi.start","probe_serial":"8B0B8F060FCB"}"#)
            .unwrap();

        assert_eq!(cmd.id, "1");
        assert_eq!(cmd.method, "sdi.start");
        assert_eq!(cmd.probe_serial.as_deref(), Some("8B0B8F060FCB"));
        assert_eq!(
            cmd.sdi_watch_options().output,
            SerialWatchOutput::BridgeJsonl
        );
    }

    #[test]
    fn serializes_ok_response() {
        let line = response_json("1", Some("sdi")).unwrap();

        assert_eq!(
            line,
            r#"{"type":"response","id":"1","status":"ok","stream":"sdi"}"#
        );
    }

    #[test]
    fn serializes_protocol_error_with_hint() {
        let err = Error::Protocol(0x55, vec![0x81, 0x55, 0x01, 0x01]);
        let line = error_json(Some("9"), &err).unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();

        assert_eq!(value["type"], "error");
        assert_eq!(value["id"], "9");
        assert_eq!(value["status"], "error");
        assert_eq!(value["diagnostic"], "target_debug_attach_or_control_failed");
        assert!(
            value["hint"]
                .as_str()
                .unwrap()
                .contains("true target power-on reset")
        );
    }

    #[test]
    fn daemon_reports_unknown_method_without_starting_watch() {
        let input = br#"{"id":"2","method":"unknown"}
"#;
        let mut output = Vec::new();

        run_jsonl_daemon(&input[..], &mut output).unwrap();

        let text = String::from_utf8(output).unwrap();
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["type"], "error");
        assert_eq!(value["id"], "2");
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains("unknown bridge method")
        );
    }
}
