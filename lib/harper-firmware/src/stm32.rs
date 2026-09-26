// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::{
    AdcController, DeviceInfo, FirmwareDevice, GpioController, I2cController, Platform,
    PwmController, Result, SpiController, UartController,
};
use async_trait::async_trait;

pub struct Stm32Device {
    port: String,
    name: String,
}

impl Stm32Device {
    pub fn new(port: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            port: port.into(),
            name: name.into(),
        }
    }
}

#[async_trait]
impl FirmwareDevice for Stm32Device {
    fn device_info(&self) -> DeviceInfo {
        DeviceInfo {
            name: self.name.clone(),
            platform: Platform::Stm32,
            firmware_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            capabilities: vec![
                "gpio".to_string(),
                "i2c".to_string(),
                "spi".to_string(),
                "uart".to_string(),
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
        log::info!("Connecting to STM32 device on {}", self.port);
        Ok(())
    }

    async fn disconnect(&self) -> Result<()> {
        log::info!("Disconnecting from STM32 device");
        Ok(())
    }

    async fn is_connected(&self) -> Result<bool> {
        Ok(true)
    }
    async fn reset(&self) -> Result<()> {
        log::info!("Resetting STM32 device");
        Ok(())
    }
}
