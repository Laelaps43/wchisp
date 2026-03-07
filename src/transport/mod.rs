//! Abstract Device transport interface.
use std::{thread::sleep, time::Duration};

use anyhow::Result;

use crate::protocol::{Command, Response};

pub use self::serial::{Baudrate, SerialTransport};
pub use self::usb::UsbTransport;

mod serial;
mod usb;

const DEFAULT_TRANSPORT_TIMEOUT_MS: u64 = 1000;

/// Abstraction of the transport layer.
/// Might be a USB, a serial port, or Network.
pub trait Transport {
    fn send_raw(&mut self, raw: &[u8]) -> Result<()>;
    fn recv_raw(&mut self, timeout: Duration) -> Result<Vec<u8>>;

    fn transfer(&mut self, cmd: Command) -> Result<Response> {
        self.transfer_with_wait(cmd, Duration::from_millis(DEFAULT_TRANSPORT_TIMEOUT_MS))
    }

    fn transfer_with_wait(&mut self, cmd: Command, wait: Duration) -> Result<Response> {
        let skip_echo_check = matches!(
            cmd,
            Command::KmboxInit
                | Command::KmboxWrite { .. }
                | Command::KmboxVerify { .. }
                | Command::KmboxEnd
                | Command::KmboxRaw { .. }
        );
        let req = &cmd.into_raw()?;
        log::debug!("=> {}   {}", hex::encode(&req[..3]), hex::encode(&req[3..]));
        self.send_raw(&req)?;
        sleep(Duration::from_micros(1)); // required for some Linux platform

        let resp = self.recv_raw(wait)?;

        // kmbox commands return a bare status payload and do not echo the request byte.
        // Match on the command type rather than the numeric range so fuzzing can probe
        // unknown bytes such as 0x90-0xff without tripping the standard ISP check.
        if !skip_echo_check {
            anyhow::ensure!(req[0] == resp[0], "response command type mismatch");
        }

        log::debug!("<= {} {}", hex::encode(&resp[..4.min(resp.len())]),
                    if resp.len() > 4 { hex::encode(&resp[4..]) } else { String::new() });
        Response::from_raw(&resp)
    }
}
