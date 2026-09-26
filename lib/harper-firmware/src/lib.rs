// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur during firmware operations
#[derive(Debug, Error)]
pub enum FirmwareError {
    /// Device is not connected or available
    #[error("Device not connected: {0}")]
    DeviceNotConnected(String),
    /// Communication failure with the device
    #[error("Communication error: {0}")]
    CommunicationError(String),
    /// Pin configuration or access error
    #[error("Pin error: {0}")]
    PinError(String),
    /// Platform is not supported for firmware operations
    #[error("Unsupported platform: {0}")]
    UnsupportedPlatform(String),
    /// Input/output operation failed
    #[error("IO error: {0}")]
    IoError(String),
}

pub type Result<T> = std::result::Result<T, FirmwareError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinMode {
    Input,
    Output,
    InputPullUp,
    InputPullDown,
    Analog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinState {
    Low,
    High,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinConfig {
    pub pin_number: u32,
    pub mode: PinMode,
    pub initial_state: Option<PinState>,
}

#[async_trait]
pub trait GpioController: Send + Sync {
    async fn configure_pin(&self, config: PinConfig) -> Result<()>;
    async fn write_pin(&self, pin: u32, state: PinState) -> Result<()>;
    async fn read_pin(&self, pin: u32) -> Result<PinState>;
    async fn toggle_pin(&self, pin: u32) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2cConfig {
    pub address: u8,
    pub bus_speed: u32,
    pub sda_pin: Option<u32>,
    pub scl_pin: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2cMessage {
    pub address: u8,
    pub data: Vec<u8>,
}

#[async_trait]
pub trait I2cController: Send + Sync {
    async fn write(&self, message: I2cMessage) -> Result<()>;
    async fn read(&self, address: u8, length: usize) -> Result<Vec<u8>>;
    async fn write_read(&self, write_data: Vec<u8>, read_len: usize) -> Result<Vec<u8>>;
    async fn scan_devices(&self) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiConfig {
    pub mode: SpiMode,
    pub frequency: u32,
    pub mosi_pin: Option<u32>,
    pub miso_pin: Option<u32>,
    pub clock_pin: Option<u32>,
    pub chip_select: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpiMode {
    Mode0,
    Mode1,
    Mode2,
    Mode3,
}

#[async_trait]
pub trait SpiController: Send + Sync {
    async fn transfer(&self, data: &[u8]) -> Result<Vec<u8>>;
    async fn write(&self, data: &[u8]) -> Result<()>;
    async fn read(&self, length: usize) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UartConfig {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub rx_pin: Option<u32>,
    pub tx_pin: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Parity {
    None,
    Even,
    Odd,
}

#[async_trait]
pub trait UartController: Send + Sync {
    async fn write(&self, data: &[u8]) -> Result<usize>;
    async fn read(&self, length: usize) -> Result<Vec<u8>>;
    async fn write_string(&self, data: &str) -> Result<usize>;
    async fn flush(&self) -> Result<()>;
    async fn available(&self) -> Result<usize>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PwmConfig {
    pub pin: u32,
    pub frequency: u32,
    pub duty_cycle: f32,
}

#[async_trait]
pub trait PwmController: Send + Sync {
    async fn configure(&self, config: PwmConfig) -> Result<()>;
    async fn set_duty_cycle(&self, pin: u32, duty: f32) -> Result<()>;
    async fn set_frequency(&self, pin: u32, frequency: u32) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdcConfig {
    pub pin: u32,
    pub resolution: u8,
    pub vref: f32,
}

#[async_trait]
pub trait AdcController: Send + Sync {
    async fn read_voltage(&self, pin: u32) -> Result<f32>;
    async fn read_raw(&self, pin: u32) -> Result<u32>;
    async fn configure(&self, config: AdcConfig) -> Result<()>;
}

#[async_trait]
pub trait DelayController: Send + Sync {
    async fn delay_ms(&self, ms: u32);
    async fn delay_us(&self, us: u32);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub platform: Platform,
    pub firmware_version: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Esp32,
    Esp8266,
    Stm32,
    RaspberryPiPico,
    Arduino,
    Custom,
}

impl Platform {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "esp32" => Platform::Esp32,
            "esp8266" => Platform::Esp8266,
            "stm32" => Platform::Stm32,
            "raspberrypipico" | "pico" => Platform::RaspberryPiPico,
            "arduino" => Platform::Arduino,
            _ => Platform::Custom,
        }
    }
}

#[async_trait]
pub trait FirmwareDevice: Send + Sync {
    fn device_info(&self) -> DeviceInfo;

    fn gpio(&self) -> Option<&dyn GpioController>;
    fn i2c(&self) -> Option<&dyn I2cController>;
    fn spi(&self) -> Option<&dyn SpiController>;
    fn uart(&self) -> Option<&dyn UartController>;
    fn pwm(&self) -> Option<&dyn PwmController>;
    fn adc(&self) -> Option<&dyn AdcController>;
    fn delay(&self) -> Option<&dyn DelayController>;

    async fn connect(&self) -> Result<()>;
    async fn disconnect(&self) -> Result<()>;
    async fn is_connected(&self) -> Result<bool>;
    async fn reset(&self) -> Result<()>;
}

pub struct FirmwareRegistry {
    devices: HashMap<String, Box<dyn FirmwareDevice>>,
}

impl FirmwareRegistry {
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: String, device: Box<dyn FirmwareDevice>) {
        self.devices.insert(name, device);
    }

    pub fn get(&self, name: &str) -> Option<&dyn FirmwareDevice> {
        self.devices
            .get(name)
            .map(|d| d.as_ref() as &dyn FirmwareDevice)
    }

    pub fn list_devices(&self) -> Vec<String> {
        self.devices.keys().cloned().collect()
    }
}

impl Default for FirmwareRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub mod esp32;
pub mod raspberry_pi;
pub mod stm32;

pub use esp32::Esp32Device;
pub use raspberry_pi::RaspberryPiDevice;
pub use stm32::Stm32Device;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pin_mode_variants() {
        let modes = [
            PinMode::Input,
            PinMode::Output,
            PinMode::InputPullUp,
            PinMode::InputPullDown,
            PinMode::Analog,
        ];
        for mode in modes {
            assert!(!format!("{:?}", mode).is_empty());
        }
    }

    #[test]
    fn test_pin_state_variants() {
        let states = [PinState::Low, PinState::High, PinState::Unknown];
        for state in states {
            assert!(!format!("{:?}", state).is_empty());
        }
    }

    #[test]
    fn test_pin_config_creation() {
        let config = PinConfig {
            pin_number: 13,
            mode: PinMode::Output,
            initial_state: Some(PinState::High),
        };
        assert_eq!(config.pin_number, 13);
        assert!(matches!(config.mode, PinMode::Output));
        assert!(matches!(config.initial_state, Some(PinState::High)));
    }

    #[test]
    fn test_platform_parse() {
        assert!(matches!(Platform::parse("esp32"), Platform::Esp32));
        assert!(matches!(Platform::parse("ESP32"), Platform::Esp32));
        assert!(matches!(Platform::parse("stm32"), Platform::Stm32));
        assert!(matches!(Platform::parse("pico"), Platform::RaspberryPiPico));
        assert!(matches!(Platform::parse("arduino"), Platform::Arduino));
        assert!(matches!(Platform::parse("unknown"), Platform::Custom));
    }

    #[test]
    fn test_spi_mode_variants() {
        let modes = [
            SpiMode::Mode0,
            SpiMode::Mode1,
            SpiMode::Mode2,
            SpiMode::Mode3,
        ];
        for mode in modes {
            assert!(!format!("{:?}", mode).is_empty());
        }
    }

    #[test]
    fn test_parity_variants() {
        let parity = [Parity::None, Parity::Even, Parity::Odd];
        for p in parity {
            assert!(!format!("{:?}", p).is_empty());
        }
    }

    #[test]
    fn test_i2c_config_creation() {
        let config = I2cConfig {
            address: 0x68,
            bus_speed: 400_000,
            sda_pin: Some(21),
            scl_pin: Some(22),
        };
        assert_eq!(config.address, 0x68);
        assert_eq!(config.bus_speed, 400_000);
        assert_eq!(config.sda_pin, Some(21));
    }

    #[test]
    fn test_spi_config_creation() {
        let config = SpiConfig {
            mode: SpiMode::Mode0,
            frequency: 1_000_000,
            mosi_pin: Some(23),
            miso_pin: Some(19),
            clock_pin: Some(18),
            chip_select: Some(5),
        };
        assert!(matches!(config.mode, SpiMode::Mode0));
        assert_eq!(config.frequency, 1_000_000);
    }

    #[test]
    fn test_uart_config_creation() {
        let config = UartConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: Parity::None,
            rx_pin: Some(16),
            tx_pin: Some(17),
        };
        assert_eq!(config.baud_rate, 115_200);
        assert_eq!(config.data_bits, 8);
        assert!(matches!(config.parity, Parity::None));
    }

    #[test]
    fn test_pwm_config_creation() {
        let config = PwmConfig {
            pin: 12,
            frequency: 1000,
            duty_cycle: 0.5,
        };
        assert_eq!(config.pin, 12);
        assert_eq!(config.frequency, 1000);
        assert_eq!(config.duty_cycle, 0.5);
    }

    #[test]
    fn test_adc_config_creation() {
        let config = AdcConfig {
            pin: 34,
            resolution: 12,
            vref: 3.3,
        };
        assert_eq!(config.pin, 34);
        assert_eq!(config.resolution, 12);
        assert_eq!(config.vref, 3.3);
    }

    #[test]
    fn test_device_info_creation() {
        let info = DeviceInfo {
            name: "ESP32 DevKit".to_string(),
            platform: Platform::Esp32,
            firmware_version: Some("1.0.0".to_string()),
            capabilities: vec!["gpio".to_string(), "i2c".to_string()],
        };
        assert_eq!(info.name, "ESP32 DevKit");
        assert!(matches!(info.platform, Platform::Esp32));
        assert_eq!(info.capabilities.len(), 2);
    }

    #[test]
    fn test_firmware_registry() {
        let registry = FirmwareRegistry::new();
        assert!(registry.list_devices().is_empty());
        assert!(registry.get("test").is_none());
    }

    #[test]
    fn test_firmware_error_display() {
        let err = FirmwareError::DeviceNotConnected("uart0".to_string());
        assert!(format!("{}", err).contains("Device not connected"));

        let err = FirmwareError::CommunicationError("timeout".to_string());
        assert!(format!("{}", err).contains("Communication error"));

        let err = FirmwareError::PinError("invalid pin".to_string());
        assert!(format!("{}", err).contains("Pin error"));

        let err = FirmwareError::UnsupportedPlatform("custom".to_string());
        assert!(format!("{}", err).contains("Unsupported platform"));

        let err = FirmwareError::IoError("read failed".to_string());
        assert!(format!("{}", err).contains("IO error"));
    }
}
