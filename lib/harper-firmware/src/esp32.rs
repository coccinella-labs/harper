// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::{
    AdcController, DeviceInfo, FirmwareDevice, GpioController, I2cController, Platform,
    PwmController, Result, SpiController, UartController,
};
use async_trait::async_trait;

pub struct Esp32Device {
    port: String,
    name: String,
}

impl Esp32Device {
    pub fn new(port: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            port: port.into(),
            name: name.into(),
        }
    }
}

#[async_trait]
impl FirmwareDevice for Esp32Device {
    fn device_info(&self) -> DeviceInfo {
        DeviceInfo {
            name: self.name.clone(),
            platform: Platform::Esp32,
            firmware_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            capabilities: vec![
                "gpio".to_string(),
                "i2c".to_string(),
                "spi".to_string(),
                "uart".to_string(),
                "pwm".to_string(),
                "adc".to_string(),
            ],
        }
    }

    fn gpio(&self) -> Option<&dyn GpioController> {
        None
    }

    fn i2c(&self) -> Option<&dyn I2cController> {
        None
    }

    fn spi(&self) -> Option<&dyn SpiController> {
        None
    }

    fn uart(&self) -> Option<&dyn UartController> {
        None
    }

    fn pwm(&self) -> Option<&dyn PwmController> {
        None
    }

    fn adc(&self) -> Option<&dyn AdcController> {
        None
    }

    fn delay(&self) -> Option<&dyn crate::DelayController> {
        None
    }

    async fn connect(&self) -> Result<()> {
        log::info!("Connecting to ESP32 device on {}", self.port);
        Ok(())
    }

    async fn disconnect(&self) -> Result<()> {
        log::info!("Disconnecting from ESP32 device");
        Ok(())
    }

    async fn is_connected(&self) -> Result<bool> {
        Ok(true)
    }

    async fn reset(&self) -> Result<()> {
        log::info!("Resetting ESP32 device");
        Ok(())
    }
}
