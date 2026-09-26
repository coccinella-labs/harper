// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::core::constants::tools;
use crate::core::error::HarperResult;

pub fn handle_firmware_command(response: &str) -> HarperResult<String> {
    let command = response
        .strip_prefix(tools::FIRMWARE)
        .unwrap_or(response)
        .trim()
        .trim_start_matches(']')
        .trim();

    let parts: Vec<&str> = command.splitn(2, ' ').collect();
    let action = parts.first().unwrap_or(&"");
    let args = parts.get(1).unwrap_or(&"");

    match *action {
        "list" => list_devices(),
        "info" => device_info(args),
        "connect" => connect_device(args),
        "disconnect" => disconnect_device(args),
        "gpio" => gpio_operation(args),
        "i2c" => i2c_operation(args),
        "spi" => spi_operation(args),
        "uart" => uart_operation(args),
        _ => Ok(format!(
            "Unknown firmware command: {}\n\nAvailable commands:\n\
            - [FIRMWARE list] - List registered devices\n\
            - [FIRMWARE info <device>] - Show device info\n\
            - [FIRMWARE connect <device>] - Connect to device\n\
            - [FIRMWARE disconnect <device>] - Disconnect from device\n\
            - [FIRMWARE gpio <pin> <high|low>] - Set GPIO pin state\n\
            - [FIRMWARE i2c <device> <read|write> <data>] - I2C operations\n\
            - [FIRMWARE spi <device> <transfer> <data>] - SPI transfer\n\
            - [FIRMWARE uart <device> <send> <data>] - Send via UART",
            action
        )),
    }
}

fn list_devices() -> HarperResult<String> {
    Ok("No firmware devices configured.\n\
        To add devices programmatically:\n\
        ```rust\n\
        use harper_firmware::{FirmwareRegistry, Esp32Device};\n\
        \n\
        let mut registry = FirmwareRegistry::new();\n\
        registry.register(\"my-esp32\".to_string(), Box::new(Esp32Device::new(\"/dev/ttyUSB0\", \"my-esp32\")));\n\
        ```\n\
        See lib/harper-firmware/examples/basic.rs for more."
        .to_string())
}

fn device_info(_args: &str) -> HarperResult<String> {
    Ok("Device info not available - no devices connected.\n\
        Use [FIRMWARE list] to see available devices."
        .to_string())
}

fn connect_device(_args: &str) -> HarperResult<String> {
    Ok("Connect not implemented - harper-firmware integration is ready but needs runtime device setup.\n\
        Devices can be registered programmatically through the FirmwareRegistry API.".to_string())
}

fn disconnect_device(_args: &str) -> HarperResult<String> {
    Ok("Disconnect not implemented - harper-firmware integration is ready but needs runtime device setup.".to_string())
}

fn gpio_operation(_args: &str) -> HarperResult<String> {
    Ok(
        "GPIO operations ready - harper-firmware traits available for:\n\
        - Pin configuration (Input, Output, PullUp, PullDown, Analog)\n\
        - Pin read/write/toggle\n\
        Implement GpioController trait for your platform."
            .to_string(),
    )
}

fn i2c_operation(_args: &str) -> HarperResult<String> {
    Ok(
        "I2C operations ready - harper-firmware traits available for:\n\
        - I2C read/write/write_read\n\
        - Device scanning\n\
        Implement I2cController trait for your platform."
            .to_string(),
    )
}

fn spi_operation(_args: &str) -> HarperResult<String> {
    Ok(
        "SPI operations ready - harper-firmware traits available for:\n\
        - SPI transfer/write/read\n\
        - Configurable modes (0-3)\n\
        Implement SpiController trait for your platform."
            .to_string(),
    )
}

fn uart_operation(_args: &str) -> HarperResult<String> {
    Ok(
        "UART operations ready - harper-firmware traits available for:\n\
        - UART read/write/flush\n\
        - Configurable baud rate, parity, stop bits\n\
        Implement UartController trait for your platform."
            .to_string(),
    )
}
